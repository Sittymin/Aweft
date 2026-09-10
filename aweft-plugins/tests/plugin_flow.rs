//! 插件闭环测试：目录装载 → Guest 自描述 → 写日志投递 → 命令执行写回落真相。
//!
//! 被测插件是仓库里的真实插件目录 `plugins/test-plugin`（清单 + 已提交的
//! `guest.wasm` + `panel.slint`），不是为测试另写的夹具：跑通它即跑通示例。
//! Rust 源码在 `plugins/test-plugin/guest`，用 `plugins/test-plugin/build.sh` 重编。

use std::path::PathBuf;

use aweft_model::ids::ItemId;
use aweft_model::sequence::{EventData, ItemLocation};
use aweft_plugins::PluginManager;
use aweft_project::{AweftDocument, Origin, ProjectStore};
use chrono::TimeDelta;

const PLUGIN_ID: &str = "test-plugin";

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
    let mut store = ProjectStore::new(AweftDocument::new());
    let mut ids = Vec::new();
    for (index, (start, end)) in times.iter().enumerate() {
        let id = store
            .document_mut()
            .project
            .sequence_mut()
            .create_event(event_data(*start, *end), ItemLocation::Root(index))
            .expect("测试事件行创建失败");
        ids.push(id);
    }
    (store, ids)
}

/// 装载真实插件目录并建好行。返回的写日志已清空（播种不算变更）。
fn manager_with_events(times: &[(i64, i64)]) -> (PluginManager, ProjectStore, Vec<ItemId>) {
    let (mut store, ids) = store_with_events(times);
    let mut manager = PluginManager::new();
    manager
        .load_dir(&plugin_dir())
        .expect("插件目录装载失败");
    let _ = store.take_pending();
    (manager, store, ids)
}

/// 面板状态行按行号取值，未写过的行返回 None。
fn status_line(manager: &PluginManager, line: usize) -> Option<String> {
    manager
        .status_lines(PLUGIN_ID)
        .and_then(|lines| lines.get(line).cloned())
        .filter(|text| !text.is_empty())
}

#[test]
fn directory_load_reads_commands_and_panel_from_guest() {
    let (manager, _store, _ids) = manager_with_events(&[(1000, 2000)]);
    assert_eq!(manager.plugins().len(), 1);
    let plugin = &manager.plugins()[0];
    assert_eq!(plugin.id, PLUGIN_ID);
    assert_eq!(plugin.title, "示例插件");
    // 命令标签来自 Guest 的描述 JSON，下标即命令编号。
    assert_eq!(
        plugin.commands,
        vec!["计算最早时间".to_string(), "全部后移 1s".to_string()]
    );
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
    let mut manager = PluginManager::new();
    assert!(manager.load_dir(&PathBuf::from("/nonexistent/plugin")).is_err());
    // 目录扫描同理：根目录不存在时装载数为 0。
    assert_eq!(manager.load_root(&PathBuf::from("/nonexistent/root")), 0);
    assert!(manager.is_empty());
}

#[test]
fn command_writes_back_into_store_and_reports_receipts() {
    let (mut manager, mut store, ids) = manager_with_events(&[(1000, 2000), (5000, 6000)]);

    let (code, receipts) = manager
        .run_command(&mut store, PLUGIN_ID, 1)
        .expect("命令执行失败");
    assert_eq!(code, 0);
    // 两行各一条回执，来源是插件。
    assert_eq!(receipts.len(), 2);
    for receipt in &receipts {
        assert_eq!(receipt.origin, Origin::Plugin(PLUGIN_ID.to_string()));
    }
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

#[test]
fn guest_side_computation_reads_current_rows() {
    let (mut manager, mut store, _ids) =
        manager_with_events(&[(3000, 4000), (1000, 2000), (5000, 6000)]);
    manager
        .run_command(&mut store, PLUGIN_ID, 0)
        .expect("命令执行失败");
    assert_eq!(
        status_line(&manager, 0).as_deref(),
        Some("最早：00:00:01.000")
    );

    let first = store.ordered_event_ids()[0];
    store
        .set_event_start_ms(Origin::LocalGui, first, 100)
        .expect("写入失败");
    manager.deliver_pending(&mut store);
    manager
        .run_command(&mut store, PLUGIN_ID, 0)
        .expect("命令执行失败");
    assert_eq!(
        status_line(&manager, 0).as_deref(),
        Some("最早：00:00:00.100")
    );
}

#[test]
fn time_write_reaches_subscribed_plugin_with_row_and_values() {
    let (mut manager, mut store, ids) =
        manager_with_events(&[(3000, 4000), (1000, 2000), (5000, 6000)]);

    // 用户把最早的一行（1000-2000，句柄 0）的开始时间改成 1500。
    store
        .set_event_start_ms(Origin::LocalGui, ids[1], 1500)
        .expect("写入失败");
    assert!(status_line(&manager, 2).is_none(), "未通知前不应有状态行");
    let delivered = manager.deliver_pending(&mut store);
    assert_eq!(delivered.len(), 1, "一次界面写入投递一条回执");

    // 句柄 = ordered_event_ids 的下标，值 = 钳制后的成对时间。
    assert_eq!(
        status_line(&manager, 2).as_deref(),
        Some("最近通知：第 0 行 00:00:01.500 ~ 00:00:02.000")
    );
    // Guest 自己写的诊断行同样留在插件实例里，宿主只搬运不生成。
    assert_eq!(
        manager
            .plugin(PLUGIN_ID)
            .expect("插件未加载")
            .log_lines()
            .to_vec(),
        vec!["第 0 行时间已变更".to_string()]
    );
}

#[test]
fn plugin_own_writes_are_not_echoed_back_to_it() {
    let (mut manager, mut store, _ids) = manager_with_events(&[(1000, 2000)]);
    // 命令 1 由插件自己写入两段时间：写日志来源标记为插件，投递时过滤掉自己。
    manager
        .run_command(&mut store, PLUGIN_ID, 1)
        .expect("命令执行失败");
    assert!(
        status_line(&manager, 2).is_none(),
        "插件自己的写入不应再回投给自己： {:?}",
        manager.status_lines(PLUGIN_ID)
    );

    // 同一行被界面改动后，订阅才生效。
    let row = store.ordered_event_ids()[0];
    store
        .adjust_event_time(
            Origin::LocalGui,
            row,
            true,
            TimeDelta::milliseconds(10),
        )
        .expect("界面写入失败");
    manager.deliver_pending(&mut store);
    assert_eq!(
        status_line(&manager, 2).as_deref(),
        Some("最近通知：第 0 行 00:00:02.010 ~ 00:00:03.000")
    );
}

#[test]
fn undeclared_command_is_rejected() {
    let (mut manager, mut store, _ids) = manager_with_events(&[(1000, 2000)]);
    assert!(manager.run_command(&mut store, PLUGIN_ID, 99).is_err());
    assert!(manager
        .run_command(&mut store, "no-such-plugin", 0)
        .is_err());
    assert!(store.take_pending().is_empty(), "被拒绝的命令不应写入");
}
