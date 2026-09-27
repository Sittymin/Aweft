//! 插件 worker 线程：每插件一个，独占其 `GuestPlugin`。
//!
//! 设计约束（与 `aweft-gui` 解耦的关键）：
//! - 本模块不碰 `ProjectStore`：快照由调用方在发 job 前打包好随 job 进来，
//!   脏行随 event 出去，写回真相是客户端调 `PluginManager::apply_event` 的事。
//! - 本模块不持有事件接收端：`events` 发送端在 `spawn` 时由客户端给，
//!   pump 归各客户端自己（图形界面 `spawn_local`，命令行/测试 `try_next` 轮询）。
//! - job 通道用 `std::sync::mpsc`（worker 侧阻塞 `recv`），事件通道用
//!   `futures` 无界通道（客户端侧既可 `.await` 也可 `try_next`）。

use aweft_model::ids::ItemId;
use futures::channel::mpsc::UnboundedSender;
use std::sync::mpsc::{Receiver, Sender};

use crate::runtime::GuestPlugin;

/// Guest 可见的一行快照。句柄 = 本向量下标，宿主与 Guest 按
/// `ProjectStore::ordered_event_ids` 的确定性顺序对齐。
/// 必须是 `pub`：`WorkerEvent` 的公有字段里有它（否则 `private_interfaces` 告警）。
#[derive(Debug, Clone)]
pub struct RowSnap {
    pub id: ItemId,
    pub start_ms: i64,
    pub end_ms: i64,
}

/// worker 发往客户端的事件。全部是 owned 数据（`Send`），面板刷新只读这些，
/// 永远不需要回头借 Guest。
pub type PluginEvent = (String, WorkerEvent);

/// 单个插件的事件。`Progress` 在 Guest 调用中途直发（执行中刷新面板）；
/// `CommandFinished`/`NotificationsApplied` 携带写回真相所需的脏行；
/// `GuestTrapped` 表示 Guest 陷阱，脏写已丢弃。
#[derive(Debug, Clone)]
pub enum WorkerEvent {
    Progress {
        status_lines: Vec<String>,
        log_lines: Vec<String>,
    },
    CommandFinished {
        cmd_id: i32,
        code: i32,
        dirty: Vec<RowSnap>,
        status_lines: Vec<String>,
    },
    NotificationsApplied {
        dirty: Vec<RowSnap>,
    },
    GuestTrapped {
        context: String,
    },
}

/// 一次行时间变更通知（`MutationReceipt` 的线程可发送投影）。
#[derive(Debug, Clone)]
pub(crate) struct TimeNotice {
    pub item: ItemId,
    pub start_ms: i64,
    pub end_ms: i64,
}

/// 发往 worker 的任务。同一通道 FIFO：命令与通知的先后顺序即处理顺序。
pub(crate) enum PluginJob {
    RunCommand {
        cmd_id: i32,
        snapshot: Vec<RowSnap>,
    },
    NotifyTimeChanged {
        notices: Vec<TimeNotice>,
        snapshot: Vec<RowSnap>,
    },
    Shutdown,
}

/// worker 线程句柄：持有 job 发送端与线程。`Drop` 时发 `Shutdown` 并 join，
///
/// 注意：worker 若卡在某次 Guest 调用里（v1 无 fuel 上限），`Drop` 会等到调用
/// 返回。失控 Guest 的强杀是后续加固的事，不在本轮。
pub(crate) struct PluginWorker {
    jobs: Sender<PluginJob>,
    handle: Option<std::thread::JoinHandle<()>>,
}

impl PluginWorker {
    pub(crate) fn spawn(
        id: &str,
        mut guest: GuestPlugin,
        events: UnboundedSender<PluginEvent>,
    ) -> Self {
        guest.set_event_sink(id.to_string(), events);
        let (tx, rx) = std::sync::mpsc::channel::<PluginJob>();
        let owned_id = id.to_string();
        let handle = std::thread::Builder::new()
            .name(format!("aweft-plugin-{owned_id}"))
            .spawn(move || run_loop(&owned_id, guest, rx))
            .expect("插件 worker 线程启动失败");
        Self {
            jobs: tx,
            handle: Some(handle),
        }
    }

    pub(crate) fn sender(&self) -> Sender<PluginJob> {
        self.jobs.clone()
    }
}

impl Drop for PluginWorker {
    fn drop(&mut self) {
        let _ = self.jobs.send(PluginJob::Shutdown);
        if let Some(handle) = self.handle.take() {
            let _ = handle.join();
        }
    }
}

/// worker 主循环。Guest 调用前后装快照/取脏行：与旧同步路径相同的快照语义，
/// 只是快照随 job 进来、脏行随 event 出去，中间不借任何外部存储。
/// 事件一律经 `GuestPlugin::emit`（直发通道在 ctx 里），主循环不另持发送端。
fn run_loop(id: &str, mut guest: GuestPlugin, jobs: Receiver<PluginJob>) {
    for job in jobs {
        match job {
            PluginJob::Shutdown => break,
            PluginJob::RunCommand { cmd_id, snapshot } => {
                guest.install_snapshot(snapshot);
                match guest.call_command(cmd_id) {
                    Ok(code) => {
                        let event = WorkerEvent::CommandFinished {
                            cmd_id,
                            code,
                            dirty: guest.take_dirty_rows(),
                            status_lines: guest.status_lines(),
                        };
                        if guest.emit(event).is_err() {
                            break;
                        }
                    }
                    Err(error) => {
                        // 陷阱前的脏写随快照丢弃：没写回即没发生，真相不受影响。
                        let _ = guest.take_dirty_rows();
                        log::warn!("插件命令陷阱（{id}:{cmd_id}）：{error:?}");
                        let event = WorkerEvent::GuestTrapped {
                            context: format!("{error:?}"),
                        };
                        if guest.emit(event).is_err() {
                            break;
                        }
                    }
                }
            }
            PluginJob::NotifyTimeChanged { notices, snapshot } => {
                guest.install_snapshot(snapshot);
                for notice in notices {
                    let Some(handle) = guest.handle_of(notice.item) else {
                        continue;
                    };
                    if let Err(error) =
                        guest.call_time_changed(handle, notice.start_ms, notice.end_ms)
                    {
                        log::warn!("插件通知投递失败（{id}）：{error:?}");
                        break;
                    }
                }
                // 脏行恒定发出（即使为空）：客户端的 drain 循环有确定的结束信号，
                // 不必猜 worker 是否已经处理完。
                let event = WorkerEvent::NotificationsApplied {
                    dirty: guest.take_dirty_rows(),
                };
                if guest.emit(event).is_err() {
                    break;
                }
            }
        }
    }
    log::info!("插件 worker 退出（{id}）");
}
