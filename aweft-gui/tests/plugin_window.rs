//! 插件闭环的无头集成测试：装载真实插件目录 → 界面写入经广播直达 Guest →
//! 命令写回真相 → 广播回界面刷进行模型。主程序侧不出现任何插件专属的文本或功能硬编码。

use std::cell::RefCell;
use std::rc::Rc;

use aweft_gui::actions::sequence::SequenceAction;
use aweft_gui::actions::Action;
use aweft_gui::bridge::types::MainWindow;
use aweft_gui::store::AppStore;
use aweft_model::sequence::{EventData, ItemContent, ItemLocation};
use slint::{Model, ModelRc};

fn event_data(start_ms: i64, end_ms: i64) -> EventData {
    EventData {
        start_time: chrono::TimeDelta::try_milliseconds(start_ms).unwrap(),
        end_time: chrono::TimeDelta::try_milliseconds(end_ms).unwrap(),
        actor: None,
        text: String::new(),
        segment_timing: None,
        style: "Default".into(),
    }
}

fn setup() -> (MainWindow, Rc<RefCell<AppStore>>) {
    i_slint_backend_testing::init_no_event_loop();
    let instance = MainWindow::new().unwrap();
    let store = Rc::new(RefCell::new(AppStore::new(&instance)));
    {
        let mut store_mut = store.borrow_mut();
        let sequence = store_mut.state_mut().project.sequence_mut();
        sequence
            .create_event(event_data(3000, 4000), ItemLocation::Root(0))
            .unwrap();
        sequence
            .create_event(event_data(1000, 2000), ItemLocation::Root(1))
            .unwrap();
        store_mut.rebuild_sequence_models();
    }
    (instance, store)
}

/// 开始时间等于 `start_ms` 的那一行。
fn find_event_starting_at(store: &AppStore, start_ms: i64) -> aweft_model::ids::ItemId {
    store
        .state()
        .project
        .sequence()
        .items
        .iter()
        .find_map(|(id, item)| match &item.content {
            ItemContent::Event(event)
                if event.start_time == chrono::TimeDelta::try_milliseconds(start_ms).unwrap() =>
            {
                Some(*id)
            }
            _ => None,
        })
        .unwrap()
}

fn event_starts(store: &AppStore) -> Vec<i64> {
    let mut starts: Vec<i64> = store
        .state()
        .project
        .sequence()
        .items
        .values()
        .filter_map(|item| match &item.content {
            ItemContent::Event(event) => Some(event.start_time.num_milliseconds()),
            _ => None,
        })
        .collect();
    starts.sort();
    starts
}

fn status_line(store: &AppStore, line: usize) -> String {
    store
        .plugin_manager()
        .status_lines("test-plugin")
        .unwrap_or_default()
        .into_iter()
        .nth(line)
        .unwrap_or_default()
}

/// 未过滤的映射行模型（Rows → Map）：投影刷新后的显示值。
fn all_rows(store: &AppStore) -> ModelRc<aweft_gui::bridge::types::SlintSubtitleRow> {
    let rows_model: Rc<aweft_gui::store::sequence_rows_model::SequenceRowsModel> =
        Rc::new(store.ui_models().sequence_state.rows_model().clone());
    let mapper = rows_model.mapper();
    let mapped = slint::MapModel::new(rows_model, move |data| mapper.map(&data));
    ModelRc::new(mapped)
}

/// 投影出的各行开始时间（排序后），用来验证刷新真的走到了 UI 模型。
fn projected_starts(store: &AppStore) -> Vec<String> {
    let rows = all_rows(store);
    let mut starts: Vec<String> = (0..rows.row_count())
        .filter_map(|index| rows.row_data(index))
        .filter(|row| row.row_type != aweft_gui::bridge::types::SlintSubtitleRowType::GroupHeader)
        .map(|row| row.start_time.to_string())
        .collect();
    starts.sort();
    starts
}

#[test]
fn plugins_load_from_directory_with_guest_described_commands() {
    let (_instance, store) = setup();
    let borrowed = store.borrow();
    let plugin = borrowed
        .plugin_manager()
        .plugins()
        .iter()
        .find(|plugin| plugin.id == "test-plugin")
        .expect("插件目录里的示例插件未装载");
    // 命令标签来自 Guest 自描述，主程序不硬编码。
    assert_eq!(plugin.title, "示例插件");
    assert_eq!(plugin.commands, ["计算最早时间", "全部后移 1s"]);
    assert!(plugin.panel().is_some(), "插件自带面板应可用");
}

#[test]
fn gui_time_edit_reaches_subscribed_plugin() {
    let (_instance, store) = setup();
    // 句柄按开始时间排序：1000–2000 那一行是第 0 行。
    let target = find_event_starting_at(&store.borrow(), 1000);
    {
        let mut store_mut = store.borrow_mut();
        store_mut
            .dispatch(Action::Sequence(SequenceAction::InputTimecode {
                event_id: target,
                text: "0:00:01.50".into(),
                is_start_time: true,
            }))
            .unwrap();
        store_mut.evaluate_depsgraph();
    }
    // 无需任何 GUI 侧通知代码：写入返回前存储就把回执广播给了订阅的插件。
    assert_eq!(
        status_line(&store.borrow(), 2),
        "最近通知：第 0 行 00:00:01.500 ~ 00:00:02.000"
    );
}

#[test]
fn command_writes_through_the_store_and_refreshes_rows() {
    let (_instance, store) = setup();
    store
        .borrow_mut()
        .run_plugin_command("test-plugin", 1)
        .expect("命令执行失败");
    // 命令写的是同一个真相存储：全部行 +1s。
    assert_eq!(event_starts(&store.borrow()), vec![2000, 4000]);
    // 写入经广播进了界面的刷新队列，投影出的行也跟着变了（不是只改了数据）。
    assert_eq!(
        projected_starts(&store.borrow()),
        vec!["0:00:02.00", "0:00:04.00"],
        "插件写入应刷进 UI 行模型"
    );
    assert_eq!(status_line(&store.borrow(), 1), "已平移 2 行 +1s");
    // 插件自己的写入不回投给自己：状态行 2 仍是空。
    assert_eq!(status_line(&store.borrow(), 2), "");
}

#[test]
fn guest_side_computation_reads_current_rows() {
    let (_instance, store) = setup();
    store
        .borrow_mut()
        .run_plugin_command("test-plugin", 0)
        .expect("命令执行失败");
    assert_eq!(status_line(&store.borrow(), 0), "最早：00:00:01.000");
}

#[test]
fn undeclared_command_is_rejected() {
    let (_instance, store) = setup();
    let error = store
        .borrow_mut()
        .run_plugin_command("test-plugin", 99)
        .expect_err("未声明的命令编号应被拒绝");
    assert!(format!("{error:?}").contains("未声明命令"));
    assert_eq!(event_starts(&store.borrow()), vec![1000, 3000]);
}

#[test]
fn one_window_per_plugin_reused_and_unknown_rejected() {
    let (_instance, store) = setup();
    // 无头后端下 show 可能失败，关键是不 panic 且重复打开不新增窗口。
    aweft_gui::plugin_bridge::open_plugin_window(&store, "test-plugin");
    let first = store.borrow().open_window_count();
    aweft_gui::plugin_bridge::open_plugin_window(&store, "test-plugin");
    assert_eq!(store.borrow().open_window_count(), first);
    aweft_gui::plugin_bridge::open_plugin_window(&store, "no-such-plugin");
    assert_eq!(store.borrow().open_window_count(), first);
    // 越界下标只记日志。
    aweft_gui::plugin_bridge::open_plugin_window_at(&store, 99);
    assert_eq!(store.borrow().open_window_count(), first);
}
