use anyhow::{bail, Context, Result};
use aweft_project::{
    MutationKind, MutationReceipt, MutationSubscriber, Origin, ProjectStore, SubscriberRef,
};
use futures::channel::mpsc::UnboundedSender;
use slint_interpreter::ComponentDefinition;
use std::path::Path;
use std::rc::Rc;

use crate::ctx::snapshot_rows;
use crate::manifest::{PluginManifest, MANIFEST_FILE};
use crate::runtime::GuestPlugin;
use crate::ui;
use crate::worker::{PluginEvent, PluginJob, PluginWorker, RowSnap, TimeNotice, WorkerEvent};

/// 客户端侧日志缓存上限：防止健谈的 Guest 把内存吃光（v1 的简易背压）。
const MAX_LOG_LINES: usize = 500;

/// 一个已装载的插件：身份来自 `aweft-plugin.json`，命令与订阅来自 Guest 自描述，
/// 面板源码来自插件目录。三者缺一项都不必让插件消失（面板坏了仍能后台工作）。
///
/// Guest 本体住在 worker 线程里（`worker` 拥有），本类型只持任务发送端与
/// 显示缓存：UI 线程永不直接调 Guest，因此也不存在跨存储写入的借用问题——
/// 快照随 job 进去、脏行随 event 出来。
pub struct LoadedPlugin {
    pub id: String,
    pub title: String,
    /// 命令标签，下标即命令编号，与 Guest 导出 `aweft_on_command` 的参数一致。
    pub commands: Vec<String>,
    panel: Option<ComponentDefinition>,
    /// worker 线程的所有者：持有它即保持线程存活，丢弃即要求退出并 join。
    /// 任务发送端按需从它克隆，不另存一份。
    worker: PluginWorker,
    /// 是否有命令正在 worker 上跑。置位后新的 `run_command` 直接拒绝，
    /// 面板侧由插件作者按 `command-busy` 自行置灰按钮（宿主只给信号，不代劳）。
    busy: bool,
    time_changed: bool,
    status_cache: Vec<String>,
    log_cache: Vec<String>,
}

impl LoadedPlugin {
    pub fn panel(&self) -> Option<&ComponentDefinition> {
        self.panel.as_ref()
    }

    /// 面板状态行：worker 经事件回传的缓存，pump 落盘后立即可读。
    pub fn status_lines(&self) -> Vec<String> {
        self.status_cache.clone()
    }

    /// Guest 经 `log_text` 写出的诊断行（同上，缓存）。
    pub fn log_lines(&self) -> Vec<String> {
        self.log_cache.clone()
    }

    fn subscribes_time_changed(&self) -> bool {
        self.time_changed
    }
}

/// 把 Guest 写过的行放回工程存储。写入时存储会广播，因此调用方必须已经
/// 不再借用任何插件状态——本路径下 Guest 远在 worker 线程，不存在嵌套借用。
fn write_back(origin: &Origin, id: &str, store: &mut ProjectStore, rows: Vec<RowSnap>) {
    for row in rows {
        if let Err(e) = store.set_event_times_ms(origin.clone(), row.id, row.start_ms, row.end_ms) {
            log::warn!("插件写回真相失败（{id}）：{e:?}");
        }
    }
}

/// 插件侧的变更订阅者。订阅关系由 Guest 的描述 JSON 决定：声明了
/// `time_changed` 的插件才会注册进来，未声明的插件收不到任何通知。
///
/// 只做快照与发送：广播发生在存储写入中途（调用方可能正借着存储），这里绝不
/// 调 Guest、不碰通道以外的任何东西；真正的 Guest 调用在 worker 线程里发生。
struct PluginSubscriber {
    id: String,
    origin: Origin,
    jobs: std::sync::mpsc::Sender<PluginJob>,
}

impl MutationSubscriber for PluginSubscriber {
    fn origin(&self) -> Origin {
        self.origin.clone()
    }

    fn on_mutations(&self, store: &mut ProjectStore, mutations: &[MutationReceipt]) {
        let notices: Vec<TimeNotice> = mutations
            .iter()
            .map(|receipt| {
                let MutationKind::EventTimeChanged(detail) = &receipt.kind;
                TimeNotice {
                    item: detail.item,
                    start_ms: detail.start_ms,
                    end_ms: detail.end_ms,
                }
            })
            .collect();
        if notices.is_empty() {
            return;
        }
        let job = PluginJob::NotifyTimeChanged {
            notices,
            snapshot: snapshot_rows(store),
        };
        if self.jobs.send(job).is_err() {
            log::warn!("插件 worker 已退出，通知丢弃（{}）", self.id);
        }
    }
}

/// 插件装载与命令分发。数据真相在使用方手里（图形界面、命令行……）：
/// 本类型不复制工程数据，`run_command` 只读快照并发送，写回发生在客户端调
/// `apply_event` 时——由各客户端自己的 pump 驱动，与界面框架无关。
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

    /// 某插件是否有命令正在 worker 上跑（面板 `command-busy` 的宿主侧真相）。
    pub fn is_busy(&self, id: &str) -> bool {
        self.plugin(id).is_some_and(|plugin| plugin.busy)
    }

    fn index_of(&self, id: &str) -> Result<usize> {
        self.plugins
            .iter()
            .position(|plugin| plugin.id == id)
            .with_context(|| format!("插件未加载：{id}"))
    }

    /// 装载一个插件目录：读清单、读 Guest 字节码、编译面板、校验 Guest 导出，
    /// 起 worker 线程，并把声明了 `time_changed` 的插件注册成 `store` 的订阅者。
    /// Guest 装载失败（含描述 ABI 不匹配）时整个插件不进入列表。
    /// 面板缺失或编译失败只丢掉窗口，插件的命令与订阅仍然可用。
    ///
    /// 事件接收端归调用方（客户端）：`event_tx` 的另一头由各客户端自己的 pump
    /// 持有，本成员只发不收。
    pub fn load_dir(
        &mut self,
        dir: &Path,
        store: &mut ProjectStore,
        event_tx: &UnboundedSender<PluginEvent>,
    ) -> Result<()> {
        let manifest = PluginManifest::load_dir(dir)?;
        if self.plugin(&manifest.id).is_some() {
            bail!("插件重复加载：{}", manifest.id);
        }
        let wasm_bytes = manifest.read_guest(dir)?;
        let guest = GuestPlugin::load(&wasm_bytes, &manifest.permissions)
            .with_context(|| format!("插件 Guest 装载失败：{}", manifest.id))?;

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

        let descriptor = guest.descriptor().clone();
        let worker = PluginWorker::spawn(&manifest.id, guest, event_tx.clone());
        if descriptor.time_changed {
            store.subscribe(Rc::new(PluginSubscriber {
                id: manifest.id.clone(),
                origin: Origin::Plugin(manifest.id.clone()),
                jobs: worker.sender(),
            }) as SubscriberRef);
        }
        self.plugins.push(LoadedPlugin {
            id: manifest.id.clone(),
            title: manifest.name.clone(),
            commands: descriptor.commands,
            panel,
            worker,
            busy: false,
            time_changed: descriptor.time_changed,
            status_cache: Vec::new(),
            log_cache: Vec::new(),
        });
        Ok(())
    }

    /// 扫描 `root` 的一层子目录，凡是含 `aweft-plugin.json` 的都尝试装载。
    /// 单个插件失败只记日志，不影响其余插件；返回成功装载的数量。
    /// 目录名按字典序遍历，保证插件间投递顺序稳定。
    pub fn load_root(
        &mut self,
        root: &Path,
        store: &mut ProjectStore,
        event_tx: &UnboundedSender<PluginEvent>,
    ) -> usize {
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
            match self.load_dir(&dir, store, event_tx) {
                Ok(()) => loaded += 1,
                Err(e) => log::warn!("插件装载失败（{}）：{e:?}", dir.display()),
            }
        }
        loaded
    }

    /// 分发一条插件命令：只做校验、快照与发送，立即返回，不等 Guest 跑完。
    /// 真正的执行在 worker 线程，产出经事件回来，客户端在 pump 里调
    /// `apply_event` 落真相。忙时（上一条还没跑完）直接拒绝，不排队——
    /// 排队语义以后真有插件要再加描述开关，v1 不做。
    pub fn run_command(
        &mut self,
        store: &ProjectStore,
        plugin_id: &str,
        cmd_id: i32,
    ) -> Result<()> {
        let index = self.index_of(plugin_id)?;
        let plugin = &mut self.plugins[index];
        let declared = plugin.commands.len() as i32;
        if cmd_id < 0 || cmd_id >= declared {
            bail!("插件 {plugin_id} 未声明命令 {cmd_id}（自描述命令共 {declared} 条）");
        }
        if plugin.busy {
            bail!("插件 {plugin_id} 正忙：上一条命令尚未完成，不排队");
        }
        plugin.busy = true;
        let job = PluginJob::RunCommand {
            cmd_id,
            snapshot: snapshot_rows(store),
        };
        if plugin.worker.sender().send(job).is_err() {
            plugin.busy = false;
            bail!("插件 worker 已退出，命令无法投递（{plugin_id}）");
        }
        Ok(())
    }

    /// 落一个 worker 事件：刷新显示缓存；脏行在批量作用域里写回真相
    /// （一条命令算一次交互，其间所有写入合成一次广播，语义与旧同步路径一致）。
    /// 返回 `busy` 是否因此出清——客户端据此刷新面板的 `command-busy`。
    pub fn apply_event(
        &mut self,
        store: &mut ProjectStore,
        plugin_id: &str,
        event: WorkerEvent,
    ) -> Result<bool> {
        let index = self.index_of(plugin_id)?;
        match event {
            WorkerEvent::Progress {
                status_lines,
                log_lines,
            } => {
                let plugin = &mut self.plugins[index];
                plugin.status_cache = status_lines;
                plugin.log_cache = log_lines;
                if plugin.log_cache.len() > MAX_LOG_LINES {
                    let excess = plugin.log_cache.len() - MAX_LOG_LINES;
                    plugin.log_cache.drain(..excess);
                }
                Ok(false)
            }
            WorkerEvent::CommandFinished {
                cmd_id,
                code,
                dirty,
                status_lines,
            } => {
                if code != 0 {
                    log::warn!("插件命令返回非零状态码（{plugin_id}:{cmd_id}）：{code}");
                }
                let plugin = &mut self.plugins[index];
                plugin.status_cache = status_lines;
                let batch = store.begin_batch();
                write_back(
                    &Origin::Plugin(plugin_id.to_string()),
                    plugin_id,
                    store,
                    dirty,
                );
                // 出错也要收口：`hold` 落不回零，之后的写入就永远不再通知订阅者。
                store.end_batch(batch);
                self.plugins[index].busy = false;
                Ok(true)
            }
            WorkerEvent::NotificationsApplied { dirty } => {
                if dirty.is_empty() {
                    return Ok(false);
                }
                let batch = store.begin_batch();
                write_back(
                    &Origin::Plugin(plugin_id.to_string()),
                    plugin_id,
                    store,
                    dirty,
                );
                store.end_batch(batch);
                Ok(false)
            }
            WorkerEvent::GuestTrapped { context } => {
                log::warn!("插件 Guest 陷阱已丢弃脏写（{plugin_id}）：{context}");
                self.plugins[index].busy = false;
                Ok(true)
            }
        }
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
