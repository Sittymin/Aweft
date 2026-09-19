//! 插件闭环测试：目录装载 → Guest 自描述 → worker 线程执行 → 事件落真相。
//!
//! 被测插件是仓库里的真实插件目录 `plugins/test-plugin`（清单 + 已提交的
//! `guest.wasm` + `panel.slint`），不是为测试另写的夹具：跑通它即跑通示例。
//! Rust 源码在 `plugins/test-plugin/guest`，用 `plugins/test-plugin/build.sh` 重编。
//!
//! 线程模型：命令分发（`run_command`）只做快照与发送，Guest 在 worker 线程跑，
//! 产出经事件通道回来，测试扮演客户端 pump（`apply_event` 落真相）。
//! 因此"写入返回前通知送达"已是过去式：写入返回时 job 刚发出，
//! 测试用 `drain_until` 等到条件成立。

use std::cell::RefCell;
use std::path::PathBuf;
use std::rc::Rc;
use std::time::{Duration, Instant};

use aweft_model::ids::ItemId;
use aweft_model::sequence::{EventData, ItemLocation};
use aweft_plugins::{PluginEvent, PluginManager, WorkerEvent, MANIFEST_FILE};
use aweft_project::{
    MutationReceipt, MutationSubscriber, Origin, ProjectStore, SavedProject, SubscriberRef,
};
use chrono::TimeDelta;
use futures::channel::mpsc::{UnboundedReceiver, unbounded};

const PLUGIN_ID: &str = "test-plugin";

/// 等一个插件事件的最长时限：worker 与测试同机，正常几毫秒即到；
/// 超时只意味着实现挂了（死锁/事件丢失），此时 panic 而不是无限卡住。
/// 取 30 秒而非 10 秒：延迟命令本身就要跑 10.2 秒（100×100ms 睡眠+开销），
/// 等它完成的 drain 必须比它更长——之前用 10 秒恰好在完成前超时，教训。
const EVENT_TIMEOUT: Duration = Duration::from_secs(30);

fn plugin_dir() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../plugins/test-plugin")
}

fn event_data(start_ms: i64, end_ms: i64) -> EventData {
    EventData {
        start_time: TimeDelta::milliseconds(start_ms),
        end_time: TimeDelta::milliseconds(end_ms),
        actor: None,
        text: String::new(),
        segment_timing: None,
        style: String::new(),
    }
}

fn store_with_events(times: &[(i64, i64)]) -> (ProjectStore, Vec<ItemId>) {
    let mut store = ProjectStore::new(SavedProject::new());
    let mut ids = Vec::new();
    for (index, (start, end)) in times.iter().enumerate() {
        let id = store
            .saved_mut()
            .project
            .sequence_mut()
            .create_event(event_data(*start, *end), ItemLocation::Root(index))
            .expect("测试事件行创建失败");
        ids.push(id);
    }
    (store, ids)
}

/// 装载真实插件目录并建好行，同时建好事件通道（测试是"客户端"，接收端自己拿）。
/// 播种经 `document_mut` 直改，不产生回执，插件收不到。
fn manager_with_events(
    times: &[(i64, i64)],
) -> (
    PluginManager,
    ProjectStore,
    Vec<ItemId>,
    UnboundedReceiver<PluginEvent>,
) {
    let (mut store, ids) = store_with_events(times);
    let mut manager = PluginManager::new();
    let (tx, rx) = unbounded::<PluginEvent>();
    manager
        .load_dir(&plugin_dir(), &mut store, &tx)
        .expect("插件目录装载失败");
    (manager, store, ids, rx)
}

/// 阻塞等下一个插件事件（带超时，见 `EVENT_TIMEOUT`）。
/// 注意用 `try_recv` 而不是已弃用的 `try_next`：后者的 `Ok(None)`/`Err`
/// 语义恰好反直觉（`Ok(None)` = 已关闭，`Err` = 开着但没货），踩过一次。
fn next_event(rx: &mut UnboundedReceiver<PluginEvent>) -> PluginEvent {
    let deadline = Instant::now() + EVENT_TIMEOUT;
    loop {
        match rx.try_recv() {
            Ok(event) => return event,
            Err(error) if error.is_empty() => {
                if Instant::now() > deadline {
                    panic!("等待插件事件超时");
                }
                std::thread::sleep(Duration::from_millis(1));
            }
            Err(_) => panic!("插件事件通道已关闭"),
        }
    }
}

/// 分发一条命令并 pump 到 busy 出清，返回 Guest 的状态码。
/// `GuestTrapped` 视为测试失败（示例插件不应陷阱）。
fn run_command_sync(
    manager: &mut PluginManager,
    store: &mut ProjectStore,
    rx: &mut UnboundedReceiver<PluginEvent>,
    cmd_id: i32,
) -> i32 {
    manager
        .run_command(&*store, PLUGIN_ID, cmd_id)
        .expect("命令分发失败");
    loop {
        let (id, event) = next_event(rx);
        assert_eq!(id, PLUGIN_ID, "事件来源插件不对");
        let code = match &event {
            WorkerEvent::CommandFinished { code, .. } => Some(*code),
            WorkerEvent::GuestTrapped { context } => panic!("Guest 陷阱：{context}"),
            _ => None,
        };
        let cleared = manager
            .apply_event(store, &id, event)
            .expect("事件落盘失败");
        if cleared {
            return code.expect("出清 busy 的必是 Finished");
        }
    }
}

/// pump 事件直到 `cond` 成立（用于通知类断言：写回不一定有，条件看缓存）。
fn drain_until(
    manager: &mut PluginManager,
    store: &mut ProjectStore,
    rx: &mut UnboundedReceiver<PluginEvent>,
    mut cond: impl FnMut(&PluginManager) -> bool,
) {
    let deadline = Instant::now() + EVENT_TIMEOUT;
    while !cond(manager) {
        if Instant::now() > deadline {
            panic!("等待插件事件超时");
        }
        match rx.try_recv() {
            Ok((id, event)) => {
                manager
                    .apply_event(store, &id, event)
                    .expect("事件落盘失败");
            }
            Err(error) if error.is_empty() => std::thread::sleep(Duration::from_millis(1)),
            Err(_) => panic!("插件事件通道已关闭"),
        }
    }
}

/// 面板状态行按行号取值，未写过的行返回 None。
fn status_line(manager: &PluginManager, line: usize) -> Option<String> {
    manager
        .status_lines(PLUGIN_ID)
        .and_then(|lines| lines.get(line).cloned())
        .filter(|text| !text.is_empty())
}

/// 冒充界面的旁观订阅者：只记录收到的批次，用来证明插件的写入也进了公共广播。
struct Recorder {
    origin: Origin,
    batches: RefCell<Vec<Vec<MutationReceipt>>>,
}

impl Recorder {
    fn new(origin: Origin) -> Rc<Self> {
        Rc::new(Self {
            origin,
            batches: RefCell::new(Vec::new()),
        })
    }

    fn calls(&self) -> usize {
        self.batches.borrow().len()
    }

    fn items(&self) -> Vec<(ItemId, i64, i64)> {
        self.batches
            .borrow()
            .iter()
            .flatten()
            .map(|receipt| {
                let aweft_project::MutationKind::EventTimeChanged(detail) = &receipt.kind;
                (detail.item, detail.start_ms, detail.end_ms)
            })
            .collect()
    }
}

impl MutationSubscriber for Recorder {
    fn origin(&self) -> Origin {
        self.origin.clone()
    }

    fn on_mutations(&self, _store: &mut ProjectStore, mutations: &[MutationReceipt]) {
        self.batches.borrow_mut().push(mutations.to_vec());
    }
}

fn record(recorder: Rc<Recorder>, store: &mut ProjectStore) {
    store.subscribe(recorder as SubscriberRef);
}

#[test]
fn directory_load_reads_commands_and_panel_from_guest() {
    let (manager, _store, _ids, _rx) = manager_with_events(&[(1000, 2000)]);
    assert_eq!(manager.plugins().len(), 1);
    let plugin = &manager.plugins()[0];
    assert_eq!(plugin.id, PLUGIN_ID);
    assert_eq!(plugin.title, "示例插件");
    // 命令标签来自 Guest 的描述 JSON，下标即命令编号。
    assert_eq!(
        plugin.commands,
        vec![
            "计算最早时间".to_string(),
            "全部后移 1s".to_string(),
            "全部前移 10s（延迟 10s）".to_string(),
        ]
    );
    // 声明了 time_changed 的插件在装载时就注册进了存储，之后不需要有人 pump。
    assert_eq!(manager.subscribed_count(), 1);
    // 面板片段单独编译一次，失败时能看到 Slint 的诊断（load_dir 只记日志）。
    let source =
        std::fs::read_to_string(plugin_dir().join("panel.slint")).expect("面板源码读取失败");
    let definition = aweft_plugins::compile_panel(&source).expect("面板编译失败");
    assert_eq!(definition.name(), "PluginPanel");
    // 装载路径同样拿到定义，宿主只按通用约定填 command-labels / status-lines。
    let panel = plugin.panel().expect("面板未编译");
    assert_eq!(panel.name(), "PluginPanel");
}

#[test]
fn unknown_plugin_dir_is_reported_not_panicked() {
    let mut store = ProjectStore::new(SavedProject::new());
    let mut manager = PluginManager::new();
    let (tx, _rx) = unbounded::<PluginEvent>();
    assert!(manager
        .load_dir(&PathBuf::from("/nonexistent/plugin"), &mut store, &tx)
        .is_err());
    // 目录扫描同理：根目录不存在时装载数为 0。
    assert_eq!(
        manager.load_root(&PathBuf::from("/nonexistent/root"), &mut store, &tx),
        0
    );
    assert!(manager.is_empty());
}

/// 搭一个临时插件目录：清单用给定文本，字节码照抄示例插件。
/// 除清单外一切相同，两边的装载结果差异就只能来自清单。
fn temp_plugin_dir(name: &str, manifest: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(name);
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).expect("临时插件目录创建失败");
    std::fs::write(dir.join(MANIFEST_FILE), manifest).expect("临时清单写入失败");
    std::fs::copy(plugin_dir().join("guest.wasm"), dir.join("guest.wasm"))
        .expect("Guest 字节码复制失败");
    dir
}

/// 只改 `project_write` 一项的清单文本。
fn manifest_granting(project_write: bool) -> String {
    format!(
        r#"{{"id":"tmp","name":"临时","version":"0.1.0","guest":"guest.wasm","panel":"panel.slint",
            "permissions":{{"network":false,"fs_read":false,"fs_write":false,"project_write":{project_write}}}}}"#
    )
}

#[test]
fn write_import_without_grant_is_rejected_at_load() {
    let dir = temp_plugin_dir("aweft_plugin_write_denied", &manifest_granting(false));
    let mut store = ProjectStore::new(SavedProject::new());
    let mut manager = PluginManager::new();
    let (tx, _rx) = unbounded::<PluginEvent>();
    // 示例插件的 Guest import 了 set_event_*_ms，即它确实要改真相。
    let err = manager
        .load_dir(&dir, &mut store, &tx)
        .expect_err("未申请 project_write 的写插件应被拒绝装载");
    // 报错要点名缺的那项权限，含糊的"实例化失败"帮不上插件作者。
    assert!(
        format!("{err:?}").contains("project_write"),
        "报错未指明缺的权限：{err:?}"
    );
    assert!(manager.is_empty());
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn same_guest_loads_once_project_write_is_granted() {
    let dir = temp_plugin_dir("aweft_plugin_write_granted", &manifest_granting(true));
    let mut store = ProjectStore::new(SavedProject::new());
    let mut manager = PluginManager::new();
    let (tx, _rx) = unbounded::<PluginEvent>();
    manager
        .load_dir(&dir, &mut store, &tx)
        .expect("授予 project_write 后同一份字节码应能装载");
    assert_eq!(manager.plugins().len(), 1);
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn command_writes_back_into_store_truth() {
    let (mut manager, mut store, ids, mut rx) =
        manager_with_events(&[(1000, 2000), (5000, 6000)]);

    let code = run_command_sync(&mut manager, &mut store, &mut rx, 1);
    assert_eq!(code, 0);
    // 命令的产出直接落在真相上，读回即是新时间。
    assert_eq!(
        store.event_times_ms(ids[0]).expect("读回失败"),
        (2000, 3000)
    );
    assert_eq!(
        store.event_times_ms(ids[1]).expect("读回失败"),
        (6000, 7000)
    );
    assert!(store.is_dirty());
    // 显示文本由插件自己写进面板状态行，宿主不拼装。
    assert_eq!(status_line(&manager, 1).as_deref(), Some("已平移 2 行 +1s"));
}

/// 命令 2 的端到端：点下后倒计时 10 秒（面板实时刷剩余），再前移全部行。
/// 跑满约 10 秒：这是 worker 架构要演示的核心场景（长任务不卡 + 执行中刷新），
/// 值得花这 10 秒。倒计时期间插件是忙的，结束时 busy 出清、真相落地。
#[test]
fn delayed_shift_counts_down_then_shifts() {
    let (mut manager, mut store, ids, mut rx) =
        manager_with_events(&[(30_000, 31_000), (40_000, 45_000)]);
    manager
        .run_command(&store, PLUGIN_ID, 2)
        .expect("命令分发失败");
    assert!(manager.is_busy(PLUGIN_ID));
    // 倒计时出现即证明"执行中刷新"活着：第一次 `now_ms` 读数后立刻发出，
    // 文案含"倒计时"三个字说明走的是倒计时分支而非陈旧状态行。
    drain_until(&mut manager, &mut store, &mut rx, |manager| {
        status_line(manager, 3)
            .as_deref()
            .is_some_and(|line| line.contains("倒计时"))
    });
    assert!(
        manager.is_busy(PLUGIN_ID),
        "倒计时期间命令尚未完成，应仍是忙"
    );
    // 等最终结果：两行各前移 10 秒，状态行换成完成文案。
    // 条件必须含 busy 出清：完成文案是 Progress 先到的，紧随其后的 Finished
    // 才清 busy——只等文案会停在 Finished 落盘之前。
    drain_until(&mut manager, &mut store, &mut rx, |manager| {
        status_line(manager, 3).as_deref() == Some("已前移 2 行 -10s")
            && !manager.is_busy(PLUGIN_ID)
    });
    assert_eq!(
        store.event_times_ms(ids[0]).expect("读回失败"),
        (20_000, 21_000)
    );
    assert_eq!(
        store.event_times_ms(ids[1]).expect("读回失败"),
        (30_000, 35_000)
    );
}

#[test]
fn guest_side_computation_reads_current_rows() {
    let (mut manager, mut store, _ids, mut rx) =
        manager_with_events(&[(3000, 4000), (1000, 2000), (5000, 6000)]);
    run_command_sync(&mut manager, &mut store, &mut rx, 0);
    assert_eq!(
        status_line(&manager, 0).as_deref(),
        Some("最早：00:00:01.000")
    );

    let first = store.ordered_event_ids()[0];
    store
        .set_event_start_ms(Origin::LocalGui, first, 100)
        .expect("写入失败");
    // 界面写入的通知 job 排在下一条命令 job 之前（同一通道 FIFO），
    // 且命令自带新快照，所以直接跑下一条命令结果也是新的，无需手动 pump。
    run_command_sync(&mut manager, &mut store, &mut rx, 0);
    assert_eq!(
        status_line(&manager, 0).as_deref(),
        Some("最早：00:00:00.100")
    );
}

#[test]
fn time_write_reaches_subscribed_plugin_via_events() {
    let (mut manager, mut store, ids, mut rx) =
        manager_with_events(&[(3000, 4000), (1000, 2000), (5000, 6000)]);
    assert!(status_line(&manager, 2).is_none(), "未写入前不应有通知");
    // 用户把最早的一行（1000-2000，句柄 0）的开始时间改成 1500。
    // 写入返回时 job 刚发出（不再是"返回前送达"）；pump 后 Guest 才处理完。
    store
        .set_event_start_ms(Origin::LocalGui, ids[1], 1500)
        .expect("写入失败");
    // 句柄 = ordered_event_ids 的下标，值 = 钳制后的成对时间。
    // 条件含日志行：Guest 先 set_status（第 0 个 Progress）再 log（第 1 个），
    // 只等状态行会停在日志到达之前——这正是异步事件流的语义。
    drain_until(&mut manager, &mut store, &mut rx, |manager| {
        status_line(manager, 2).as_deref() == Some("最近通知：第 0 行 00:00:01.500 ~ 00:00:02.000")
            && manager
                .plugin(PLUGIN_ID)
                .is_some_and(|plugin| !plugin.log_lines().is_empty())
    });
    // Guest 自己写的诊断行同样留在插件实例里，宿主只搬运不生成。
    assert_eq!(
        manager.plugin(PLUGIN_ID).expect("插件未加载").log_lines(),
        vec!["第 0 行时间已变更".to_string()]
    );
}

#[test]
fn plugin_own_writes_are_not_echoed_back_to_it() {
    let (mut manager, mut store, _ids, mut rx) = manager_with_events(&[(1000, 2000)]);
    // 命令 1 由插件自己写入两段时间：回执来源标记为插件，广播按来源过滤掉自己。
    run_command_sync(&mut manager, &mut store, &mut rx, 1);
    assert!(
        status_line(&manager, 2).is_none(),
        "插件自己的写入不应再回投给自己： {:?}",
        manager.status_lines(PLUGIN_ID)
    );

    // 同一行被界面改动后，订阅才生效。
    let row = store.ordered_event_ids()[0];
    store
        .adjust_event_time(Origin::LocalGui, row, true, TimeDelta::milliseconds(10))
        .expect("界面写入失败");
    drain_until(&mut manager, &mut store, &mut rx, |manager| {
        status_line(manager, 2).as_deref() == Some("最近通知：第 0 行 00:00:02.010 ~ 00:00:03.000")
    });
}

#[test]
fn plugin_writes_reach_other_subscribers_too() {
    // 通知不是插件专用的：界面（或以后的协作）注册成订阅者就能收到插件的产出。
    let (mut manager, mut store, ids, mut rx) =
        manager_with_events(&[(1000, 2000), (5000, 6000)]);
    let watcher = Recorder::new(Origin::LocalGui);
    record(Rc::clone(&watcher), &mut store);

    run_command_sync(&mut manager, &mut store, &mut rx, 1);
    // 一条命令改两行 = 一次广播：旁观者收到一个批次两条回执，来源都是插件。
    assert_eq!(watcher.calls(), 1, "一次命令应只通知一次");
    assert_eq!(
        watcher.items(),
        vec![(ids[0], 2000, 3000), (ids[1], 6000, 7000)],
        "插件写回应进入公共广播"
    );
    assert!(
        status_line(&manager, 2).is_none(),
        "插件不应收到自己写回的回声： {:?}",
        manager.status_lines(PLUGIN_ID)
    );
}

#[test]
fn undeclared_command_is_rejected() {
    let (mut manager, store, _ids, _rx) = manager_with_events(&[(1000, 2000)]);
    // 拒绝发生在分发时（worker 根本收不到 job），busy 也不应置位。
    assert!(manager.run_command(&store, PLUGIN_ID, 99).is_err());
    assert!(manager.run_command(&store, "no-such-plugin", 0).is_err());
    assert!(!manager.is_busy(PLUGIN_ID));
    assert!(!store.is_dirty(), "被拒绝的命令不应写入");
    assert_eq!(
        store
            .event_times_ms(store.ordered_event_ids()[0])
            .expect("读回失败"),
        (1000, 2000)
    );
}

#[test]
fn second_command_while_busy_is_rejected_not_queued() {
    let (mut manager, mut store, _ids, mut rx) = manager_with_events(&[(1000, 2000)]);
    manager
        .run_command(&store, PLUGIN_ID, 0)
        .expect("第一条命令分发失败");
    assert!(manager.is_busy(PLUGIN_ID));
    // 上一条没跑完：直接拒绝，不排队。面板侧由插件作者按 command-busy 置灰，
    // 宿主这里只是 backstop（老面板没接 busy 时漏进来的点击落到这里）。
    let err = manager
        .run_command(&store, PLUGIN_ID, 0)
        .expect_err("忙时第二条命令应被拒绝");
    assert!(
        format!("{err:?}").contains("正忙"),
        "报错应说明是忙而不是别的：{err:?}"
    );
    // pump 到出清后又能跑了。
    drain_until(&mut manager, &mut store, &mut rx, |manager| {
        !manager.is_busy(PLUGIN_ID)
    });
    manager
        .run_command(&store, PLUGIN_ID, 0)
        .expect("出清后应能再跑");
}

#[test]
fn progress_events_arrive_before_command_finishes() {
    let (mut manager, mut store, _ids, mut rx) = manager_with_events(&[(1000, 2000)]);
    manager
        .run_command(&store, PLUGIN_ID, 0)
        .expect("命令分发失败");
    // 命令 0 调了 set_status：Progress 应在 Finished 之前到达，
    // 这就是"执行中刷新"的通道（pump 里同步面板即可）。
    let mut saw_progress = false;
    let mut finished = false;
    while !finished {
        let (id, event) = next_event(&mut rx);
        assert_eq!(id, PLUGIN_ID);
        if let WorkerEvent::Progress { status_lines, .. } = &event {
            if status_lines.first().map(String::as_str) == Some("最早：00:00:01.000") {
                saw_progress = true;
            }
        }
        if matches!(event, WorkerEvent::CommandFinished { .. }) {
            finished = true;
        }
        manager
            .apply_event(&mut store, &id, event)
            .expect("事件落盘失败");
    }
    assert!(saw_progress, "Finished 之前应先收到 Progress");
}
