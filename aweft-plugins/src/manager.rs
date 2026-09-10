use anyhow::{bail, Context, Result};
use aweft_project::{
    MutationKind, MutationReceipt, MutationSubscriber, Origin, ProjectStore, SubscriberRef,
};
use slint_interpreter::ComponentDefinition;
use std::cell::RefCell;
use std::path::Path;
use std::rc::Rc;

use crate::ctx::RowSnap;
use crate::manifest::{PluginManifest, MANIFEST_FILE};
use crate::runtime::GuestPlugin;
use crate::ui;

/// 一个已装载的插件：身份来自 `aweft-plugin.json`，命令与订阅来自 Guest 自描述，
/// 面板源码来自插件目录。三者缺一项都不必让插件消失（面板坏了仍能后台工作）。
///
/// Guest 放在 `Rc<RefCell<_>>` 里：它既是工程存储的订阅者（存储持有它），
/// 又是命令执行的对象（管理器借用它），两边都不能把借用跨过对存储的写入。
pub struct LoadedPlugin {
    pub id: String,
    pub title: String,
    /// 命令标签，下标即命令编号，与 Guest 导出 `aweft_on_command` 的参数一致。
    pub commands: Vec<String>,
    panel: Option<ComponentDefinition>,
    guest: Rc<RefCell<GuestPlugin>>,
}

impl LoadedPlugin {
    pub fn panel(&self) -> Option<&ComponentDefinition> {
        self.panel.as_ref()
    }

    /// 面板状态行：内容由插件自己用 `set_status_line` 写出，宿主不参与拼装。
    pub fn status_lines(&self) -> Vec<String> {
        self.guest.borrow().status_lines()
    }

    /// Guest 经 `log_text` 写出的诊断行。
    pub fn log_lines(&self) -> Vec<String> {
        self.guest.borrow().log_lines()
    }

    fn subscribes_time_changed(&self) -> bool {
        self.guest.borrow().descriptor().time_changed
    }
}

/// 把 Guest 写过的行放回工程存储。写入时存储会广播，因此调用方必须已经
/// 释放对该插件 Guest 的借用，否则广播会绕回本插件形成嵌套借用。
fn write_back(origin: &Origin, id: &str, store: &mut ProjectStore, rows: Vec<RowSnap>) {
    for row in rows {
        if let Err(e) = store.set_event_times_ms(origin.clone(), row.id, row.start_ms, row.end_ms) {
            log::warn!("插件写回真相失败（{id}）：{e:?}");
        }
    }
}

/// 插件侧的变更订阅者。订阅关系由 Guest 的描述 JSON 决定：声明了
/// `time_changed` 的插件才会注册进来，未声明的插件收不到任何通知。
struct PluginSubscriber {
    id: String,
    origin: Origin,
    guest: Rc<RefCell<GuestPlugin>>,
}

impl MutationSubscriber for PluginSubscriber {
    fn origin(&self) -> Origin {
        self.origin.clone()
    }

    fn on_mutations(&self, store: &mut ProjectStore, mutations: &[MutationReceipt]) {
        let diffs = {
            let mut guest = self.guest.borrow_mut();
            guest.sync_from_store(store);
            for receipt in mutations {
                let MutationKind::EventTimeChanged(detail) = &receipt.kind;
                let Some(handle) = guest.handle_of(detail.item) else {
                    continue;
                };
                if let Err(e) = guest.call_time_changed(handle, detail.start_ms, detail.end_ms) {
                    log::warn!("插件通知投递失败（{}）：{e:?}", self.id);
                }
            }
            guest.take_dirty_rows()
        };
        write_back(&self.origin, &self.id, store, diffs);
    }
}

/// 插件装载与命令执行。数据真相在 `ProjectStore`：本类型不复制工程数据，
/// 也不负责通知——订阅者身份在装载时交给了存储，由存储在写入时广播。
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
        self.plugin(id).map(|plugin| plugin.status_lines())
    }

    fn index_of(&self, id: &str) -> Result<usize> {
        self.plugins
            .iter()
            .position(|plugin| plugin.id == id)
            .with_context(|| format!("插件未加载：{id}"))
    }

    /// 装载一个插件目录：读清单、读 Guest 字节码、编译面板、校验 Guest 导出，
    /// 并把声明了 `time_changed` 的插件注册成 `store` 的订阅者。
    /// Guest 装载失败（含描述 ABI 不匹配）时整个插件不进入列表。
    /// 面板缺失或编译失败只丢掉窗口，插件的命令与订阅仍然可用。
    pub fn load_dir(&mut self, dir: &Path, store: &mut ProjectStore) -> Result<()> {
        let manifest = PluginManifest::load_dir(dir)?;
        if self.plugin(&manifest.id).is_some() {
            bail!("插件重复加载：{}", manifest.id);
        }
        let wasm_bytes = manifest.read_guest(dir)?;
        let guest = Rc::new(RefCell::new(
            GuestPlugin::load(&wasm_bytes, &manifest.permissions)
                .with_context(|| format!("插件 Guest 装载失败：{}", manifest.id))?,
        ));

        let panel_source_path = manifest.panel_path(dir);
        let panel = match manifest.read_panel(dir) {
            Ok(source) => match ui::compile_panel(&source) {
                Ok(definition) => Some(definition),
                Err(e) => {
                    log::warn!(
                        "插件面板编译失败，仅关闭窗口：{}：{e:?}",
                        panel_source_path.display()
                    );
                    None
                }
            },
            Err(e) => {
                log::warn!("插件面板读取失败，仅关闭窗口：{e:?}");
                None
            }
        };

        let descriptor = guest.borrow().descriptor().clone();
        if descriptor.time_changed {
            store.subscribe(Rc::new(PluginSubscriber {
                id: manifest.id.clone(),
                origin: Origin::Plugin(manifest.id.clone()),
                guest: Rc::clone(&guest),
            }) as SubscriberRef);
        }
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
    pub fn load_root(&mut self, root: &Path, store: &mut ProjectStore) -> usize {
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
            match self.load_dir(&dir, store) {
                Ok(()) => loaded += 1,
                Err(e) => log::warn!("插件装载失败（{}）：{e:?}", dir.display()),
            }
        }
        loaded
    }

    /// 执行一条插件命令。命令期间的写入落回真相存储，存储随即广播，
    /// 其余订阅者收到；本插件按回声过滤收不到自己那次写入。
    /// 一条命令算一次交互：其间所有写入合成一次广播，订阅者不会被逐行打扰。
    pub fn run_command(
        &mut self,
        store: &mut ProjectStore,
        plugin_id: &str,
        cmd_id: i32,
    ) -> Result<i32> {
        let index = self.index_of(plugin_id)?;
        let declared = self.plugins[index].commands.len() as i32;
        if cmd_id < 0 || cmd_id >= declared {
            bail!("插件 {plugin_id} 未声明命令 {cmd_id}（自描述命令共 {declared} 条）");
        }
        let batch = store.begin_batch();
        let outcome = self.exec_command(store, index, plugin_id, cmd_id);
        // 出错也要收口：`hold` 落不回零，之后的写入就永远不再通知订阅者。
        store.end_batch(batch);
        outcome
    }

    fn exec_command(
        &mut self,
        store: &mut ProjectStore,
        index: usize,
        plugin_id: &str,
        cmd_id: i32,
    ) -> Result<i32> {
        let (code, diffs) = {
            let mut guest = self.plugins[index].guest.borrow_mut();
            guest.sync_from_store(store);
            let code = guest.call_command(cmd_id)?;
            (code, guest.take_dirty_rows())
        };
        write_back(
            &Origin::Plugin(plugin_id.to_string()),
            plugin_id,
            store,
            diffs,
        );
        Ok(code)
    }

    /// 已注册为存储订阅者的插件数（诊断用：装载了多少个订阅 time_changed 的插件）。
    pub fn subscribed_count(&self) -> usize {
        self.plugins
            .iter()
            .filter(|plugin| plugin.subscribes_time_changed())
            .count()
    }
}

impl Default for PluginManager {
    fn default() -> Self {
        Self::new()
    }
}
