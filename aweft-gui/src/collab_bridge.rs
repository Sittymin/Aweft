//! GUI 侧协作落点：按活跃会话起后台引擎、把协作登记成工程存储的订阅者、
//! 再把引擎事件写回真相并刷出 tag。
//!
//! 与 `plugin_bridge` 同构，因为它解决的是同一个问题：真相是单线程 `Rc` 模型，
//! 而网络在后台线程。区别只在装载的判据——插件看目录，协作看配置里的活跃会话：
//!
//! - `collab/active` 不存在（首启的正常状态）或身份/会话读不出：只记日志，
//!   主程序照常工作，界面不需要知道协作存在过。
//! - 会话可用：起引擎、注册订阅者，之后本地每一批时间变更由订阅者交给引擎发送。
//!
//! 引擎的事件一律在 UI 线程落盘（异步 pump 或测试的 `drain_collab_events`），
//! 后台线程从不碰 `ProjectStore`，见 `aweft_collab::engine` 顶部的线程桥接说明。

use aweft_collab::config;
use aweft_collab::engine::{CollabEngine, CollabEvent};
use aweft_collab::subscriber;
use aweft_project::ProjectStore;
use futures::channel::mpsc::UnboundedReceiver;
use std::cell::RefCell;
use std::rc::Rc;

use crate::store::AppStore;

pub struct CollabBridge {
    /// 活跃会话的引擎。`None` 表示协作未启用，此时订阅者也没登记。
    engine: Option<Rc<CollabEngine>>,
    /// 引擎来自哪一份会话记录：本机凭证被换掉时要写回同一份。
    /// 只留标识不留记录本身，见 [`CollabBridge::persist_capability`]。
    namespace: Option<String>,
    /// 事件接收端：`start_event_pump` 取走后为 `None`，之后走异步 pump；
    /// 无事件循环的客户端（测试）走 `poll_events` 同步 drain。
    events: Option<UnboundedReceiver<CollabEvent>>,
}

impl CollabBridge {
    /// 装载协作：读身份与活跃会话，可用时起引擎并登记订阅者。
    pub fn load(store: &mut ProjectStore) -> Self {
        let idle = Self {
            engine: None,
            namespace: None,
            events: None,
        };
        let member = match config::load_or_create_member() {
            Ok(member) => member,
            Err(error) => {
                log::warn!("协作身份不可用，协作待命：{error:?}");
                return idle;
            }
        };
        let Some(namespace) = config::active_namespace() else {
            log::info!("未指定协作工程（collab/active 不存在），协作待命");
            return idle;
        };
        let record = match config::load_record(&namespace) {
            Ok(record) => record,
            Err(error) => {
                log::warn!("活跃协作会话读不出，协作待命：{error:?}");
                return idle;
            }
        };
        let session = match config::session(&record, &member) {
            Ok(session) => session,
            Err(error) => {
                log::warn!(
                    "工程「{}」的协作会话拼不出来，协作待命：{error:?}",
                    record.name
                );
                return idle;
            }
        };
        let (tx, rx) = futures::channel::mpsc::unbounded::<CollabEvent>();
        let engine = match CollabEngine::spawn(session, tx) {
            Ok(engine) => Rc::new(engine),
            Err(error) => {
                log::warn!("协作引擎启动失败，协作待命：{error:?}");
                return idle;
            }
        };
        subscriber::register(store, Rc::clone(&engine));
        log::info!(
            "已加入协作工程「{}」，本机标识 {}",
            record.name,
            member.hex_id()
        );
        Self {
            engine: Some(engine),
            namespace: Some(namespace),
            events: Some(rx),
        }
    }

    pub fn is_active(&self) -> bool {
        self.engine.is_some()
    }

    /// 同步 drain 事件通道（无事件循环时的客户端：测试、命令行）。
    /// 有异步 pump 在跑时接收端已被取走，此时返回空。
    pub fn poll_events(&mut self) -> Vec<CollabEvent> {
        let mut events = Vec::new();
        let Some(rx) = self.events.as_mut() else {
            return events;
        };
        while let Ok(event) = rx.try_recv() {
            events.push(event);
        }
        events
    }

    /// 本机凭证被换掉（上级收紧，或上级重签了一张更宽/更窄的）后写回会话记录。
    ///
    /// 这一笔必须落盘：引擎名册里本机那一条在收到通告时就已换新，而重启时
    /// `Roster::load` 让会话记录压过名册文件（那是识别过期记录的判据）。记录里
    /// 留着旧的一张，重启后本机就以过期凭证上岗，两处后果：
    ///
    /// 1. 自己的名册把旧凭证当作本机现行那一张，所以过期写入在本机一切正常，
    ///    只有听过收紧的对端会静默丢掉——本机看不到任何异常。
    /// 2. 给自己下游重签也不成：新凭证只能从现行那张往下推（`roster::authorises`
    ///    的 `extends`），拿旧的当基础，通告会被自己的名册判成无权采纳。
    ///
    /// 现读现写，不复用启动期那份记录：命令行可能在这期间往同一条记录里
    /// 追加过签发条目，拿旧副本整份写回会把它们抹掉。
    fn persist_capability(&self, capability: &str) {
        let Some(namespace) = &self.namespace else {
            log::error!("本机协作凭证已更换，但没有活跃会话可写回，重启后会失效");
            return;
        };
        let outcome = config::load_record(namespace)
            .and_then(|mut record| config::save_capability(&mut record, capability));
        match outcome {
            Ok(()) => log::warn!("本机协作凭证已更换，新凭证已写回会话记录"),
            Err(error) => {
                log::error!("新凭证写回会话记录失败，重启后本机可能被自己的名册挡住：{error:?}")
            }
        }
    }
}

/// 协作事件的 UI 线程 pump：远端变更落真相 → 换刷新 tag → 重算投影。
/// await 点不持有任何 `store` 借用（与插件、波形 pump 同一写法）。
pub fn start_event_pump(store: &Rc<RefCell<AppStore>>) {
    use futures::StreamExt as _;
    let Some(rx) = store.borrow_mut().collab_bridge.events.take() else {
        return; // 未启用协作，或 pump 重复启动
    };
    let store_ref = Rc::clone(store);
    let _ = slint::spawn_local(async move {
        let mut rx = rx;
        while let Some(event) = rx.next().await {
            store_ref.borrow_mut().apply_collab_event(event);
        }
    });
}

impl AppStore {
    /// 落一条协作事件。只有远端变更要写真相，其余是节点自身的状态变化，记日志。
    pub(crate) fn apply_collab_event(&mut self, event: CollabEvent) {
        match event {
            CollabEvent::Ready { node_id } => log::info!("协作节点就绪：{node_id}"),
            CollabEvent::Peers { connected } => log::info!("协作对端数：{connected}"),
            CollabEvent::Failed { reason } => log::error!("协作引擎退出：{reason}"),
            CollabEvent::Capability { capability } => {
                self.collab_bridge.persist_capability(&capability)
            }
            CollabEvent::Remote(edit) => {
                // 写入即广播：本机订阅者按 `Origin::Collab(_)` 整类跳过，不会再发回去。
                subscriber::apply_remote(&mut self.state.project, &edit);
                // 沿用 `dispatch` 的顺序：先取回排队的外来写入换成 tag，再重算投影。
                self.apply_refresh_queue();
                self.evaluate_depsgraph();
            }
        }
    }

    /// 同步 drain 协作事件并逐个落盘（测试、命令行）。
    pub fn drain_collab_events(&mut self) {
        for event in self.collab_bridge.poll_events() {
            self.apply_collab_event(event);
        }
    }

    /// 协作是否已启用（面板与命令行用它决定要不要显示状态）。
    pub fn collab_is_active(&self) -> bool {
        self.collab_bridge.is_active()
    }
}
