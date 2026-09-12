//! GUI 侧插件落点：装载插件、注册订阅者、持有每插件一个面板工厂、跑事件 pump。
//!
//! 编排（发现/装载/校验）内聚在 `aweft_plugins::PluginManager`，通知与回声过滤
//! 内聚在 `ProjectStore` 的广播，worker 线程与事件类型内聚在 `aweft_plugins`，
//! 本文件只是其中一个客户端的薄适配层：
//! 1. 启动期从目录装载插件（给 manager 一个事件发送端，接收端自己留着），
//!    顺带把声明了订阅的插件注册成工程存储的订阅者，并把有面板的插件写给 Area 切换菜单；
//! 2. 按插件惰性建出面板的 `ComponentFactory` 并缓存，交给 Area 里的
//!    `ComponentContainer` 嵌入，面板按钮按插件 id 路由回 Guest 命令；
//! 3. 在 UI 线程 pump worker 事件：落真相 → 换刷新 tag → 同步面板（含 busy）。
//!
//! 这里不出现任何具体插件的名字、命令或文案：命令来自 Guest 自描述，
//! 面板来自插件目录，状态行由插件自己用宿主提供的 `set_status_line` 写出。

use aweft_plugins::{PanelHandle, PluginEvent, PluginManager, WorkerEvent};
use aweft_project::ProjectStore;
use futures::channel::mpsc::UnboundedReceiver;
use std::cell::RefCell;
use std::collections::HashMap;
use std::path::PathBuf;
use std::rc::{Rc, Weak};

use crate::store::AppStore;

pub struct PluginBridge {
    manager: PluginManager,
    /// 每插件一个面板工厂，第一次被 Area 要时才建（建工厂要借 `AppStore` 的弱句柄，
    /// 而那时装载已经完成）。缓存是必需的：`ComponentFactory` 克隆后指针相等，
    /// Area 投影重算时写回同一个值，Slint 就不会重建容器里已经嵌好的面板。
    panels: RefCell<HashMap<String, PanelHandle>>,
    /// 面板上的按钮要投递命令，而命令要过 `AppStore`。`AppStore` 的 `Rc` 在自己的
    /// 构造期还不存在，所以由 `attach_store` 事后补进来。
    self_store: RefCell<Weak<RefCell<AppStore>>>,
    /// 事件接收端：`start_event_pump` 取走后为 `None`，之后走异步 pump；
    /// 测试（无事件循环）走 `poll_plugin_events` 同步 drain。
    /// 发送端只在装载时存在（克隆给每个 worker 后即丢弃，通道靠两边持有续命）。
    event_rx: Option<UnboundedReceiver<PluginEvent>>,
}

impl PluginBridge {
    /// 从插件根目录装载插件，并把声明了时间变更订阅的插件注册成 `store` 的订阅者。
    /// 装载失败只记日志：没有插件时主程序照常工作。
    ///
    /// 候选目录按存在与否探测（设置里配的插件根、配置目录下的 `plugins`，再是
    /// 当前目录、其父目录、可执行文件同级下的 `plugins`），
    /// 覆盖"用户装机"与"从仓库根跑"、"从 crate 目录跑测试"、"打包安装"这几种布局。
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
            panels: RefCell::new(HashMap::new()),
            self_store: RefCell::new(Weak::new()),
            event_rx: Some(event_rx),
        }
    }

    /// 补上承载本桥的 `AppStore` 句柄（面板命令的路由目标）。
    pub fn attach_store(&mut self, store: &Rc<RefCell<AppStore>>) {
        *self.self_store.borrow_mut() = Rc::downgrade(store);
    }

    pub fn manager(&self) -> &PluginManager {
        &self.manager
    }

    pub fn manager_mut(&mut self) -> &mut PluginManager {
        &mut self.manager
    }

    /// 可以成为一个 Area 的插件（即有可用面板），顺序与 `plugins()` 一致。
    /// 每项是 (插件 id, 展示名)，喂 Area 切换菜单的插件一列。
    pub fn panel_areas(&self) -> Vec<(String, String)> {
        self.manager
            .plugins()
            .iter()
            .filter(|plugin| plugin.panel().is_some())
            .map(|plugin| (plugin.id.clone(), plugin.title.clone()))
            .collect()
    }

    /// 取某插件面板的嵌入工厂（第一次要时建）。插件不存在或没有可用面板时 `None`。
    ///
    /// 只在 Area 投影里调，此时 `AppStore` 正被可变借用——所以这里绝不碰 store：
    /// 工厂闭包读的是 `PanelHandle` 自持的值快照，命令投递要到用户点击时才借 store。
    pub fn panel_factory(&self, plugin_id: &str) -> Option<slint::ComponentFactory> {
        if let Some(handle) = self.panels.borrow().get(plugin_id) {
            return Some(handle.factory());
        }
        let plugin = self.manager.plugin(plugin_id)?;
        let definition = plugin.panel()?;
        let handle = PanelHandle::new(
            definition,
            &plugin.commands,
            &plugin.status_lines(),
            self.manager.is_busy(plugin_id),
            {
                let store = self.self_store.borrow().clone();
                let id = plugin_id.to_string();
                move |cmd_id| {
                    let Some(store) = store.upgrade() else {
                        log::warn!("宿主已释放，插件命令丢弃（{id}:{cmd_id}）");
                        return;
                    };
                    if let Err(e) = store.borrow_mut().run_plugin_command(&id, cmd_id) {
                        log::warn!("插件命令执行失败（{id}:{cmd_id}）：{e:?}");
                    }
                }
            },
        );
        let factory = handle.factory();
        self.panels
            .borrow_mut()
            .insert(plugin_id.to_string(), handle);
        Some(factory)
    }

    /// 把 manager 的当前真相（状态行 + busy）推给某插件的所有活动面板实例。
    /// 没有面板时什么都不做。
    pub fn sync_panel(&self, plugin_id: &str) {
        let Some(lines) = self.manager.status_lines(plugin_id) else {
            return;
        };
        let busy = self.manager.is_busy(plugin_id);
        if let Some(handle) = self.panels.borrow().get(plugin_id) {
            handle.sync(&lines, busy);
        }
    }

    pub(crate) fn sync_all_panels(&self) {
        let ids: Vec<String> = self.panels.borrow().keys().cloned().collect();
        for id in ids {
            self.sync_panel(&id);
        }
    }

    /// 某插件当前活着的面板实例数（测试断言"切走 Area 即释放面板"用）。
    pub fn panel_instance_count(&self, plugin_id: &str) -> usize {
        self.panels
            .borrow()
            .get(plugin_id)
            .map(PanelHandle::live_instances)
            .unwrap_or(0)
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
/// 前两项是"用户装插件的地方"：设置里显式配的根优先，其次是配置目录下的 `plugins`
///（Android 上配置目录就是外部数据目录，`adb push` 进得去，见
/// `docs/tmp/plugin-panel-android.md` §8）。后三项是开发期兜底。
fn plugin_roots() -> Vec<PathBuf> {
    let mut roots: Vec<PathBuf> = Vec::new();
    let mut push = |path: PathBuf| {
        if !roots.contains(&path) {
            roots.push(path);
        }
    };
    if let Some(root) = aweft_settings::settings().paths.plugin_root.clone() {
        push(root);
    }
    if let Some(dir) = aweft_settings::config_dir() {
        push(dir.join("plugins"));
    }
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
        self.plugin_bridge.sync_panel(plugin_id);
        Ok(())
    }

    /// 落一个 worker 事件：写回真相（存储随即广播）→ 刷新队列换 tag →
    /// 重算投影 → 同步面板。忙态出清与状态行更新都在 `sync_panel` 里按真相做。
    pub(crate) fn apply_plugin_event(&mut self, plugin_id: &str, event: WorkerEvent) {
        {
            let AppStore {
                plugin_bridge,
                state,
                ..
            } = self;
            if let Err(error) = plugin_bridge
                .manager_mut()
                .apply_event(&mut state.project, plugin_id, event)
            {
                log::warn!("插件事件落真相失败（{plugin_id}）：{error:?}");
                return;
            }
        }
        // 报错前已落进真相的写入同样要刷出来，所以队列先于判错处理
        //（此处沿用 `dispatch` 的顺序：先队列，后投影）。
        self.apply_refresh_queue();
        self.evaluate_depsgraph();
        self.plugin_bridge.sync_panel(plugin_id);
    }

    /// 同步 drain 事件通道并逐个落盘（无事件循环时的客户端：测试、命令行）。
    pub fn drain_plugin_events(&mut self) {
        for (plugin_id, event) in self.plugin_bridge.poll_plugin_events() {
            self.apply_plugin_event(&plugin_id, event);
        }
    }

    /// 某插件当前嵌入在多少个 Area 里（测试断言用）。
    pub fn panel_instance_count(&self, plugin_id: &str) -> usize {
        self.plugin_bridge.panel_instance_count(plugin_id)
    }
}
