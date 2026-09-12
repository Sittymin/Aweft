//! 插件闭环的无头集成测试：装载真实插件目录 → 界面写入经广播直达 Guest →
//! 命令写回真相 → 广播回界面刷进行模型 → 面板作为 Area 内容嵌进界面树。
//! 主程序侧不出现任何插件专属的文本或功能硬编码。

use std::cell::RefCell;
use std::rc::Rc;

use aweft_gui::actions::area::AreaAction;
use aweft_gui::actions::screen::ScreenAction;
use aweft_gui::actions::sequence::SequenceAction;
use aweft_gui::actions::Action;
use aweft_gui::bridge::slint_area_key_to_key;
use aweft_gui::bridge::types::{MainWindow, SlintAreaType};
use aweft_gui::screen::area::{AreaType, ScrAreaKey};
use aweft_gui::screen::primitives::Insets;
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

/// 建好工程数据并把屏幕布局初始化成默认的三块 Area（面板要嵌进 Area，先得有 Area）。
fn setup_with_areas() -> (MainWindow, Rc<RefCell<AppStore>>, ScrAreaKey) {
    let (instance, store) = setup();
    {
        let mut store_mut = store.borrow_mut();
        store_mut
            .dispatch(Action::Screen(ScreenAction::InitializeLayout {
                width: 1200,
                height: 800,
                scale_factor: 1.0,
                inset: Insets::default(),
            }))
            .unwrap();
        store_mut.evaluate_depsgraph();
    }
    let first = area_keys(&store.borrow())[0];
    // 模型绑定：`AppStore::new` 只填 Rust 侧的 VecModel，把它交给窗口是
    // `window_manager` 在真实启动路径里做的（无头测试不走那里）。不绑就只有所谓
    // 的"模型行改了"，界面树里一个 Area 都没有，容器也就不会去向工厂索取面板。
    instance.set_scr_areas(slint::ModelRc::from(store.borrow().ui_models().scrareas()));
    (instance, store, first)
}

fn area_keys(store: &AppStore) -> Vec<ScrAreaKey> {
    store.state().screen_map.areas.keys().collect()
}

/// 切 Area 类型并跑到投影：`dispatch` 只打 tag，实例的建与销都在这一步发生。
fn switch_area(store: &Rc<RefCell<AppStore>>, area_key: ScrAreaKey, area_type: AreaType) {
    {
        let mut store_mut = store.borrow_mut();
        store_mut
            .dispatch(Action::Area(AreaAction::SwitchType {
                area_key,
                area_type,
            }))
            .unwrap();
        store_mut.evaluate_depsgraph();
    }
    // 无头测试没有渲染循环，而 repeater / 条件元素 / ComponentContainer 是懒实例化的
    //（工厂只在 Slint 的实例化遍历里被调用）。推一次 mock 时间把这一遍跑掉，
    // 否则模型行改了、界面树里却什么也没建。
    instantiate_ui();
}

fn instantiate_ui() {
    i_slint_backend_testing::mock_elapsed_time(std::time::Duration::from_millis(0));
}

fn scrarea_row(store: &AppStore, area_key: ScrAreaKey) -> aweft_gui::bridge::types::ScrAreaData {
    let areas = store.ui_models().scrareas();
    (0..areas.row_count())
        .filter_map(|i| areas.row_data(i))
        .find(|row| slint_area_key_to_key::<ScrAreaKey>(&row.area_key) == area_key)
        .expect("Area 应在 UI 模型里有一行")
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

/// pump 到条件成立（带超时）：worker 回事件要几毫秒，单次 drain 可能扑空。
/// 真机上是 `spawn_local` 的 pump 事件驱动等待，这里用轮询等价。
fn drain_until(store: &Rc<RefCell<AppStore>>, mut cond: impl FnMut(&AppStore) -> bool) {
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(10);
    loop {
        if cond(&store.borrow()) {
            return;
        }
        if std::time::Instant::now() > deadline {
            panic!("等待插件事件超时");
        }
        store.borrow_mut().drain_plugin_events();
        std::thread::sleep(std::time::Duration::from_millis(1));
    }
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
    assert_eq!(
        plugin.commands,
        ["计算最早时间", "全部后移 1s", "全部前移 10s（延迟 10s）"]
    );
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
    // 无事件循环时手动 pump（真机上是 spawn_local 的 pump 在做同一件事）：
    // 写入返回时通知 job 刚发出，drain 后 Guest 才处理完。
    drain_until(&store, |store| {
        status_line(store, 2) == "最近通知：第 0 行 00:00:01.500 ~ 00:00:02.000"
    });
}

#[test]
fn command_writes_through_the_store_and_refreshes_rows() {
    let (_instance, store) = setup();
    // 分发只发送不等跑完，drain 后 worker 的产出才落真相。
    store
        .borrow_mut()
        .run_plugin_command("test-plugin", 1)
        .expect("命令分发失败");
    drain_until(&store, |store| {
        event_starts(store) == vec![2000, 4000]
    });
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
        .expect("命令分发失败");
    drain_until(&store, |store| {
        status_line(store, 0) == "最早：00:00:01.000"
    });
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
fn second_command_while_busy_is_rejected() {
    let (_instance, store) = setup();
    store
        .borrow_mut()
        .run_plugin_command("test-plugin", 0)
        .expect("第一条命令分发失败");
    // 上一条没跑完就再点：宿主 backstop 直接拒绝（面板侧由插件作者置灰）。
    // busy 标志是分发时同步置位的，不需要等 worker。
    let error = store
        .borrow_mut()
        .run_plugin_command("test-plugin", 0)
        .expect_err("忙时第二条命令应被拒绝");
    assert!(format!("{error:?}").contains("正忙"));
    drain_until(&store, |store| {
        !store.plugin_manager().is_busy("test-plugin")
    });
    // 出清后又能跑了。
    store
        .borrow_mut()
        .run_plugin_command("test-plugin", 0)
        .expect("出清后应能再跑");
}

#[test]
fn panel_lives_in_the_area_not_in_a_separate_window() {
    let (_instance, store, area_key) = setup_with_areas();
    let second_area = area_keys(&store.borrow())[1];

    // 切成插件 Area：这一行带上 id 与非空工厂，容器当场嵌出一个面板实例。
    switch_area(&store, area_key, AreaType::Plugin("test-plugin".into()));
    let row = scrarea_row(&store.borrow(), area_key);
    assert_eq!(row.area_type, SlintAreaType::Plugin);
    assert_eq!(row.plugin_id, "test-plugin");
    assert_ne!(row.plugin_factory, slint::ComponentFactory::default());
    assert_eq!(store.borrow().panel_instance_count("test-plugin"), 1);

    // 再切一次同一插件：工厂克隆后指针相等，Slint 不重建容器里已嵌好的面板。
    switch_area(&store, area_key, AreaType::Plugin("test-plugin".into()));
    assert_eq!(store.borrow().panel_instance_count("test-plugin"), 1);

    // 第二个 Area 也指过来（分屏复制同一插件的情形）：一处一个实例。
    switch_area(&store, second_area, AreaType::Plugin("test-plugin".into()));
    assert_eq!(store.borrow().panel_instance_count("test-plugin"), 2);

    // 都切走：容器销毁即实例释放，插件 id 与工厂跟着清空。
    switch_area(&store, area_key, AreaType::LineEditor);
    switch_area(&store, second_area, AreaType::LineEditor);
    assert_eq!(store.borrow().panel_instance_count("test-plugin"), 0);
    let row = scrarea_row(&store.borrow(), area_key);
    assert_eq!(row.area_type, SlintAreaType::LineEditor);
    assert_eq!(row.plugin_id, "");
    assert_eq!(row.plugin_factory, slint::ComponentFactory::default());

    // 没有可用面板的 id：空工厂，嵌不出东西也不 panic。
    switch_area(&store, area_key, AreaType::Plugin("no-such-plugin".into()));
    let row = scrarea_row(&store.borrow(), area_key);
    assert_eq!(row.plugin_id, "no-such-plugin");
    assert_eq!(row.plugin_factory, slint::ComponentFactory::default());
    assert_eq!(store.borrow().panel_instance_count("no-such-plugin"), 0);
}
