//! GUI 侧插件落点：装载插件、注册订阅者、持有每插件一个窗口、跑事件 pump。
//!
//! 编排（发现/装载/校验）内聚在 `aweft_plugins::PluginManager`，通知与回声过滤
//! 内聚在 `ProjectStore` 的广播，worker 线程与事件类型内聚在 `aweft_plugins`，
//! 本文件只是其中一个客户端的薄适配层：
//! 1. 启动期从目录装载插件（给 manager 一个事件发送端，接收端自己留着），
//!    顺带把声明了订阅的插件注册成工程存储的订阅者，并把插件名字写给 `PluginState`；
//! 2. 打开并持有插件自带的 Slint 窗口，把窗口按钮路由到 Guest 命令；
//! 3. 在 UI 线程 pump worker 事件：落真相 → 换刷新 tag → 同步面板（含 busy）。
//!
//! 这里不出现任何具体插件的名字、命令或文案：命令来自 Guest 自描述，
//! 面板来自插件目录，状态行由插件自己用宿主提供的 `set_status_line` 写出。

use aweft_plugins::{PluginEvent, PluginManager, PluginWindow, WorkerEvent};
use aweft_project::ProjectStore;
use futures::channel::mpsc::UnboundedReceiver;
use std::cell::RefCell;
use std::collections::HashMap;
use std::path::PathBuf;
use std::rc::Rc;

use crate::store::AppStore;

pub struct PluginBridge {
    manager: PluginManager,
    /// 已打开的插件窗口（每插件一个；丢弃守卫即关窗）。
    windows: HashMap<String, PluginWindow>,
    /// 事件接收端：`start_event_pump` 取走后为 `None`，之后走异步 pump；
    /// 测试（无事件循环）走 `poll_plugin_events` 同步 drain。
    /// 发送端只在装载时存在（克隆给每个 worker 后即丢弃，通道靠两边持有续命）。
    event_rx: Option<UnboundedReceiver<PluginEvent>>,
}

impl PluginBridge {
    /// 从插件根目录装载插件，并把声明了时间变更订阅的插件注册成 `store` 的订阅者。
    /// 装载失败只记日志：没有插件时主程序照常工作。
    ///
    /// 候选目录按存在与否探测（当前目录、其父目录、可执行文件同级下的 `plugins`），
    /// 覆盖"从仓库根跑"、"从 crate 目录跑测试"与"打包安装"三种布局。
    pub fn load(store: &mut ProjectStore) -> Self {
        let mut manager = PluginManager::new();
        let (event_tx, event_rx) = futures::channel::mpsc::unbounded::<PluginEvent>();
        for root in plugin_roots() {
            if !root.is_dir() {
                continue;
            }
            let loaded = manager.load_root(&root, store, &event_tx);
            if loaded > 0 {
                log::info!("从 {} 装载了 {loaded} 个插件", root.display());
            }
        }
        Self {
            manager,
            windows: HashMap::new(),
            event_rx: Some(event_rx),
        }
    }

    pub fn manager(&self) -> &PluginManager {
        &self.manager
    }

    pub fn manager_mut(&mut self) -> &mut PluginManager {
        &mut self.manager
    }

    /// 装载到的插件名字，顺序与 `plugins()` 一致，下标即 UI 上的按钮序号。
    pub fn titles(&self) -> Vec<String> {
        self.manager
            .plugins()
            .iter()
            .map(|plugin| plugin.title.clone())
            .collect()
    }

    fn plugin_id_at(&self, index: usize) -> Option<&str> {
        self.manager
            .plugins()
            .get(index)
            .map(|plugin| plugin.id.as_str())
    }

    /// 把某插件当前状态行写进它的窗口（没有窗口则什么都不做）。
    fn sync_window(&mut self, plugin_id: &str) {
        let Some(lines) = self.manager.status_lines(plugin_id) else {
            return;
        };
        if let Some(window) = self.windows.get_mut(plugin_id) {
            window.set_status_lines(&lines);
        }
    }

    pub(crate) fn sync_all_windows(&mut self) {
        let ids: Vec<String> = self.windows.keys().cloned().collect();
        for id in ids {
            self.sync_window(&id);
        }
    }

    /// 把某插件的 busy 态写进它的窗口（没有窗口则什么都不做）。
    fn set_window_busy(&mut self, plugin_id: &str, busy: bool) {
        if let Some(window) = self.windows.get_mut(plugin_id) {
            window.set_busy(busy);
        }
    }

    /// 同步 drain 事件通道（无事件循环时的客户端：测试、命令行）。
    /// 有异步 pump 在跑时接收端已被取走，此时返回空。
    pub fn poll_plugin_events(&mut self) -> Vec<PluginEvent> {
        let mut events = Vec::new();
        let Some(rx) = self.event_rx.as_mut() else {
            return events;
        };
        while let Ok(event) = rx.try_recv() {
            events.push(event);
        }
        events
    }
}

/// 插件事件的 UI 线程 pump（`spawn_local` 任务）：落真相 → 换刷新 tag →
/// 同步面板。接收端在这里取走，之后 `poll_plugin_events` 不再有东西。
/// await 点不持有任何 `store` 借用（与波形 pump 同一写法），回调里借才安全。
pub fn start_event_pump(store: &Rc<RefCell<AppStore>>) {
    use futures::StreamExt as _;
    let rx = match store.borrow_mut().plugin_bridge.event_rx.take() {
        Some(rx) => rx,
        None => {
            log::warn!("插件事件 pump 重复启动，忽略");
            return;
        }
    };
    let store_ref = Rc::clone(store);
    let _ = slint::spawn_local(async move {
        let mut rx = rx;
        while let Some((plugin_id, event)) = rx.next().await {
            store_ref
                .borrow_mut()
                .apply_plugin_event(&plugin_id, event);
        }
        log::info!("插件事件通道关闭，pump 退出");
    });
}

/// 插件根目录候选（去重后按顺序尝试）。
fn plugin_roots() -> Vec<PathBuf> {
    let mut roots: Vec<PathBuf> = Vec::new();
    let mut push = |path: PathBuf| {
        if !roots.contains(&path) {
            roots.push(path);
        }
    };
    if let Ok(cwd) = std::env::current_dir() {
        push(cwd.join("plugins"));
        if let Some(parent) = cwd.parent() {
            push(parent.join("plugins"));
        }
    }
    if let Ok(exe) = std::env::current_exe() {
        if let Some(dir) = exe.parent() {
            push(dir.join("plugins"));
        }
    }
    roots
}

impl AppStore {
    pub fn plugin_manager(&self) -> &PluginManager {
        &self.plugin_bridge.manager
    }

    /// 分发一条插件命令（面板按钮路由）：只做校验、快照与发送，立即返回。
    /// 执行在 worker 线程，产出经事件回来，由 pump（`start_event_pump` 或测试的
    /// `drain_plugin_events`）调 `apply_plugin_event` 落真相。忙时直接拒绝。
    pub fn run_plugin_command(&mut self, plugin_id: &str, cmd_id: i32) -> anyhow::Result<()> {
        {
            let AppStore {
                plugin_bridge,
                state,
                ..
            } = self;
            plugin_bridge
                .manager_mut()
                .run_command(&state.project, plugin_id, cmd_id)?;
        }
        self.plugin_bridge.set_window_busy(plugin_id, true);
        Ok(())
    }

    /// 落一个 worker 事件：写回真相（存储随即广播）→ 刷新队列换 tag →
    /// 重算投影 → 同步面板。`busy` 出清时同时清面板 busy 态。
    pub(crate) fn apply_plugin_event(&mut self, plugin_id: &str, event: WorkerEvent) {
        let busy_cleared = {
            let AppStore {
                plugin_bridge,
                state,
                ..
            } = self;
            match plugin_bridge
                .manager_mut()
                .apply_event(&mut state.project, plugin_id, event)
            {
                Ok(cleared) => cleared,
                Err(error) => {
                    log::warn!("插件事件落真相失败（{plugin_id}）：{error:?}");
                    return;
                }
            }
        };
        // 报错前已落进真相的写入同样要刷出来，所以队列先于判错处理
        //（此处沿用 `dispatch` 的顺序：先队列，后投影）。
        self.apply_refresh_queue();
        self.evaluate_depsgraph();
        if busy_cleared {
            self.plugin_bridge.set_window_busy(plugin_id, false);
        }
        self.plugin_bridge.sync_window(plugin_id);
    }

    /// 同步 drain 事件通道并逐个落盘（无事件循环时的客户端：测试、命令行）。
    pub fn drain_plugin_events(&mut self) {
        for (plugin_id, event) in self.plugin_bridge.poll_plugin_events() {
            self.apply_plugin_event(&plugin_id, event);
        }
    }

    /// 已打开的插件窗口数（测试断言用）。
    pub fn open_window_count(&self) -> usize {
        self.plugin_bridge.windows.len()
    }
}

/// 打开某插件的窗口（每插件一个；已可见则复用）。
/// 窗口内容是插件目录自带的 Slint 片段，按钮按命令编号回投给 Guest。
/// 开窗时按 manager 的 busy 真相同步一次面板 busy 态（命令可能在窗口打开前
/// 就已经发出，例如测试或其它客户端先点了命令）。
pub fn open_plugin_window(store: &Rc<RefCell<AppStore>>, plugin_id: &str) {
    let (definition, labels, lines, busy) = {
        let borrowed = store.borrow();
        let bridge = &borrowed.plugin_bridge;
        if bridge
            .windows
            .get(plugin_id)
            .is_some_and(PluginWindow::is_visible)
        {
            return;
        }
        let Some(plugin) = bridge.manager().plugin(plugin_id) else {
            log::warn!("插件未加载，无窗口可开：{plugin_id}");
            return;
        };
        let Some(definition) = plugin.panel().cloned() else {
            log::warn!("插件 {plugin_id} 没有可用面板（面板缺失或编译失败）");
            return;
        };
        (
            definition,
            plugin.commands.clone(),
            plugin.status_lines(),
            bridge.manager().is_busy(plugin_id),
        )
    };
    let store_ref = Rc::clone(store);
    let id = plugin_id.to_string();
    let window = match PluginWindow::open(&definition, &labels, &lines, move |cmd_id| {
        if let Err(e) = store_ref.borrow_mut().run_plugin_command(&id, cmd_id) {
            log::warn!("插件命令执行失败（{id}:{cmd_id}）：{e:?}");
        }
    }) {
        Ok(window) => window,
        Err(e) => {
            log::warn!("插件窗口打开失败（{plugin_id}）：{e:?}");
            return;
        }
    };
    window.set_busy(busy);
    store
        .borrow_mut()
        .plugin_bridge
        .windows
        .insert(plugin_id.to_string(), window);
}

/// 按 UI 按钮下标打开对应插件的窗口（下标来自 `PluginState.plugin-titles`）。
pub fn open_plugin_window_at(store: &Rc<RefCell<AppStore>>, index: i32) {
    let Some(plugin_id) = store
        .borrow()
        .plugin_bridge
        .plugin_id_at(index.max(0) as usize)
        .map(|id| id.to_string())
    else {
        log::warn!("插件按钮下标越界：{index}");
        return;
    };
    open_plugin_window(store, &plugin_id);
}
