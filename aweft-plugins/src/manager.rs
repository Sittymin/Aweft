use anyhow::{Context, Result, bail};
use aweft_model::ids::ItemId;
use aweft_project::{EventTimeChanged, MutationKind, MutationReceipt, Origin, ProjectStore};
use slint_interpreter::ComponentDefinition;
use std::collections::HashMap;
use std::path::Path;

use crate::manifest::{MANIFEST_FILE, PluginManifest};
use crate::runtime::GuestPlugin;
use crate::ui;

/// 投递循环的最大轮数。插件 A 的写回会触发插件 B 的通知，B 在通知里又可以写入。
/// 超过这个轮数按成环处理：停止投递并把剩余回执交回调用方，不再重入 Guest。
pub const MAX_DELIVERY_ROUNDS: usize = 8;

/// 一个已装载的插件：身份来自 `aweft-plugin.json`，命令与订阅来自 Guest 自描述，
/// 面板源码来自插件目录。三者缺一项都不必让插件消失（面板坏了仍能后台工作）。
pub struct LoadedPlugin {
    pub id: String,
    pub title: String,
    /// 命令标签，下标即命令编号，与 Guest 导出 `aweft_on_command` 的参数一致。
    pub commands: Vec<String>,
    panel: Option<ComponentDefinition>,
    guest: GuestPlugin,
}

impl LoadedPlugin {
    pub fn panel(&self) -> Option<&ComponentDefinition> {
        self.panel.as_ref()
    }

    /// 面板状态行：内容由插件自己用 `set_status_line` 写出，宿主不参与拼装。
    pub fn status_lines(&self) -> &[String] {
        self.guest.status_lines()
    }

    /// Guest 经 `log_text` 写出的诊断行。
    pub fn log_lines(&self) -> &[String] {
        self.guest.log_lines()
    }

    fn subscribes_time_changed(&self) -> bool {
        self.guest.descriptor().time_changed
    }
}

/// 插件装载与通知投递。数据真相在使用方（GUI 的 `AppStore`、CLI 读到的文件），
/// 本类型只持有 Guest 实例与面板编译结果，不复制工程数据。
///
/// 通知走写日志：使用方写完一批变更后调用 [`PluginManager::deliver_pending`]，
/// 由它取走存储的写日志并分发。写入瞬间不做同步回调——插件在收到通知后
/// 会继续写同一个存储，同步回调等于在可变借用期间再次写入。
pub struct PluginManager {
    plugins: Vec<LoadedPlugin>,
}

impl PluginManager {
    pub fn new() -> Self {
        Self {
            plugins: Vec::new(),
        }
    }

    pub fn plugins(&self) -> &[LoadedPlugin] {
        &self.plugins
    }

    pub fn is_empty(&self) -> bool {
        self.plugins.is_empty()
    }

    pub fn plugin(&self, id: &str) -> Option<&LoadedPlugin> {
        self.plugins.iter().find(|plugin| plugin.id == id)
    }

    pub fn command_labels(&self, id: &str) -> Option<&[String]> {
        self.plugin(id).map(|plugin| plugin.commands.as_slice())
    }

    pub fn status_lines(&self, id: &str) -> Option<Vec<String>> {
        self.plugin(id)
            .map(|plugin| plugin.guest.status_lines().to_vec())
    }

    fn index_of(&self, id: &str) -> Result<usize> {
        self.plugins
            .iter()
            .position(|plugin| plugin.id == id)
            .with_context(|| format!("插件未加载：{id}"))
    }

    /// 装载一个插件目录：读清单、读 Guest 字节码、编译面板、校验 Guest 导出。
    /// Guest 装载失败（含描述 ABI 不匹配）时整个插件不进入列表。
    /// 面板缺失或编译失败只丢掉窗口，插件的命令与订阅仍然可用。
    pub fn load_dir(&mut self, dir: &Path) -> Result<()> {
        let manifest = PluginManifest::load_dir(dir)?;
        if self.plugin(&manifest.id).is_some() {
            bail!("插件重复加载：{}", manifest.id);
        }
        let wasm_bytes = manifest.read_guest(dir)?;
        let guest = GuestPlugin::load(&manifest.id, &wasm_bytes)?;

        let panel_source_path = manifest.panel_path(dir);
        let panel = match manifest.read_panel(dir) {
            Ok(source) => match ui::compile_panel(&source) {
                Ok(definition) => Some(definition),
                Err(e) => {
                    log::warn!("插件面板编译失败，仅关闭窗口：{}：{e:?}", panel_source_path.display());
                    None
                }
            },
            Err(e) => {
                log::warn!("插件面板读取失败，仅关闭窗口：{e:?}");
                None
            }
        };

        let descriptor = guest.descriptor().clone();
        self.plugins.push(LoadedPlugin {
            id: manifest.id.clone(),
            title: manifest.name.clone(),
            commands: descriptor.commands,
            panel,
            guest,
        });
        Ok(())
    }

    /// 扫描 `root` 的一层子目录，凡是含 `aweft-plugin.json` 的都尝试装载。
    /// 单个插件失败只记日志，不影响其余插件；返回成功装载的数量。
    /// 目录名按字典序遍历，保证插件间投递顺序稳定。
    pub fn load_root(&mut self, root: &Path) -> usize {
        let Ok(entries) = std::fs::read_dir(root) else {
            log::warn!("插件目录不可读，插件能力关闭：{}", root.display());
            return 0;
        };
        let mut dirs: Vec<_> = entries
            .flatten()
            .map(|entry| entry.path())
            .filter(|path| path.is_dir() && path.join(MANIFEST_FILE).is_file())
            .collect();
        dirs.sort();
        let mut loaded = 0;
        for dir in dirs {
            match self.load_dir(&dir) {
                Ok(()) => loaded += 1,
                Err(e) => log::warn!("插件装载失败（{}）：{e:?}", dir.display()),
            }
        }
        loaded
    }

    /// 取走存储的写日志并投递给订阅的插件。插件在通知里写回的行继续投递，
    /// 直到写日志空或达到轮数上限。
    ///
    /// 返回全部已投递的回执（含起点那一批与插件写回产生的）。调用方要把它们
    /// 映射成视图刷新标记，否则插件写回的行不会重画；达到轮数上限时，
    /// 末尾未投递的回执也包含在返回值里，只记一条告警。
    pub fn deliver_pending(&mut self, store: &mut ProjectStore) -> Vec<MutationReceipt> {
        let mut queue = store.take_pending();
        let mut delivered = queue.clone();
        let mut rounds = 0;
        while !queue.is_empty() && rounds < MAX_DELIVERY_ROUNDS {
            rounds += 1;
            let handles = handle_map(store);
            for receipt in &queue {
                match &receipt.kind {
                    MutationKind::EventTimeChanged(detail) => {
                        self.deliver_time_changed(store, &handles, &receipt.origin, detail);
                    }
                }
            }
            // 插件在通知里写回的行进写日志，下一轮继续投递。
            queue = store.take_pending();
            delivered.extend(queue.iter().cloned());
        }
        if !queue.is_empty() {
            log::warn!(
                "插件通知投递达到 {MAX_DELIVERY_ROUNDS} 轮上限，剩余 {} 条回执未投递",
                queue.len()
            );
        }
        delivered
    }

    fn deliver_time_changed(
        &mut self,
        store: &mut ProjectStore,
        handles: &HashMap<ItemId, i32>,
        origin: &Origin,
        detail: &EventTimeChanged,
    ) {
        let Some(&handle) = handles.get(&detail.item) else {
            return;
        };
        for plugin in &mut self.plugins {
            if !plugin.subscribes_time_changed() {
                continue;
            }
            // 回声过滤：插件自己触发的写入不再回投给它。
            if matches!(origin, Origin::Plugin(id) if *id == plugin.id) {
                continue;
            }
            if let Err(e) =
                plugin
                    .guest
                    .notify_time_changed(store, handle, detail.start_ms, detail.end_ms)
            {
                log::warn!("插件通知投递失败（{}）：{e:?}", plugin.id);
            }
        }
    }

    /// 执行一条插件命令，并把命令期间的写入投递给其它订阅者。
    /// 返回（Guest 状态码（0 表示成功），本次执行与投递产生的回执）。
    pub fn run_command(
        &mut self,
        store: &mut ProjectStore,
        plugin_id: &str,
        cmd_id: i32,
    ) -> Result<(i32, Vec<MutationReceipt>)> {
        let index = self.index_of(plugin_id)?;
        let declared = self.plugins[index].commands.len() as i32;
        if cmd_id < 0 || cmd_id >= declared {
            bail!("插件 {plugin_id} 未声明命令 {cmd_id}（自描述命令共 {declared} 条）");
        }
        let code = self.plugins[index].guest.invoke_command(store, cmd_id)?;
        Ok((code, self.deliver_pending(store)))
    }
}

impl Default for PluginManager {
    fn default() -> Self {
        Self::new()
    }
}

/// 行号（Guest 句柄）→ 行 id 的反向表。句柄按
/// `ProjectStore::ordered_event_ids` 的顺序编号，写入后顺序可能变化，
/// 因此每一轮投递前重建一次。
fn handle_map(store: &ProjectStore) -> HashMap<ItemId, i32> {
    store
        .ordered_event_ids()
        .into_iter()
        .enumerate()
        .map(|(index, id)| (id, index as i32))
        .collect()
}
