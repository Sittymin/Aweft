//! 协作引擎：后台线程独占 `p2panda::Node` 与它自己的运行时。
//!
//! 线程桥接沿用插件 worker 的规矩（见 `aweft-plugins/src/worker.rs` 顶部注释）：
//! 真相 `ProjectStore` 是 `Rc` 单线程模型，而 `Node` 要求 `Send + Sync`，
//! 所以本模块持有全部异步对象，界面线程只经两条通道与它说话，且两条都不要求
//! 界面线程自己有运行时：
//! - 任务进：`tokio::sync::mpsc` 无界通道，`send` 是同步方法。
//! - 事件出：`futures` 无界通道，与插件事件同款，界面 `spawn_local` 轮询、
//!   测试里 `try_recv` 同步 drain。
//!
//! 后台线程绝不碰 `ProjectStore`：验算只用凭证与消息字节（`cap::authorize` 与
//! `roster`），落盘发生在界面线程收到 `CollabEvent::Remote` 之后。

use anyhow::{Context, Result};
use aweft_model::ids::ItemId;
use aweft_project::Origin;
use futures::channel::mpsc::UnboundedSender;
use p2panda::node::AckPolicy;
use p2panda::streams::{ProcessedOperation, StreamEvent, StreamFrom};
use p2panda::{Hash, NodeId};
use std::collections::{HashSet, VecDeque};
use std::path::PathBuf;
use tokio::sync::mpsc;
use tokio_stream::StreamExt;

use crate::cap::{Member, Namespace, authorize, decode_capability, member_id_of_bytes, path_from_wire};
use crate::roster::{Adoption, Roster, roster_path};
use crate::wire::{CollabMessage, PendingEdit};

/// 已验算过的操作哈希保留多少个，超过按先进先出淘汰。
const SEEN_CAPACITY: usize = 8192;

/// 打洞失败时用于中转的引导节点。
#[derive(Debug, Clone)]
pub struct Bootstrap {
    pub node_id: NodeId,
    /// 中继地址文本。解析失败只记日志，不让引擎起不来。
    pub relay_url: String,
}

/// 一个工程一会话的全部启动参数。
pub struct Session {
    /// 本机成员身份：签名密钥的公钥即 `willow` 子空间密钥的标识。
    pub member: Member,
    /// 本工程命名空间。成员侧只需标识：转授是在自己那张凭证后面追加一段，
    /// 命名空间密钥只用于签发邀请之外的创世凭证（见 `config::invite`）。
    pub namespace: Namespace,
    /// 本机当前持有的写凭证（`cap::encode_capability` 的产物）。
    /// 随每条消息携带，接收端独立验算；运行中被收紧后以名册里新的一张为准。
    pub capability: Vec<u8>,
    /// 节点数据库文件。`None` 用内存库，只给进程内测试。
    /// 真机必须给路径：本作者日志的高度只存在库里，重启后丢了就会从 0 重新长，
    /// 同一作者的两段日志会在对端打架。
    pub database: Option<PathBuf>,
    /// 引导节点：只在两端不能直连、需要中继转发时才用。
    pub bootstrap: Option<Bootstrap>,
    /// 本机签发过的凭证，启动时逐条重新通告。
    ///
    /// 需要它是因为名册靠通告建立：命令行里转授出去的凭证不落进任何人的日志，
    /// 新节点上线就无从知道「Carol 的现行凭证是 Bob 给的那一张」，
    /// 于是日后对 Carol 的收紧落不到这台机器上。
    pub announcements: Vec<Vec<u8>>,
}

/// 远端来的、已通过验算的一次行时间变更。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RemoteEdit {
    pub row: ItemId,
    pub start_ms: i64,
    pub end_ms: i64,
    /// 写入者的十六进制成员标识（即消息签名者），只用于日志与界面提示。
    pub author: String,
}

/// 引擎发往界面的事件。全部是 owned 数据，界面线程不需要借用任何东西。
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum CollabEvent {
    /// 节点与流已就绪，携带本节点标识供界面显示。
    Ready { node_id: String },
    /// 一条待写回的远端变更。
    Remote(RemoteEdit),
    /// 已连接的队友数变化。
    Peers { connected: usize },
    /// 本机凭证被换掉（收紧或撤销）。十六进制串直接写进 `ProjectRecord.capability`：
    /// 名册里本机那一条已经更新了，记录里还留着旧的一张会导致重启后写不回任何东西。
    Capability { capability: String },
    /// 引擎启动失败，随后线程退出。原因可直接展示。
    Failed { reason: String },
}

/// 界面线程发往引擎的任务。
enum CollabJob {
    Publish(Vec<PendingEdit>),
    /// 本机刚签出一张凭证：发出去通告，同时记进自己的名册。
    Announce(Vec<u8>),
    Shutdown,
}

/// 引擎句柄：持有任务发送端与线程，`Drop` 时要求退出并 join。
pub struct CollabEngine {
    jobs: mpsc::UnboundedSender<CollabJob>,
    thread: Option<std::thread::JoinHandle<()>>,
    origin: Origin,
}

impl CollabEngine {
    pub fn spawn(session: Session, events: UnboundedSender<CollabEvent>) -> Result<Self> {
        let origin = Origin::Collab(session.member.hex_id());
        let (jobs, jobs_rx) = mpsc::unbounded_channel();
        let handle = std::thread::Builder::new()
            .name("aweft-collab".into())
            .spawn(move || run(session, jobs_rx, events))
            .context("协作线程启动失败")?;
        Ok(Self {
            jobs,
            thread: Some(handle),
            origin,
        })
    }

    /// 本引擎写回真相时用的来源标记。注册成订阅者时用的也是这个值，
    /// 存储才会在广播时把远端写入过滤掉，不会让协作消息绕回自己再发一遍。
    pub fn origin(&self) -> Origin {
        self.origin.clone()
    }

    /// 交一批本地变更给后台发送。通道已关（引擎退出）时报错，调用方记日志即可。
    pub fn publish(&self, edits: Vec<PendingEdit>) -> Result<()> {
        self.jobs
            .send(CollabJob::Publish(edits))
            .map_err(|_| anyhow::anyhow!("协作引擎已退出，本次变更不发送"))
    }

    /// 通告一张本机刚签出的凭证（`cap::encode_capability` 的产物）。
    ///
    /// 后台线程会先把它记进自己的名册再发出去：转授者本应立即认得自己签的东西，
    /// 等自己日志的回声则意味着「离线期间签的权限，上线后有一段时间不好使」。
    pub fn announce(&self, capability: Vec<u8>) -> Result<()> {
        self.jobs
            .send(CollabJob::Announce(capability))
            .map_err(|_| anyhow::anyhow!("协作引擎已退出，本次通告不发送"))
    }
}

impl Drop for CollabEngine {
    fn drop(&mut self) {
        let _ = self.jobs.send(CollabJob::Shutdown);
        if let Some(handle) = self.thread.take() {
            let _ = handle.join();
        }
    }
}

fn run(
    session: Session,
    mut jobs: mpsc::UnboundedReceiver<CollabJob>,
    events: UnboundedSender<CollabEvent>,
) {
    let Ok(runtime) = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
    else {
        // 运行时都建不起来就没有可降级的路径，只能把原因报给界面。
        let _ = events.unbounded_send(CollabEvent::Failed {
            reason: format!("协作运行时创建失败（本机 {}）", session.member.hex_id()),
        });
        return;
    };
    runtime.block_on(async { serve(session, &mut jobs, &events).await });
    log::info!("协作引擎退出");
}

async fn serve(
    session: Session,
    jobs: &mut mpsc::UnboundedReceiver<CollabJob>,
    events: &UnboundedSender<CollabEvent>,
) {
    let mut builder = p2panda::builder()
        .signing_key(session.member.signing_key())
        // 显式确认策略：投递即确认会让崩溃时还没处理的变更凭空消失，
        // 我们要在验算完并交给界面之后才确认。
        .ack_policy(AckPolicy::Explicit);
    if let Some(path) = &session.database {
        builder = builder.database_url(&format!("sqlite://{}", path.display()));
    }
    if let Some(bootstrap) = &session.bootstrap {
        match bootstrap.relay_url.parse() {
            Ok(relay_url) => builder = builder.bootstrap(bootstrap.node_id, relay_url),
            Err(error) => log::warn!("中继地址解析失败，按无引导启动：{error:?}"),
        }
    }
    let node = match builder.spawn().await {
        Ok(node) => node,
        Err(error) => {
            let _ = events.unbounded_send(CollabEvent::Failed {
                reason: format!(
                    "协作节点启动失败（本机 {}）：{error:?}",
                    session.member.hex_id()
                ),
            });
            return;
        }
    };
    let (publisher, mut subscription) =
        match node
            .stream_from::<CollabMessage>(session.namespace.topic(), StreamFrom::Frontier)
            .await
        {
            Ok(stream) => stream,
            Err(error) => {
                let _ = events.unbounded_send(CollabEvent::Failed {
                    reason: format!(
                        "协作流创建失败（本机 {}）：{error:?}",
                        session.member.hex_id()
                    ),
                });
                return;
            }
        };
    let _ = events.unbounded_send(CollabEvent::Ready {
        node_id: node.id().to_hex(),
    });

    let mut seen = SeenOperations::default();
    let mut peers: HashSet<NodeId> = HashSet::new();
    // 名册在节点上线之前打开：本机凭证解不开就没有可验算的东西，与其带着空名册上线
    // （空名册等于「不认识即放行」，收紧会静默失效），不如直接报失败。
    let mut roster = match Roster::open(
        session.capability.clone(),
        roster_path(session.database.as_deref()),
    ) {
        Ok(roster) => roster,
        Err(error) => {
            let _ = events.unbounded_send(CollabEvent::Failed {
                reason: format!(
                    "协作名册打不开（本机 {}）：{error:?}",
                    session.member.hex_id()
                ),
            });
            return;
        }
    };
    // 补发本机签发过的凭证：名册靠通告建立，而命令行里转授那一步没有引擎在场。
    // 名册里已经有那张的会被 `announce` 挡下不发，这不算丢通告：名册文件与日志库
    // 同目录成对，本机记得它就说明当初那条操作还在自己的日志里，连上来的对端随日志
    // 同步照样拿到。真正补的是「记录里有而名册里没有」那一段。
    for capability in session.announcements.clone() {
        announce(&mut roster, &publisher, &session.member, capability, events).await;
    }
    loop {
        tokio::select! {
            job = jobs.recv() => {
                match job {
                    None | Some(CollabJob::Shutdown) => break,
                    Some(CollabJob::Publish(edits)) => {
                        for edit in edits {
                            let message = CollabMessage::Edit {
                                track: edit.track,
                                row: edit.row,
                                start_ms: edit.start_ms,
                                end_ms: edit.end_ms,
                                capability: roster.own().to_vec(),
                            };
                            publish(&publisher, message).await;
                        }
                    }
                    Some(CollabJob::Announce(capability)) => {
                        announce(&mut roster, &publisher, &session.member, capability, events).await;
                    }
                }
            }
            event = subscription.next() => {
                let Some(event) = event else { break };
                match event {
                    StreamEvent::Processed { operation, .. } => {
                        if let Some(event) = receive(&session, &mut roster, &mut seen, &operation) {
                            let _ = events.unbounded_send(event);
                        }
                        // 采纳还是丢弃都确认：确认的是「我已看过并判完」，
                        // 不确认的话同一条操作每次重连都要再来一遍。
                        if let Err(error) = operation.ack().await {
                            log::warn!("协作操作确认失败：{error:?}");
                        }
                    }
                    StreamEvent::SyncStarted { remote_node_id, .. } => {
                        report_peers(&mut peers, &remote_node_id, true, events);
                    }
                    StreamEvent::SyncEnded { remote_node_id, .. } => {
                        report_peers(&mut peers, &remote_node_id, false, events);
                    }
                    StreamEvent::DecodeFailed { error, .. } => {
                        // 非本协议的消息：流层已丢弃，记一条就够。
                        log::warn!("协作消息解码失败：{error:?}");
                    }
                    StreamEvent::ProcessingFailed { error, .. } => {
                        log::warn!("协作操作处理失败：{error:?}");
                    }
                    other => log::debug!("协作流事件不予处理：{other:?}"),
                }
            }
        }
    }
}

async fn publish(publisher: &p2panda::streams::StreamPublisher<CollabMessage>, message: CollabMessage) {
    match publisher.publish(message).await {
        // 第二个 await 等到这条操作真正落进自己的日志：落不进就等于没发出。
        Ok(future) => {
            if let Err(error) = future.await {
                log::warn!("协作变更未落进本地日志：{error:?}");
            }
        }
        Err(error) => log::warn!("协作变更发布失败：{error:?}"),
    }
}

/// 本机签出一张凭证后：先记进自己的名册，再向全场通告。
async fn announce(
    roster: &mut Roster,
    publisher: &p2panda::streams::StreamPublisher<CollabMessage>,
    member: &Member,
    capability: Vec<u8>,
    events: &UnboundedSender<CollabEvent>,
) {
    let cap = match decode_capability(&capability) {
        Ok(cap) => cap,
        Err(error) => {
            log::warn!("本机签发的凭证解不开，不通告：{error:?}");
            return;
        }
    };
    match roster.apply(member.id(), &cap) {
        Ok(Adoption::Ignored) => {
            log::warn!("本机名册不采纳自己签发的凭证：手里的权限可能已被收紧");
            return;
        }
        Ok(Adoption::Ours) => {
            let _ = events.unbounded_send(CollabEvent::Capability {
                capability: hex::encode(&capability),
            });
        }
        Err(error) => {
            log::warn!("凭证通告处理失败：{error:?}");
            return;
        }
        Ok(Adoption::Accepted) => {}
    }
    publish(publisher, CollabMessage::Grant { capability }).await;
}

fn report_peers(
    peers: &mut HashSet<NodeId>,
    remote: &NodeId,
    connected: bool,
    events: &UnboundedSender<CollabEvent>,
) {
    let changed = if connected {
        peers.insert(*remote)
    } else {
        peers.remove(remote)
    };
    if changed {
        let _ = events.unbounded_send(CollabEvent::Peers {
            connected: peers.len(),
        });
    }
}

/// 一条已收到的操作：验算通过才变成待处理事件，否则 `None`（调用方仍要确认它）。
fn receive(
    session: &Session,
    roster: &mut Roster,
    seen: &mut SeenOperations,
    operation: &ProcessedOperation<CollabMessage>,
) -> Option<CollabEvent> {
    if !seen.insert_if_new(operation.id()) {
        return None;
    }
    let author = match member_id_of_bytes(operation.author().as_bytes()) {
        Ok(author) => author,
        Err(error) => {
            log::warn!("协作消息作者标识不合法，丢弃：{error:?}");
            return None;
        }
    };
    let capability = match operation.message() {
        CollabMessage::Edit { capability, .. } => capability,
        CollabMessage::Grant { capability } => capability,
    };
    let cap = match decode_capability(capability) {
        Ok(cap) => cap,
        Err(error) => {
            log::warn!("协作消息里的凭证无效，丢弃：{error:?}");
            return None;
        }
    };
    match operation.message() {
        CollabMessage::Edit {
            track,
            row,
            start_ms,
            end_ms,
            ..
        } => {
            let (path, row) = match path_from_wire(track, row) {
                Ok(parsed) => parsed,
                Err(error) => {
                    log::warn!("协作消息路径不合法，丢弃：{error:?}");
                    return None;
                }
            };
            if let Err(error) = authorize(&session.namespace, &author, &cap, &path) {
                log::warn!("协作写入被拒（行 {row}）：{error:?}");
                return None;
            }
            // 链合法只说明「签发当时授权过」。收紧之后同一位作者的旧凭证
            // 必须在这里被拦下，否则名册白做。
            if !roster.is_current(&author, &cap) {
                log::warn!("协作写入用的是过期凭证（行 {row}），丢弃");
                return None;
            }
            Some(CollabEvent::Remote(RemoteEdit {
                row,
                start_ms: *start_ms,
                end_ms: *end_ms,
                author: hex::encode(author.to_bytes()),
            }))
        }
        CollabMessage::Grant { .. } => {
            if cap.granted_namespace() != session.namespace.id() {
                log::warn!("凭证通告不属于本工程命名空间，丢弃");
                return None;
            }
            match roster.apply(&author, &cap) {
                Ok(Adoption::Ours) => Some(CollabEvent::Capability {
                    capability: hex::encode(capability),
                }),
                Ok(Adoption::Accepted) => {
                    log::info!("已登记一条凭证通告（成员 {}）", hex::encode(author.to_bytes()));
                    None
                }
                Ok(Adoption::Ignored) => {
                    log::debug!("凭证通告未被采纳：宣布者给不起这张凭证");
                    None
                }
                Err(error) => {
                    log::warn!("凭证通告处理失败：{error:?}");
                    None
                }
            }
        }
    }
}

/// 按操作哈希去重。
///
/// 这是语义正确的一部分，不是性能优化：`p2panda` 只保证至少一次送达，
/// 而本设计的冲突规则是「后到达的写入覆盖先到达的」，重投的旧操作若不拦下来，
/// 就会把已经落地的新值压回去。
#[derive(Default)]
struct SeenOperations {
    set: HashSet<Hash>,
    order: VecDeque<Hash>,
}

impl SeenOperations {
    /// 第一次见到这条操作时返回 `true`。
    fn insert_if_new(&mut self, id: Hash) -> bool {
        if !self.set.insert(id) {
            return false;
        }
        self.order.push_back(id);
        if self.order.len() > SEEN_CAPACITY {
            if let Some(old) = self.order.pop_front() {
                self.set.remove(&old);
            }
        }
        true
    }
}
