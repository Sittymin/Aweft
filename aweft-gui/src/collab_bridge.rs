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
use aweft_collab::cap::Member;
use aweft_collab::engine::{CollabEngine, CollabEvent};
use aweft_collab::subscriber;
use aweft_project::ProjectStore;
use futures::channel::mpsc::UnboundedReceiver;
use slint::ComponentHandle;
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
    member: Option<Member>,
    session_name: Option<String>,
    peer_count: i32,
    status: String,
}

impl CollabBridge {
    /// 装载协作：读身份与活跃会话，可用时起引擎并登记订阅者。
    pub fn load(store: &mut ProjectStore) -> Self {
        let idle = Self {
            engine: None,
            namespace: None,
            events: None,
            member: None,
            session_name: None,
            peer_count: 0,
            status: "未连接".into(),
        };
        let member = match config::load_or_create_member() {
            Ok(member) => member,
            Err(error) => {
                log::warn!("协作身份不可用，协作待命：{error:?}");
                return idle;
            }
        };
        let Some(namespace) = config::active_namespace() else {
            return Self { member: Some(member), ..idle };
        };
        let record = match config::load_record(&namespace) {
            Ok(record) => record,
            Err(error) => {
                log::warn!("活跃协作会话读不出，协作待命：{error:?}");
                return Self { member: Some(member), ..idle };
            }
        };
        let session = match config::session(&record, &member) {
            Ok(session) => session,
            Err(error) => {
                log::warn!(
                    "工程「{}」的协作会话拼不出来，协作待命：{error:?}",
                    record.name
                );
                return Self { member: Some(member), ..idle };
            }
        };
        let (tx, rx) = futures::channel::mpsc::unbounded::<CollabEvent>();
        let engine = match CollabEngine::spawn(session, tx) {
            Ok(engine) => Rc::new(engine),
            Err(error) => {
                log::warn!("协作引擎启动失败，协作待命：{error:?}");
                return Self { member: Some(member), ..idle };
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
            member: Some(member),
            session_name: Some(record.name),
            peer_count: 0,
            status: "连接中".into(),
        }
    }

    pub fn member_id(&self) -> String {
        self.member.as_ref().map(Member::hex_id).unwrap_or_default()
    }

    pub fn session_name(&self) -> String {
        self.session_name.clone().unwrap_or_default()
    }

    pub fn create_session(&mut self, store: &mut ProjectStore, name: &str) -> anyhow::Result<()> {
        if self.engine.is_some() {
            self.leave(store)?;
        }
        let member = config::load_or_create_member()?;
        let record = config::create_project(name.trim(), &member)?;
        config::set_active(&record.namespace)?;
        self.start_record(store, member, record)
    }

    pub fn accept_invite(&mut self, store: &mut ProjectStore, invite: &str) -> anyhow::Result<()> {
        if self.engine.is_some() {
            self.leave(store)?;
        }
        let member = config::load_or_create_member()?;
        let record = config::accept(invite.trim(), &member)?;
        config::set_active(&record.namespace)?;
        self.start_record(store, member, record)
    }

    pub fn make_invite(&mut self, member_id: &str) -> anyhow::Result<String> {
        let member = self.member.as_ref().ok_or_else(|| anyhow::anyhow!("协作身份未初始化"))?;
        let namespace = self.namespace.as_ref().ok_or_else(|| anyhow::anyhow!("尚未加入协作会话"))?;
        let mut record = config::load_record(namespace)?;
        let invite = config::invite(&mut record, &member, member_id.trim(), config::Scope::All)?;
        let engine = self.engine.as_ref().expect("active session has an engine");
        engine.announce(config::invite_capability(&invite)?)?;
        Ok(invite)
    }

    pub fn leave(&mut self, store: &mut ProjectStore) -> anyhow::Result<()> {
        if let Some(engine) = &self.engine {
            let origin = engine.origin();
            store.unsubscribe_origin(&origin);
        }
        config::clear_active()?;
        self.engine = None;
        self.events = None;
        self.namespace = None;
        self.session_name = None;
        self.peer_count = 0;
        self.status = "未连接".into();
        Ok(())
    }

    fn start_record(&mut self, store: &mut ProjectStore, member: Member, record: config::ProjectRecord) -> anyhow::Result<()> {
        let session = config::session(&record, &member)?;
        let (tx, rx) = futures::channel::mpsc::unbounded();
        let engine = Rc::new(CollabEngine::spawn(session, tx)?);
        subscriber::register(store, Rc::clone(&engine));
        self.member = Some(member);
        self.namespace = Some(record.namespace);
        self.session_name = Some(record.name);
        self.engine = Some(engine);
        self.events = Some(rx);
        self.peer_count = 0;
        self.status = "连接中".into();
        Ok(())
    }

    pub fn is_active(&self) -> bool {
        self.engine.is_some()
    }

    pub fn peer_count(&self) -> i32 { self.peer_count }

    fn is_owner(&self) -> bool {
        let Some(ns) = &self.namespace else { return false };
        let Ok(record) = config::load_record(ns) else { return false };
        record.namespace_secret.is_some()
    }

    fn publish_snapshot(&self, store: &ProjectStore) -> anyhow::Result<()> {
        let Some(engine) = &self.engine else { return Ok(()) };
        if !self.is_owner() {
            return Ok(());
        }
        // 非空工程才值得发快照
        if store.sequence().items.is_empty() {
            return Ok(());
        }
        let data = store.encode_snapshot()?;
        log::info!("已发送工程快照（{} 字节）", data.len());
        engine.publish_snapshot(data)?;
        Ok(())
    }

    pub fn status(&self) -> &str { &self.status }

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
            CollabEvent::Ready { node_id } => {
                self.collab_bridge.status = "等待远端连接".into();
                log::info!("协作节点就绪：{node_id}");
            }
            CollabEvent::Peers { connected } => {
                let prev = self.collab_bridge.peer_count;
                self.collab_bridge.peer_count = connected.try_into().unwrap_or(i32::MAX);
                self.collab_bridge.status = if connected == 0 { "等待远端连接" } else { "已连接" }.into();
                log::info!("协作对端数：{connected}");
                // 初次加入时：所有者把整份工程推给空工程的加入者（会丢失加入者本地进度）
                if prev == 0 && connected > 0 {
                    if let Err(e) = self.collab_bridge.publish_snapshot(&self.state.project) {
                        log::warn!("快照发送失败：{e:?}");
                    }
                }
            }
            CollabEvent::Failed { reason } => {
                self.collab_bridge.status = format!("连接失败：{reason}");
                log::error!("协作引擎退出：{reason}");
            }
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
            CollabEvent::Snapshot { data, author } => {
                // 初次加入的快照：按需求直接覆盖本地（会丢失本地进度，后续再做合并）
                if author == self.collab_bridge.member_id() {
                    log::info!("收到自己发出的快照，回环忽略");
                    return;
                }
                match ProjectStore::decode_snapshot(&data, &author) {
                    Ok(store) => {
                        log::warn!("收到完整工程快照（来自 {author}），本地将被覆盖");
                        self.state.project = store;
                        // 旧 store 的订阅者（watch/collab）随替换已丢，全部重建
                        self.refresh_queue = crate::store::project_watch::register(&mut self.state.project);
                        if let Some(engine) = &self.collab_bridge.engine {
                            subscriber::register(&mut self.state.project, Rc::clone(engine));
                        }
                        self.rebuild_sequence_models();
                        self.apply_refresh_queue();
                        self.evaluate_depsgraph();
                        log::info!("快照已应用，当前行数 {}", self.state.project.sequence().items.len());
                    }
                    Err(e) => log::warn!("快照未采纳，本地不变：{e:?}"),
                }
            }
        }
        self.sync_collab_ui();
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

    pub(crate) fn sync_collab_ui(&self) {
        let Some(window) = self.main_window.as_ref().and_then(|weak| weak.upgrade()) else { return; };
        let state = window.global::<crate::bridge::types::CollabUiState>();
        state.set_member_id(self.collab_bridge.member_id().into());
        state.set_session_name(self.collab_bridge.session_name().into());
        state.set_status_text(self.collab_bridge.status().into());
        state.set_peer_count(self.collab_bridge.peer_count());
    }
}
