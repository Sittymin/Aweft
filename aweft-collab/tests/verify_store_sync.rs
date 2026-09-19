//! 设计验证 4b：本地写入经订阅者、引擎与网络，落到另一台电脑的 `ProjectStore`。
//!
//! 对应设计草稿「验证步骤」第 4 条与「总体结构」的数据流。两个节点同进程、
//! 各自的内存库与各自的存储，经发现层直连（与 `verify_topic_sync.rs` 同一前提）。
//! 断言三件事：值收敛、落盘来源是 `Origin::Collab`、越界凭证的写入被接收端丢弃。

use std::cell::RefCell;
use std::collections::VecDeque;
use std::path::PathBuf;
use std::rc::Rc;
use std::time::{Duration, Instant};

use aweft_collab::cap::{Member, Namespace, encode_capability, row_area, track_area};
use aweft_collab::engine::{CollabEngine, CollabEvent, Session};
use aweft_collab::subscriber;
use aweft_collab::wire::PendingEdit;
use aweft_model::ids::ItemId;
use aweft_model::sequence::{EventData, GroupData, ItemLocation};
use aweft_project::{MutationKind, MutationReceipt, Origin, ProjectStore, SavedProject};
use chrono::TimeDelta;
use futures::channel::mpsc;
use willow25::groupings::Area;

/// 同步收敛的等待上限。同一台机器上的发现与直连在秒级，余量留给慢机与负载。
const SETTLE: Duration = Duration::from_secs(60);

/// 观察「不该发生的事」需要一段静默期：等够这么久还没来，才算确认被丢弃。
const SILENCE: Duration = Duration::from_secs(5);

/// 网络类测试一次只跑一个：每个都要在同一进程里开两三个真节点，
/// 两个测试并行时抢的不是正确性而是发现与同步的带宽，`SETTLE` 会等不来收敛。
/// 单独跑都过——并行跑就会随机红，这种红没有任何诊断价值。
static NETWORK: std::sync::Mutex<()> = std::sync::Mutex::new(());

/// 上一个测试失败留下的毒锁不连带挡后面：这里要的是串行，不是互相惩罚。
fn network_lock() -> std::sync::MutexGuard<'static, ()> {
    NETWORK.lock().unwrap_or_else(|poisoned| poisoned.into_inner())
}

/// 每个测试各自的命名空间。命名空间即主题，两个测试共用一份就会互相收到对方的操作，
/// 「远端变更条数」这类断言会被同二进制里并行的另一个测试污染。
fn fresh_namespace() -> Namespace {
    Namespace::generate().expect("自有命名空间应当能滚出来")
}

/// 真机用的落盘库：`target/scratch/` 下按命名空间分目录（`/tmp` 是 tmpfs，且共用库文件会让测试互相投毒）。
fn scratch_db(namespace: &Namespace, who: &str) -> PathBuf {
    let dir = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .expect("crate 应当在工程目录下")
        .join("target/scratch/collab-db")
        .join(hex::encode(namespace.id_bytes()));
    std::fs::create_dir_all(&dir).expect("scratch 数据库目录应当能创建");
    dir.join(format!("{who}.sqlite"))
}

fn member(byte: u8) -> Member {
    Member::from_seed(&[byte; 32])
}

fn bytes_of(id: ItemId) -> Vec<u8> {
    id.0.as_bytes().to_vec()
}

/// 一份两行两轨的工程：`granted_*` 在转授范围内的轨道里，`other_*` 在范围外的那条。
struct Fixture {
    /// 工程快照字节：两台「电脑」各解一份，走的就是生产那条编解码路。
    bytes: Vec<u8>,
    granted_track: ItemId,
    granted_row: ItemId,
    /// 同一轨道内的第二行：让两个成员可以各写一行，不抢同一个坐标。
    granted_row2: ItemId,
    other_track: ItemId,
    other_row: ItemId,
}

fn fixture() -> Fixture {
    let mut store = ProjectStore::new(SavedProject::new());
    let data = || EventData {
        start_time: TimeDelta::milliseconds(0),
        end_time: TimeDelta::milliseconds(100),
        actor: None,
        text: String::new(),
        segment_timing: None,
        style: String::new(),
    };
    let mut new_track = |name: &str| {
        let group = store
            .sequence_mut()
            .create_group(
                GroupData {
                    name: name.into(),
                    collapsed: false,
                    color: None,
                    children: vec![],
                },
                ItemLocation::Root(0),
            )
            .expect("分组创建失败");
        let row = store
            .sequence_mut()
            .create_event(data(), ItemLocation::InGroup(group, 0))
            .expect("行创建失败");
        (group, row)
    };
    let (granted_track, granted_row) = new_track("范围内的轨道");
    let (other_track, other_row) = new_track("范围外的轨道");
    // 范围内的第二条行：场景测试里两个成员各写一行，免得为抢同一个坐标比谁的时间戳晚。
    let granted_row2 = store
        .sequence_mut()
        .create_event(data(), ItemLocation::InGroup(granted_track, 1))
        .expect("第二行创建失败");
    Fixture {
        bytes: store.encode_snapshot().expect("工程快照应当可序列化"),
        granted_track,
        granted_row,
        granted_row2,
        other_track,
        other_row,
    }
}

/// 只记账、不改数据的订阅者：用来观察落盘那一次写入的 `Origin`。
struct Recorder {
    origin: Origin,
    sink: Rc<RefCell<VecDeque<MutationReceipt>>>,
}

impl aweft_project::MutationSubscriber for Recorder {
    fn origin(&self) -> Origin {
        self.origin.clone()
    }

    fn on_mutations(&self, _store: &mut ProjectStore, mutations: &[MutationReceipt]) {
        self.sink.borrow_mut().extend(mutations.iter().cloned());
    }
}

/// 一端 = 「那台电脑」：一份存储、一个后台引擎、事件接收端、回执记录仪。
struct Computer {
    store: ProjectStore,
    engine: Rc<CollabEngine>,
    events: VecDeque<CollabEvent>,
    rx: mpsc::UnboundedReceiver<CollabEvent>,
    receipts: Rc<RefCell<VecDeque<MutationReceipt>>>,
    remote_count: usize,
    /// 本机现行凭证：被远端收紧后会换成新的一张。
    capability: Vec<u8>,
}

impl Computer {
    fn new(session: Session, fixture: &Fixture) -> Self {
        let capability = session.capability.clone();
        let (tx, rx) = mpsc::unbounded::<CollabEvent>();
        let engine = Rc::new(CollabEngine::spawn(session, tx).expect("协作引擎应当能起后台线程"));
        let mut store =
            ProjectStore::decode_snapshot(&fixture.bytes, "fixture").expect("工程快照应当可读回");
        subscriber::register(&mut store, Rc::clone(&engine));
        let receipts = Rc::new(RefCell::new(VecDeque::new()));
        store.subscribe(Rc::new(Recorder {
            origin: Origin::LocalCli,
            sink: Rc::clone(&receipts),
        }));
        Self {
            store,
            engine,
            events: VecDeque::new(),
            rx,
            receipts,
            remote_count: 0,
            capability,
        }
    }

    /// 收干事件通道并把远端变更落盘。跨睡眠不持有任何借用。
    fn drain(&mut self) {
        while let Ok(event) = self.rx.try_recv() {
            self.events.push_back(event);
        }
        let mut incoming = Vec::new();
        while let Some(event) = self.events.pop_front() {
            match event {
                CollabEvent::Remote(edit) => incoming.push(edit),
                CollabEvent::Ready { node_id } => log::info!("节点就绪 {node_id}"),
                CollabEvent::Peers { connected } => log::info!("对端数 {connected}"),
                CollabEvent::Snapshot { author, .. } => log::info!("收到 {author} 的工程快照"),
                CollabEvent::Capability { capability } => {
                    self.capability = hex::decode(capability).expect("通告里的凭证应当是十六进制")
                }
                CollabEvent::Failed { reason } => panic!("协作引擎失败：{reason}"),
            }
        }
        self.remote_count += incoming.len();
        for edit in incoming {
            subscriber::apply_remote(&mut self.store, &edit);
        }
    }

    fn times(&self, row: ItemId) -> (i64, i64) {
        self.store.event_times_ms(row).expect("行存在")
    }

    /// 各方交替推进，直到本端 `row` 等于 `target`。对端也要推进：
    /// 它落盘后若会再发回去（广播风暴），只有推进行为才暴露得出来。
    fn wait_for(&mut self, peers: &mut [&mut Computer], row: ItemId, target: (i64, i64)) {
        let deadline = Instant::now() + SETTLE;
        loop {
            self.drain();
            for peer in peers.iter_mut() {
                peer.drain();
            }
            if self.times(row) == target {
                return;
            }
            assert!(
                Instant::now() < deadline,
                "超时未收敛：行 {row} 期望 {target:?}，实为 {:?}",
                self.times(row)
            );
            std::thread::sleep(Duration::from_millis(50));
        }
    }

    /// 各方交替推进，直到本机现行凭证换成 `want`：收紧只有落到本机才算生效。
    fn wait_capability(&mut self, peers: &mut [&mut Computer], want: &[u8]) {
        let deadline = Instant::now() + SETTLE;
        loop {
            self.drain();
            for peer in peers.iter_mut() {
                peer.drain();
            }
            if self.capability == want {
                return;
            }
            assert!(
                Instant::now() < deadline,
                "超时未换成本机凭证：期望 {}…，实为 {}…",
                &hex::encode(want)[..16],
                &hex::encode(&self.capability)[..16]
            );
            std::thread::sleep(Duration::from_millis(50));
        }
    }

    /// 静默期：所有人都推进够 `SILENCE`，用来断言「没有东西过来」。
    fn settle_quietly(&mut self, peers: &mut [&mut Computer]) {
        let deadline = Instant::now() + SILENCE;
        while Instant::now() < deadline {
            self.drain();
            for peer in peers.iter_mut() {
                peer.drain();
            }
            std::thread::sleep(Duration::from_millis(50));
        }
    }
}

/// 全区域凭证的所有者会话。
fn owner_session(owner: &Member, namespace: &Namespace) -> Session {
    Session {
        member: Member::from_seed(owner.seed()),
        namespace: Namespace::from_id_bytes(&namespace.id_bytes()),
        capability: encode_capability(&namespace.owner_capability(owner).expect("创世凭证")),
        database: None,
        bootstrap: None,
        announcements: Vec::new(),
    }
}

/// 被转授者会话：只有命名空间标识，凭证覆盖 `track` 一条轨道。
fn editor_session(
    editor: &Member,
    namespace: &Namespace,
    owner_cap: &willow25::authorisation::WriteCapability,
    owner: &Member,
    track: ItemId,
) -> Session {
    let cap = namespace
        .delegate(owner_cap, owner, track_area(track), editor.id())
        .expect("轨道转授应当成功");
    Session {
        member: Member::from_seed(editor.seed()),
        namespace: Namespace::from_id_bytes(&namespace.id_bytes()),
        capability: encode_capability(&cap),
        database: None,
        bootstrap: None,
        announcements: Vec::new(),
    }
}

/// 会话的通用形状：凭证由调用方签好，库落盘到 `database`，本机签出的凭证照样补发通告。
/// 与 `config::session` 在一台真机上给出的字段一致，只是不绕配置文件那圈。
fn member_session(
    who: &Member,
    namespace: &Namespace,
    cap: &willow25::authorisation::WriteCapability,
    database: PathBuf,
    announcements: Vec<Vec<u8>>,
) -> Session {
    Session {
        member: Member::from_seed(who.seed()),
        namespace: Namespace::from_id_bytes(&namespace.id_bytes()),
        capability: encode_capability(cap),
        database: Some(database),
        bootstrap: None,
        announcements,
    }
}

#[test]
fn local_write_converges_on_the_peer_store() {
    let _serial = network_lock();
    let fixture = fixture();
    let owner = member(11);
    let editor = member(22);
    let namespace = fresh_namespace();
    let owner_cap = namespace.owner_capability(&owner).expect("创世凭证");

    let mut a = Computer::new(owner_session(&owner, &namespace), &fixture);
    let mut b = Computer::new(
        editor_session(
            &editor,
            &namespace,
            &owner_cap,
            &owner,
            fixture.granted_track,
        ),
        &fixture,
    );

    a.store
        .set_event_times_ms(Origin::LocalGui, fixture.granted_row, 4321, 5678)
        .expect("本地写入应当成功");

    b.wait_for(&mut [&mut a], fixture.granted_row, (4321, 5678));
    assert_eq!(a.times(fixture.granted_row), (4321, 5678));

    // 落盘那一次的来源必须标着写作者：界面与日志靠它分辨「谁改的」。
    let seen: Vec<MutationReceipt> = b.receipts.borrow().iter().cloned().collect();
    let applied = seen.iter().find(|receipt| {
        matches!(&receipt.kind, MutationKind::EventTimeChanged(detail)
            if detail.item == fixture.granted_row
                && (detail.start_ms, detail.end_ms) == (4321, 5678))
    });
    let Some(applied) = applied else {
        panic!("B 端应当看到这条时间变更的回执，实际记录 {seen:?}");
    };
    assert_eq!(applied.origin, Origin::Collab(owner.hex_id()));

    // 广播风暴检查：A 的变更在主题里只应当存在一条操作。
    // 若 B 落盘时又把它发回去，两端各签一条新操作互相追，计数只涨不停。
    b.settle_quietly(&mut [&mut a]);
    assert_eq!(b.remote_count, 1, "B 端收到的远端变更条数");
    assert!(
        a.remote_count <= 1,
        "A 端收到的远端变更条数：{}（本端回声至多一条）",
        a.remote_count
    );
}

#[test]
fn out_of_scope_write_is_dropped_by_the_receiver() {
    let _serial = network_lock();
    let fixture = fixture();
    let owner = member(31);
    let limited = member(42);
    let peer = member(52);
    let namespace = fresh_namespace();
    let owner_cap = namespace.owner_capability(&owner).expect("创世凭证");

    // 持「范围内轨道」凭证的成员冒充「范围外轨道」写入：路径字段由发送者自填，
    // 接收端不信这套说辞，只信凭证。
    let mut sender = Computer::new(
        editor_session(
            &limited,
            &namespace,
            &owner_cap,
            &owner,
            fixture.granted_track,
        ),
        &fixture,
    );
    let mut receiver = Computer::new(
        editor_session(
            &peer,
            &namespace,
            &owner_cap,
            &owner,
            fixture.granted_track,
        ),
        &fixture,
    );

    // 先放行内的那条：证明链路是活的，后面的丢弃不是因为压根没连上。
    sender
        .engine
        .publish(vec![PendingEdit {
            track: bytes_of(fixture.granted_track),
            row: bytes_of(fixture.granted_row),
            start_ms: 1111,
            end_ms: 2222,
        }])
        .expect("发送队列应当可用");
    receiver.wait_for(&mut [&mut sender], fixture.granted_row, (1111, 2222));

    // 再发越界的那条，然后一起静默：范围外的行必须原地不动。
    sender
        .engine
        .publish(vec![PendingEdit {
            track: bytes_of(fixture.other_track),
            row: bytes_of(fixture.other_row),
            start_ms: 9999,
            end_ms: 10_000,
        }])
        .expect("发送队列应当可用");
    receiver.settle_quietly(&mut [&mut sender]);

    assert_eq!(
        receiver.times(fixture.other_row),
        (0, 100),
        "越界凭证的写入不该落盘"
    );
    assert_eq!(receiver.times(fixture.granted_row), (1111, 2222));
}

/// 设计验证 5 的进程内替身：真机用的落盘库 + 重启。
///
/// 覆盖两件事，都是内存库测不出来的：
/// 1. `database_url` 指向文件时节点能起、能收；
/// 2. 已确认过的操作在重启后不重放（`StreamFrom::Frontier` 读的是库里的游标），
///    而重启期间产生的新操作仍能补齐。
#[test]
fn restart_replays_nothing_and_still_catches_up() {
    let _serial = network_lock();
    let fixture = fixture();
    let owner = member(61);
    let editor = member(72);
    let namespace = fresh_namespace();
    let owner_cap = namespace.owner_capability(&owner).expect("创世凭证");
    let owner_db = scratch_db(&namespace, "owner");
    let editor_db = scratch_db(&namespace, "editor");

    let mut owner_session = owner_session(&owner, &namespace);
    owner_session.database = Some(owner_db);
    let mut first_session = editor_session(
        &editor,
        &namespace,
        &owner_cap,
        &owner,
        fixture.granted_track,
    );
    first_session.database = Some(editor_db.clone());

    let mut a = Computer::new(owner_session, &fixture);
    let mut b = Computer::new(first_session, &fixture);
    a.store
        .set_event_times_ms(Origin::LocalGui, fixture.granted_row, 4321, 5678)
        .expect("本地写入应当成功");
    b.wait_for(&mut [&mut a], fixture.granted_row, (4321, 5678));

    // B 关机：引擎线程 join 完才会释放库文件，这一步同时验了退出是干净的。
    drop(b);

    // 同一份库、全新的一份工程文档重启回来。
    let mut second_session = editor_session(
        &editor,
        &namespace,
        &owner_cap,
        &owner,
        fixture.granted_track,
    );
    second_session.database = Some(editor_db);
    let mut b2 = Computer::new(second_session, &fixture);
    b2.settle_quietly(&mut [&mut a]);
    assert_eq!(
        b2.times(fixture.granted_row),
        (0, 100),
        "已确认过的操作不该在重启后重放：重放会把新值压回旧值"
    );
    assert_eq!(b2.remote_count, 0, "重启后收到的重放条数");

    // 重启期间 A 的写入必须照样追得上——游标只跳过已确认的，不跳过没见过的。
    a.store
        .set_event_times_ms(Origin::LocalGui, fixture.granted_row, 7777, 8888)
        .expect("本地写入应当成功");
    b2.wait_for(&mut [&mut a], fixture.granted_row, (7777, 8888));
    assert_eq!(b2.remote_count, 1, "重启后收到的未确认变更条数");
}

/// 三机场景：所有者给 Bob 根级权限后下线，Bob 改数据并把字幕轨转授给 Carol，
/// 所有者上线同步后把 Bob 收紧到仅字幕轨。此后 Bob 在范围外的写入、Carol 凭
/// Bob 旧凭证派生的那张写入，都不许落到别人的机器上。
///
/// 三台机器同进程，各自的库与名册文件都在 `target/scratch/` 下。同一个人不能同时
/// 开两份库（同一作者的日志会分叉），所以要换凭证时都是先下线再重开。
#[test]
fn offline_edits_then_tightening_revokes_bob_and_his_downstream() {
    let _serial = network_lock();
    let fixture = fixture();
    let (owner, bob, carol) = (member(81), member(82), member(83));
    let namespace = fresh_namespace();
    let owner_cap = namespace.owner_capability(&owner).expect("创世凭证");
    let bob_wide = namespace
        .delegate(&owner_cap, &owner, Area::full(), bob.id())
        .expect("根级转授");
    let carol_from_bob = namespace
        .delegate(
            &bob_wide,
            &bob,
            track_area(fixture.granted_track),
            carol.id(),
        )
        .expect("Bob 向下转授");
    let bob_narrow = namespace
        .delegate(
            &owner_cap,
            &owner,
            track_area(fixture.granted_track),
            bob.id(),
        )
        .expect("收紧签发");
    let (owner_db, bob_db, carol_db) = (
        scratch_db(&namespace, "creator"),
        scratch_db(&namespace, "bob"),
        scratch_db(&namespace, "carol"),
    );

    // 一、所有者启用协作、把 Bob 签进工程，随即下线。
    let creator = Computer::new(
        member_session(
            &owner,
            &namespace,
            &owner_cap,
            owner_db.clone(),
            vec![encode_capability(&bob_wide)],
        ),
        &fixture,
    );
    creator
        .engine
        .announce(encode_capability(&bob_wide))
        .expect("授予通告应当进队");
    let mut bob_machine = Computer::new(
        member_session(&bob, &namespace, &bob_wide, bob_db.clone(), Vec::new()),
        &fixture,
    );
    drop(creator);

    // 二、所有者不在期间：Bob 改数据，并把字幕轨转授给刚上线的 Carol。
    bob_machine
        .store
        .set_event_times_ms(Origin::LocalGui, fixture.granted_row, 1111, 2222)
        .expect("Bob 本地写入");
    let mut carol_machine = Computer::new(
        member_session(&carol, &namespace, &carol_from_bob, carol_db, Vec::new()),
        &fixture,
    );
    bob_machine
        .engine
        .announce(encode_capability(&carol_from_bob))
        .expect("转授通告应当进队");
    carol_machine.wait_for(
        &mut [&mut bob_machine],
        fixture.granted_row,
        (1111, 2222),
    );
    carol_machine
        .store
        .set_event_times_ms(Origin::LocalGui, fixture.granted_row2, 3333, 4444)
        .expect("Carol 本地写入");
    bob_machine.wait_for(
        &mut [&mut carol_machine],
        fixture.granted_row2,
        (3333, 4444),
    );

    // 三、所有者上线：离线期间两人的改动都要追平。
    let mut creator = Computer::new(
        member_session(
            &owner,
            &namespace,
            &owner_cap,
            owner_db,
            vec![encode_capability(&bob_wide)],
        ),
        &fixture,
    );
    creator.wait_for(
        &mut [&mut bob_machine, &mut carol_machine],
        fixture.granted_row,
        (1111, 2222),
    );
    creator.wait_for(
        &mut [&mut bob_machine, &mut carol_machine],
        fixture.granted_row2,
        (3333, 4444),
    );

    // 四、所有者把 Bob 收紧到仅字幕轨：通告要一路走到 Bob 自己那侧才算生效。
    creator
        .engine
        .announce(encode_capability(&bob_narrow))
        .expect("收紧通告应当进队");
    bob_machine.wait_capability(
        &mut [&mut creator, &mut carol_machine],
        &encode_capability(&bob_narrow),
    );

    // 收紧后范围内的写入照旧收敛，范围外的那条只能留在他自己机器上。
    bob_machine
        .store
        .set_event_times_ms(Origin::LocalGui, fixture.granted_row, 5555, 6666)
        .expect("Bob 范围内写入");
    creator.wait_for(
        &mut [&mut bob_machine, &mut carol_machine],
        fixture.granted_row,
        (5555, 6666),
    );
    bob_machine
        .store
        .set_event_times_ms(Origin::LocalGui, fixture.other_row, 9999, 10_000)
        .expect("Bob 越界写入：本机不拦，交给接收端判");
    // Carol 那张是 Bob 旧凭证派生的，一并作废：她的权限不可能比 Bob 更宽。
    carol_machine
        .store
        .set_event_times_ms(Origin::LocalGui, fixture.granted_row2, 7777, 8888)
        .expect("Carol 写入");
    creator.settle_quietly(&mut [&mut bob_machine, &mut carol_machine]);

    assert_eq!(
        creator.times(fixture.other_row),
        (0, 100),
        "Bob 收紧后的越界写入不该落到所有者机器上"
    );
    assert_eq!(
        carol_machine.times(fixture.other_row),
        (0, 100),
        "同样的写入也不该落到 Carol 机器上"
    );
    assert_eq!(
        creator.times(fixture.granted_row2),
        (3333, 4444),
        "Carol 的凭证已被连坐作废，她之后的写入不该过来"
    );
    assert_eq!(bob_machine.times(fixture.other_row), (9999, 10_000));

    // 五、拿旧凭证上岗的 Bob：只靠名册那一层挡——他的链仍然合法，范围也覆盖这一行。
    drop(bob_machine);
    let mut stale_bob = Computer::new(
        member_session(&bob, &namespace, &bob_wide, bob_db.clone(), Vec::new()),
        &fixture,
    );
    stale_bob
        .store
        .set_event_times_ms(Origin::LocalGui, fixture.granted_row2, 4242, 4343)
        .expect("旧凭证的写入");
    creator.settle_quietly(&mut [&mut stale_bob, &mut carol_machine]);
    assert_eq!(
        creator.times(fixture.granted_row2),
        (3333, 4444),
        "过期凭证的写入即便范围合法也不该被接受"
    );
    assert_ne!(
        carol_machine.times(fixture.granted_row2),
        (4242, 4343),
        "Carol 那侧也不该收下旧凭证的写入（她自己的越权写入另算）"
    );

    // 六、被撤的下游要重新拿权限，只能凭 Bob 现行那张重签。
    // 重签发生在换掉凭证记录之后：名册里本机那一条以记录里那张为准，
    // 还拿着宽凭证的那台机器凭不了窄链，正是第五幕要的效果。
    let revived = namespace
        .delegate(
            &bob_narrow,
            &bob,
            row_area(fixture.granted_track, fixture.granted_row2),
            carol.id(),
        )
        .expect("从窄凭证重签");
    drop(stale_bob);
    let mut bob_machine = Computer::new(
        member_session(&bob, &namespace, &bob_narrow, bob_db, Vec::new()),
        &fixture,
    );
    bob_machine
        .engine
        .announce(encode_capability(&revived))
        .expect("重签的通告应当进队");
    carol_machine.wait_capability(
        &mut [&mut creator, &mut bob_machine],
        &encode_capability(&revived),
    );
    carol_machine
        .store
        .set_event_times_ms(Origin::LocalGui, fixture.granted_row2, 6161, 6262)
        .expect("复活后的 Carol 写入");
    creator.wait_for(
        &mut [&mut bob_machine, &mut carol_machine],
        fixture.granted_row2,
        (6161, 6262),
    );
}
