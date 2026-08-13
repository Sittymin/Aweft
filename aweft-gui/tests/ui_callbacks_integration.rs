//! 走真实 UI 回调链的无头集成测试。
//!
//! 与 `depsgraph_integration.rs` 的区别：这里调用 `setup_callbacks` 绑定
//! 真实 UI 回调，然后通过 `LineEditorCallbacks.invoke_*` 模拟用户在 UI 上的
//! 操作。覆盖完整链路：
//! `UI 回调（invoke）→ 回调层（uuid 解析 / parse_time / is_batch）→
//! dispatch → reducer 打 tag → evaluate_depsgraph → 投影 → UI 模型`。
//!
//! 用 Slint 的 testing 后端（无事件循环），`set_rendering_notifier` 仅返回
//! Err（eprintln），winit 窗口事件回调在非 winit 后端是 no-op，因此
//! `setup_callbacks` 可安全用于测试。

use std::cell::RefCell;
use std::rc::Rc;

use aweft_gui::bridge::callbacks::setup_callbacks;
use aweft_gui::bridge::types::{LineEditorCallbacks, MainWindow};
use aweft_gui::store::AppStore;
use aweft_model::structure::{EventData, ItemContent, ItemLocation};
use slint::{ComponentHandle, Model, ModelRc};

/// 构造未过滤的映射行模型（Rows → Map）。实际代码的过滤链路是
/// Rows → Filter → Map，此 helper 仅供测试访问全量映射行。
fn all_rows(store: &AppStore) -> ModelRc<aweft_gui::bridge::types::SlintSubtitleRow> {
    let rows_model: Rc<aweft_gui::store::line_rows_model::LineRowsModel> =
        Rc::new(store.ui_models().line_editor_state.rows_model().clone());
    let mapper = rows_model.mapper();
    let mapped = slint::MapModel::new(rows_model, move |data| mapper.map(&data));
    ModelRc::new(mapped)
}

/// 构造完整初始化的 store + 真实 UI 回调绑定。
/// 返回 (store, instance)，instance 保持存活（全局回调依赖组件树）。
fn setup() -> (Rc<RefCell<AppStore>>, MainWindow) {
    let instance = MainWindow::new().unwrap();
    let store = Rc::new(RefCell::new(AppStore::new(&instance)));
    setup_callbacks(&instance, Rc::clone(&store));

    // 初始化布局（真实运行时由 rendering_setup 触发，测试中手动触发）
    {
        let mut store_mut = store.borrow_mut();
        store_mut
            .dispatch(aweft_gui::actions::Action::Screen(
                aweft_gui::actions::screen::ScreenAction::InitializeLayout {
                    width: 800,
                    height: 600,
                },
            ))
            .unwrap();
        store_mut.evaluate_depsgraph();
    }

    // 注入测试字幕数据
    {
        let mut store_mut = store.borrow_mut();
        let mut structure = store_mut.state_mut().project.structure_mut();
        structure
            .create_event(
                EventData {
                    start_time: chrono::TimeDelta::try_milliseconds(0).unwrap(),
                    end_time: chrono::TimeDelta::try_milliseconds(1000).unwrap(),
                    actor: None,
                    text: "第一行".into(),
                    segment_timing: None,
                    style: "Default".into(),
                },
                ItemLocation::Root(0),
            )
            .unwrap();
        drop(structure);
        store_mut.rebuild_line_editor_models();
    }

    (store, instance)
}

/// 找到测试事件 id。
fn first_event_id(store: &Rc<RefCell<AppStore>>) -> aweft_model::ids::ItemId {
    let s = store.borrow();
    let structure = &*s.state().project.structure();
    structure
        .items
        .iter()
        .find(|(_, it)| matches!(it.content, ItemContent::Event(_)))
        .map(|(id, _)| *id)
        .expect("应有测试事件")
}

#[test]
fn ui_timecode_input_updates_row_via_callback() {
    i_slint_backend_testing::init_no_event_loop();
    let (store, instance) = setup();

    let event_id = first_event_id(&store);

    // 模拟用户在 UI 输入时间码（走真实回调 → dispatch → depsgraph）
    instance
        .global::<LineEditorCallbacks>()
        .invoke_timecode_input(
            event_id.to_string().into(),
            true,
            "0:00:00.50".into(),
            false,
        );

    // 行内容被更新（回调层 uuid 解析 + dispatch 全链路）
    let s = store.borrow();
    let rows = all_rows(&s);
    assert_eq!(rows.row_count(), 1);
    let row = rows.row_data(0).unwrap();
    assert_eq!(row.start_time.to_string(), "0:00:00.50");
    assert_eq!(row.item_id, slint::SharedString::from(event_id.to_string()));
}

#[test]
fn ui_add_line_via_callback_parses_time() {
    i_slint_backend_testing::init_no_event_loop();
    let (store, instance) = setup();

    let event_id = first_event_id(&store);

    // 通过真实回调新增一行（parse_time 在回调层解析）
    instance.global::<LineEditorCallbacks>().invoke_add_line(
        event_id.to_string().into(),
        "0:00:02.00".into(),
        "0:00:03.00".into(),
        "".into(),
    );

    let s = store.borrow();
    let rows = all_rows(&s);
    assert_eq!(rows.row_count(), 2, "add-line 应新增一行");
    // 新行 start = 传入 end_time；end = start + 参考行时长（保持时长往后推）。
    // 参考行 0→1s，时长 1s；end_time=3.00 → 新行 3.00→4.00
    let new_row = rows.row_data(1).unwrap();
    assert_eq!(new_row.start_time.to_string(), "0:00:03.00");
    assert_eq!(new_row.end_time.to_string(), "0:00:04.00");
}

#[test]
fn ui_toggle_group_collapsed_via_callback() {
    i_slint_backend_testing::init_no_event_loop();
    let (store, instance) = setup();

    // 添加一个分组 + 子行
    let anchor = first_event_id(&store);
    let group_id = {
        let mut store_mut = store.borrow_mut();
        let mut structure = store_mut.state_mut().project.structure_mut();
        let group_id = structure
            .insert_group_after(anchor, "测试组".into())
            .unwrap();
        structure
            .create_event(
                EventData {
                    start_time: chrono::TimeDelta::try_milliseconds(500).unwrap(),
                    end_time: chrono::TimeDelta::try_milliseconds(900).unwrap(),
                    actor: None,
                    text: "组内行".into(),
                    segment_timing: None,
                    style: "Default".into(),
                },
                ItemLocation::InGroup(group_id, 0),
            )
            .unwrap();
        drop(structure);
        store_mut.rebuild_line_editor_models();
        group_id
    };

    // 通过真实回调折叠分组
    instance
        .global::<LineEditorCallbacks>()
        .invoke_toggle_group_collapsed(group_id.to_string().into());

    // 分组行的 collapsed 字段被切换
    let s = store.borrow();
    let rows = all_rows(&s);
    let mut found = false;
    for i in 0..rows.row_count() {
        let row = rows.row_data(i).unwrap();
        if row.item_id == slint::SharedString::from(group_id.to_string()) {
            assert!(row.collapsed, "分组应被折叠");
            found = true;
        }
    }
    assert!(found, "应找到分组行");
}

#[test]
#[test]
fn ui_select_toggle_keeps_multi_selection() {
    // 验证 toggle 语义：先单选 e1，再 toggle e2，两行都应被选中。
    // 若投影把 SelectToggle 误降级为 select_single，则 e1 会被取消选中。
    i_slint_backend_testing::init_no_event_loop();
    let (store, instance) = setup();

    // 注入第二个事件
    let e2 = {
        let mut store_mut = store.borrow_mut();
        let mut structure = store_mut.state_mut().project.structure_mut();
        let id = structure
            .create_event(
                EventData {
                    start_time: chrono::TimeDelta::try_milliseconds(2000).unwrap(),
                    end_time: chrono::TimeDelta::try_milliseconds(3000).unwrap(),
                    actor: None,
                    text: "第二行".into(),
                    segment_timing: None,
                    style: "Default".into(),
                },
                ItemLocation::Root(1),
            )
            .unwrap();
        drop(structure);
        store_mut.rebuild_line_editor_models();
        id
    };
    // e1 = 第一个根事件（按锚点行序，不用 HashMap 无序的 first_event_id）
    let e1: aweft_model::ids::ItemId = {
        let s = store.borrow();
        let rows = all_rows(&s);
        let row0 = rows.row_data(0).unwrap();
        uuid::Uuid::parse_str(row0.item_id.as_str())
            .map(aweft_model::ids::ItemId)
            .expect("row0 应为有效 item_id")
    };

    // 先单选 e1
    instance
        .global::<LineEditorCallbacks>()
        .invoke_select_single(e1.to_string().into());

    // 验证单选后 e1 已选中
    {
        let s = store.borrow();
        let rows = all_rows(&s);
        let mut count = 0;
        for i in 0..rows.row_count() {
            let r = rows.row_data(i).unwrap();
            if r.selected == aweft_gui::bridge::types::SlintSelectionState::DirectlySelected {
                count += 1;
            }
        }
        assert_eq!(count, 1, "单选后应选中 1 行，实际 {count}");
    }

    // 再 toggle e2（不取消 e1）
    instance
        .global::<LineEditorCallbacks>()
        .invoke_select_toggle(e2.to_string().into());

    let s = store.borrow();
    let rows = all_rows(&s);
    let mut sel = Vec::new();
    for i in 0..rows.row_count() {
        let row = rows.row_data(i).unwrap();
        if row.selected == aweft_gui::bridge::types::SlintSelectionState::DirectlySelected {
            sel.push(row.item_id.to_string());
        }
    }
    assert_eq!(sel.len(), 2, "toggle 应保留两个选中行，实际选中: {:?}", sel);
}

#[test]
fn ui_invalid_uuid_callback_is_ignored() {
    i_slint_backend_testing::init_no_event_loop();
    let (store, instance) = setup();

    // 无效 uuid 应被回调层拒绝（不 panic，不 dispatch）
    instance
        .global::<LineEditorCallbacks>()
        .invoke_timecode_input("not-a-uuid".into(), true, "0:00:00.50".into(), false);

    let s = store.borrow();
    let rows = all_rows(&s);
    let row = rows.row_data(0).unwrap();
    assert_eq!(
        row.start_time.to_string(),
        "0:00:00.00",
        "无效 uuid 不应修改数据"
    );
}

#[test]
fn ui_ungroup_via_callback() {
    i_slint_backend_testing::init_no_event_loop();
    let (store, instance) = setup();

    // 添加一个分组 + 子行（记录子行 id，供 ungroup 后断言选中联动）
    let anchor = first_event_id(&store);
    let (group_id, child_id) = {
        let mut store_mut = store.borrow_mut();
        let mut structure = store_mut.state_mut().project.structure_mut();
        let group_id = structure
            .insert_group_after(anchor, "测试组".into())
            .unwrap();
        let child_id = structure
            .create_event(
                EventData {
                    start_time: chrono::TimeDelta::try_milliseconds(500).unwrap(),
                    end_time: chrono::TimeDelta::try_milliseconds(900).unwrap(),
                    actor: None,
                    text: "组内行".into(),
                    segment_timing: None,
                    style: "Default".into(),
                },
                ItemLocation::InGroup(group_id, 0),
            )
            .unwrap();
        drop(structure);
        store_mut.rebuild_line_editor_models();
        (group_id, child_id)
    };

    let before = {
        let s = store.borrow();
        all_rows(&s).row_count()
    };

    // 通过真实回调解散分组
    instance
        .global::<LineEditorCallbacks>()
        .invoke_ungroups(group_id.to_string().into());

    let s = store.borrow();
    let rows = all_rows(&s);
    // 解散后：分组头删除，子项提升到根（行数不变或减少分组头）
    assert_eq!(rows.row_count(), before - 1, "解散分组应删除分组头");
    drop(s);

    // 选中联动：子项提升后应被选中（reducer 打 SelectionChanged tag，
    // 依赖边保证结构操作先执行、选中操作后执行）
    let s = store.borrow();
    assert!(
        s.ui_models()
            .line_editor_state
            .rows_model()
            .is_selected(child_id),
        "解散分组后子项应被选中（reducer 声明联动）"
    );
}

#[test]
fn ui_timecode_input_reaggregates_group_header() {
    // 核心设计验证：分组头聚合（时间范围/行数）是惰性计算的。
    // 改子事件时间码 → 投影对分组头 notify_row_changed → Slint 重新拉取
    // 分组头行 → aggregate_group 现场重算——无需投影手动更新聚合值。
    i_slint_backend_testing::init_no_event_loop();
    let (store, instance) = setup();

    // 添加分组 + 子事件（初始 500-900ms）
    let (group_id, child_id) = {
        let mut store_mut = store.borrow_mut();
        let mut structure = store_mut.state_mut().project.structure_mut();
        let anchor = structure.root_items[0];
        let group_id = structure
            .insert_group_after(anchor, "测试组".into())
            .unwrap();
        let child_id = structure
            .create_event(
                EventData {
                    start_time: chrono::TimeDelta::try_milliseconds(500).unwrap(),
                    end_time: chrono::TimeDelta::try_milliseconds(900).unwrap(),
                    actor: None,
                    text: "组内行".into(),
                    segment_timing: None,
                    style: "Default".into(),
                },
                ItemLocation::InGroup(group_id, 0),
            )
            .unwrap();
        drop(structure);
        store_mut.rebuild_line_editor_models();
        (group_id, child_id)
    };

    // 初始：分组头聚合 = 子事件 500-900ms
    let s = store.borrow();
    let rows = all_rows(&s);
    let group_idx = (0..rows.row_count())
        .find(|&i| {
            rows.row_data(i)
                .map(|r| r.item_id.to_string() == group_id.to_string())
                .unwrap_or(false)
        })
        .unwrap();
    let before = rows.row_data(group_idx).unwrap();
    assert_eq!(before.start_time.to_string(), "0:00:00.50");
    assert_eq!(before.end_time.to_string(), "0:00:00.90");
    drop(s);

    // 通过真实回调把子事件 start 改为 0:00:00.10
    instance
        .global::<LineEditorCallbacks>()
        .invoke_timecode_input(
            child_id.to_string().into(),
            true,
            "0:00:00.10".into(),
            false,
        );

    // 分组头聚合自动重算：start 应变为 0:00:00.10
    let s = store.borrow();
    let rows = all_rows(&s);
    let after = rows.row_data(group_idx).unwrap();
    assert_eq!(
        after.start_time.to_string(),
        "0:00:00.10",
        "改子事件时间码后分组头聚合 start 应自动重算"
    );
    assert_eq!(after.end_time.to_string(), "0:00:00.90");
}

#[test]
fn ui_collapse_group_hides_children_in_filter_model() {
    // 验证方案 A：过滤在 RowData 层做（零 UUID parse），折叠分组后其子行
    // 从 filter 模型消失，而全量映射（all_rows helper，未过滤）仍保留全部行。
    i_slint_backend_testing::init_no_event_loop();
    let (store, instance) = setup();

    // 建分组 + 子行
    let anchor = first_event_id(&store);
    let (group_id, child_id) = {
        let mut store_mut = store.borrow_mut();
        let mut structure = store_mut.state_mut().project.structure_mut();
        let group_id = structure
            .insert_group_after(anchor, "测试组".into())
            .unwrap();
        let child_id = structure
            .create_event(
                EventData {
                    start_time: chrono::TimeDelta::try_milliseconds(500).unwrap(),
                    end_time: chrono::TimeDelta::try_milliseconds(900).unwrap(),
                    actor: None,
                    text: "组内行".into(),
                    segment_timing: None,
                    style: "Default".into(),
                },
                ItemLocation::InGroup(group_id, 0),
            )
            .unwrap();
        drop(structure);
        store_mut.rebuild_line_editor_models();
        (group_id, child_id)
    };

    // 折叠前：filter 模型应包含子行
    let filter_rows = {
        let mut s = store.borrow_mut();
        s.rebuild_line_editor_models()
    };
    let child_in_filter = (0..filter_rows.row_count()).any(|i| {
        filter_rows
            .row_data(i)
            .map(|r| r.item_id.to_string() == child_id.to_string())
            .unwrap_or(false)
    });
    assert!(child_in_filter, "折叠前子行应在 filter 模型中");

    // 折叠分组
    instance
        .global::<LineEditorCallbacks>()
        .invoke_toggle_group_collapsed(group_id.to_string().into());

    // 折叠后：filter 模型子行消失，但全量映射仍保留（未过滤）
    let filter_rows = {
        let mut s = store.borrow_mut();
        s.rebuild_line_editor_models()
    };
    let child_in_filter = (0..filter_rows.row_count()).any(|i| {
        filter_rows
            .row_data(i)
            .map(|r| r.item_id.to_string() == child_id.to_string())
            .unwrap_or(false)
    });
    assert!(!child_in_filter, "折叠后子行应从 filter 模型消失");

    let s = store.borrow();
    let all_rows = all_rows(&s);
    let child_in_all = (0..all_rows.row_count()).any(|i| {
        all_rows
            .row_data(i)
            .map(|r| r.item_id.to_string() == child_id.to_string())
            .unwrap_or(false)
    });
    assert!(child_in_all, "全量映射未过滤，子行应仍在");
}

#[test]
fn ui_select_events_updates_group_header_selected_state() {
    // 验证：组内事件单选 → 组头"部分选中"（Unselected）；全选 → 组头
    // DirectlySelected。确保 adjust_selected_counts_to_ancestors 只处理
    // 分组祖先（事件自身不混入 selected_event_counts）。
    i_slint_backend_testing::init_no_event_loop();
    let (store, instance) = setup();

    // 建组 + 2 个子事件
    let anchor = first_event_id(&store);
    let (group_id, e1, e2) = {
        let mut store_mut = store.borrow_mut();
        let mut structure = store_mut.state_mut().project.structure_mut();
        let gid = structure
            .insert_group_after(anchor, "测试组".into())
            .unwrap();
        let id1 = structure
            .create_event(
                EventData {
                    start_time: chrono::TimeDelta::try_milliseconds(500).unwrap(),
                    end_time: chrono::TimeDelta::try_milliseconds(900).unwrap(),
                    actor: None,
                    text: "组内1".into(),
                    segment_timing: None,
                    style: "Default".into(),
                },
                ItemLocation::InGroup(gid, 0),
            )
            .unwrap();
        let id2 = structure
            .create_event(
                EventData {
                    start_time: chrono::TimeDelta::try_milliseconds(1000).unwrap(),
                    end_time: chrono::TimeDelta::try_milliseconds(1400).unwrap(),
                    actor: None,
                    text: "组内2".into(),
                    segment_timing: None,
                    style: "Default".into(),
                },
                ItemLocation::InGroup(gid, 1),
            )
            .unwrap();
        drop(structure);
        store_mut.rebuild_line_editor_models();
        (gid, id1, id2)
    };

    // 单选 e1：组头未全选 → Unselected
    instance
        .global::<LineEditorCallbacks>()
        .invoke_select_single(e1.to_string().into());
    {
        let s = store.borrow();
        let rows = all_rows(&s);
        let group_row = (0..rows.row_count())
            .map(|i| rows.row_data(i).unwrap())
            .find(|r| r.item_id.to_string() == group_id.to_string())
            .expect("应有组头行");
        assert_eq!(
            group_row.selected,
            aweft_gui::bridge::types::SlintSelectionState::Unselected,
            "单选组内一个事件，组头不应全选"
        );
    }

    // 全选：先选 e1，再 toggle e2（多选模式）。
    // 组头未被直接点选（子项全选）→ 应 IndirectlySelected（恢复的三态）
    instance
        .global::<LineEditorCallbacks>()
        .invoke_select_toggle(e2.to_string().into());
    {
        let s = store.borrow();
        let rows = all_rows(&s);
        let group_row = (0..rows.row_count())
            .map(|i| rows.row_data(i).unwrap())
            .find(|r| r.item_id.to_string() == group_id.to_string())
            .expect("应有组头行");
        assert_eq!(
            group_row.selected,
            aweft_gui::bridge::types::SlintSelectionState::IndirectlySelected,
            "子项全选但组头未被直接点选 → IndirectlySelected"
        );
    }

    // 再直接点选组头 → 组头应变为 DirectlySelected
    instance
        .global::<LineEditorCallbacks>()
        .invoke_select_single(group_id.to_string().into());
    {
        let s = store.borrow();
        let rows = all_rows(&s);
        let group_row = (0..rows.row_count())
            .map(|i| rows.row_data(i).unwrap())
            .find(|r| r.item_id.to_string() == group_id.to_string())
            .expect("应有组头行");
        assert_eq!(
            group_row.selected,
            aweft_gui::bridge::types::SlintSelectionState::DirectlySelected,
            "直接点选组头 → DirectlySelected"
        );
    }
}

#[test]
fn ui_copy_cut_paste_delete_chain() {
    // 少而全：一条链覆盖 CopyItems + PasteItems + CutItems + DeleteItems
    // 四条 reducer 分支，以及剪贴板状态 + 结构精确增删
    // （handle_subtrees_pasted 批量插入 / handle_items_removed 锚点删除）。
    i_slint_backend_testing::init_no_event_loop();
    let (store, instance) = setup();

    // 建一个组 + 组内子行（含 e1, e2）
    let anchor = first_event_id(&store);
    let (group_id, e1, e2) = {
        let mut store_mut = store.borrow_mut();
        let mut structure = store_mut.state_mut().project.structure_mut();
        let gid = structure
            .insert_group_after(anchor, "复制组".into())
            .unwrap();
        let id1 = structure
            .create_event(
                EventData {
                    start_time: chrono::TimeDelta::try_milliseconds(500).unwrap(),
                    end_time: chrono::TimeDelta::try_milliseconds(900).unwrap(),
                    actor: None,
                    text: "源1".into(),
                    segment_timing: None,
                    style: "Default".into(),
                },
                ItemLocation::InGroup(gid, 0),
            )
            .unwrap();
        let id2 = structure
            .create_event(
                EventData {
                    start_time: chrono::TimeDelta::try_milliseconds(1000).unwrap(),
                    end_time: chrono::TimeDelta::try_milliseconds(1400).unwrap(),
                    actor: None,
                    text: "源2".into(),
                    segment_timing: None,
                    style: "Default".into(),
                },
                ItemLocation::InGroup(gid, 1),
            )
            .unwrap();
        drop(structure);
        store_mut.rebuild_line_editor_models();
        (gid, id1, id2)
    };

    let count_before = {
        let s = store.borrow();
        all_rows(&s).row_count()
    };

    // 1. 选中 e1 → CopyItems（剪贴板应写入）
    instance
        .global::<LineEditorCallbacks>()
        .invoke_select_single(e1.to_string().into());
    instance.global::<LineEditorCallbacks>().invoke_copy_items();

    // 2. 粘贴到 e1 后 → PasteItems（行数增加，handle_subtrees_pasted 精确插入）
    instance
        .global::<LineEditorCallbacks>()
        .invoke_paste_items(e1.to_string().into());
    {
        let s = store.borrow();
        let rows = all_rows(&s);
        assert_eq!(rows.row_count(), count_before + 1, "粘贴 e1 后应 +1 行");
        // 粘贴内容存在（源1 文本）
        let has_copied = (0..rows.row_count()).any(|i| {
            rows.row_data(i)
                .map(|r| r.text.to_string() == "源1")
                .unwrap_or(false)
        });
        assert!(has_copied, "粘贴后应存在源1文本");
    }

    // 3. CutItems：复制 + 删除选中（e1 被删，剪贴板保留）
    let count_before_cut = {
        let s = store.borrow();
        all_rows(&s).row_count()
    };
    instance
        .global::<LineEditorCallbacks>()
        .invoke_select_single(e1.to_string().into());
    instance.global::<LineEditorCallbacks>().invoke_cut_items();
    {
        let s = store.borrow();
        let rows = all_rows(&s);
        assert_eq!(rows.row_count(), count_before_cut - 1, "Cut 应删除选中行");
        // e1 不再存在
        let has_e1 = (0..rows.row_count()).any(|i| {
            rows.row_data(i)
                .map(|r| r.item_id.to_string() == e1.to_string())
                .unwrap_or(false)
        });
        assert!(!has_e1, "Cut 后 e1 应被删除");
    }

    // 4. DeleteItems：删除选中（e2）→ 行数再减 1
    let count_before_del = {
        let s = store.borrow();
        all_rows(&s).row_count()
    };
    instance
        .global::<LineEditorCallbacks>()
        .invoke_select_single(e2.to_string().into());
    instance
        .global::<LineEditorCallbacks>()
        .invoke_delete_selected_items();
    {
        let s = store.borrow();
        let rows = all_rows(&s);
        assert_eq!(
            rows.row_count(),
            count_before_del - 1,
            "Delete 应删除选中行"
        );
        let has_e2 = (0..rows.row_count()).any(|i| {
            rows.row_data(i)
                .map(|r| r.item_id.to_string() == e2.to_string())
                .unwrap_or(false)
        });
        assert!(!has_e2, "Delete 后 e2 应被删除");
        // 组头仍在（空组也保留）
        let has_group = (0..rows.row_count()).any(|i| {
            rows.row_data(i)
                .map(|r| r.item_id.to_string() == group_id.to_string())
                .unwrap_or(false)
        });
        assert!(has_group, "删除组内事件后组头应保留");
    }
}

#[test]
fn ui_add_group_and_batch_timecode() {
    // 少而全：AddGroup + 批量时间码（is_batch）两条分支。
    i_slint_backend_testing::init_no_event_loop();
    let (store, instance) = setup();

    // 建组（AddGroup 分支）
    let anchor = first_event_id(&store);
    let count_before = {
        let s = store.borrow();
        all_rows(&s).row_count()
    };
    instance
        .global::<LineEditorCallbacks>()
        .invoke_add_group(anchor.to_string().into());
    let group_id = {
        let s = store.borrow();
        let rows = all_rows(&s);
        assert_eq!(
            rows.row_count(),
            count_before + 1,
            "AddGroup 应 +1 行（分组头）"
        );
        // 新分组名 = "新分组 1"（insert_group_after 命名规则）
        (0..rows.row_count())
            .map(|i| rows.row_data(i).unwrap())
            .find(|r| r.row_type == aweft_gui::bridge::types::SlintSubtitleRowType::GroupHeader)
            .map(|r| uuid::Uuid::parse_str(r.item_id.as_str()).unwrap())
            .map(aweft_model::ids::ItemId)
            .expect("应有新分组头")
    };

    // 批量时间码：建两个子事件，is_batch 批量改（在回调层解析）
    let (e1, e2) = {
        let mut store_mut = store.borrow_mut();
        let mut structure = store_mut.state_mut().project.structure_mut();
        let id1 = structure
            .create_event(
                EventData {
                    start_time: chrono::TimeDelta::try_milliseconds(1000).unwrap(),
                    end_time: chrono::TimeDelta::try_milliseconds(9000).unwrap(),
                    actor: None,
                    text: "批1".into(),
                    segment_timing: None,
                    style: "Default".into(),
                },
                ItemLocation::InGroup(group_id, 0),
            )
            .unwrap();
        let id2 = structure
            .create_event(
                EventData {
                    start_time: chrono::TimeDelta::try_milliseconds(3000).unwrap(),
                    end_time: chrono::TimeDelta::try_milliseconds(9000).unwrap(),
                    actor: None,
                    text: "批2".into(),
                    segment_timing: None,
                    style: "Default".into(),
                },
                ItemLocation::InGroup(group_id, 1),
            )
            .unwrap();
        drop(structure);
        store_mut.rebuild_line_editor_models();
        (id1, id2)
    };

    // 选中 e1 和 e2（toggle 多选），批量改 start_time（is_batch=true）
    instance
        .global::<LineEditorCallbacks>()
        .invoke_select_single(e1.to_string().into());
    instance
        .global::<LineEditorCallbacks>()
        .invoke_select_toggle(e2.to_string().into());
    // 批量输入时间码：回调的 is_batch 参数 = true
    // 验证：两个事件 start_time 都变为 0:00:05.00
    instance
        .global::<LineEditorCallbacks>()
        .invoke_timecode_input(e1.to_string().into(), true, "0:00:05.00".into(), true);

    {
        let s = store.borrow();
        let rows = all_rows(&s);
        for id in [e1, e2] {
            let row = (0..rows.row_count())
                .map(|i| rows.row_data(i).unwrap())
                .find(|r| r.item_id.to_string() == id.to_string())
                .expect("事件应存在");
            assert_eq!(
                row.start_time.to_string(),
                "0:00:05.00",
                "批量时间码：{} 的 start_time 应更新为 0:00:05.00",
                id
            );
        }
        // 分组头聚合重算（时间码变化 → 祖先组头更新）
        let group_row = (0..rows.row_count())
            .map(|i| rows.row_data(i).unwrap())
            .find(|r| r.item_id.to_string() == group_id.to_string())
            .expect("组头应存在");
        assert_eq!(
            group_row.start_time.to_string(),
            "0:00:05.00",
            "批量改时间码后分组头聚合 start 应更新"
        );
    }
}

#[test]
fn ui_drag_timecode_preview_shows_override_then_commit() {
    // 验证：拖动修改时间的实时预览。
    // - preview：UI 显示覆盖值，structure 不变
    // - commit：structure 持久化，预览清除
    i_slint_backend_testing::init_no_event_loop();
    let (store, instance) = setup();

    let event_id = first_event_id(&store);

    // preview：start 从 0 拖到 +500ms（delta_x 累计，reducer 用 structure 原值 + delta）
    instance
        .global::<LineEditorCallbacks>()
        .invoke_drag_timecode_preview(event_id.to_string().into(), true, 50.0.into(), false);
    {
        let s = store.borrow();
        let rows = all_rows(&s);
        let row = rows.row_data(0).unwrap();
        // delta 50 * 10ms = 500ms → start 显示 0:00:00.50
        assert_eq!(
            row.start_time.to_string(),
            "0:00:00.50",
            "preview 应显示覆盖值（structure 不变）"
        );
        // structure 未变：原始 start 还是 0
        let s = store.borrow();
        let structure = s.state().project.structure();
        let ev = structure.get(event_id).expect("事件应存在");
        let aweft_model::structure::ItemContent::Event(e) = &ev.content else {
            panic!("应为事件");
        };
        assert_eq!(
            e.start_time,
            chrono::TimeDelta::try_milliseconds(0).unwrap(),
            "preview 不应修改 structure 的 start_time"
        );
        assert_eq!(
            e.end_time,
            chrono::TimeDelta::try_milliseconds(1000).unwrap(),
            "preview 不应修改 structure 的 end_time"
        );
    }

    // commit：同 delta，structure 持久化 + 预览清除
    instance
        .global::<LineEditorCallbacks>()
        .invoke_drag_timecode_commit(event_id.to_string().into(), true, 50.0.into(), false);
    {
        let s = store.borrow();
        let structure = s.state().project.structure();
        let ev = structure.get(event_id).expect("事件应存在");
        let aweft_model::structure::ItemContent::Event(e) = &ev.content else {
            panic!("应为事件");
        };
        assert_eq!(
            e.start_time,
            chrono::TimeDelta::try_milliseconds(500).unwrap(),
            "commit 应持久化 start_time"
        );
        // UI 显示同样 500（structure 现在也是 500，预览清除后一致）
        let rows = all_rows(&s);
        let row = rows.row_data(0).unwrap();
        assert_eq!(row.start_time.to_string(), "0:00:00.50");
    }
}

#[test]
fn ui_select_range_marks_group_header_indirectly_selected() {
    // 验证：范围选中整组子项（组头本身未直接点选）→ 组头 IndirectlySelected。
    // 依赖图化后 select_range 曾把组头也 DirectlySelected，Indirectly 从不出现。
    i_slint_backend_testing::init_no_event_loop();
    let (store, instance) = setup();

    // 建组 + 2 个子事件
    let anchor = first_event_id(&store);
    let (group_id, e1, e2) = {
        let mut store_mut = store.borrow_mut();
        let mut structure = store_mut.state_mut().project.structure_mut();
        let gid = structure
            .insert_group_after(anchor, "范围组".into())
            .unwrap();
        let id1 = structure
            .create_event(
                EventData {
                    start_time: chrono::TimeDelta::try_milliseconds(500).unwrap(),
                    end_time: chrono::TimeDelta::try_milliseconds(900).unwrap(),
                    actor: None,
                    text: "子1".into(),
                    segment_timing: None,
                    style: "Default".into(),
                },
                ItemLocation::InGroup(gid, 0),
            )
            .unwrap();
        let id2 = structure
            .create_event(
                EventData {
                    start_time: chrono::TimeDelta::try_milliseconds(1000).unwrap(),
                    end_time: chrono::TimeDelta::try_milliseconds(1400).unwrap(),
                    actor: None,
                    text: "子2".into(),
                    segment_timing: None,
                    style: "Default".into(),
                },
                ItemLocation::InGroup(gid, 1),
            )
            .unwrap();
        drop(structure);
        store_mut.rebuild_line_editor_models();
        (gid, id1, id2)
    };

    // 先单选 e1（设 anchor），再 range 到 e2 → 范围覆盖两个子项，组头未直接点选
    instance
        .global::<LineEditorCallbacks>()
        .invoke_select_single(e1.to_string().into());
    instance
        .global::<LineEditorCallbacks>()
        .invoke_select_range(e2.to_string().into());

    let s = store.borrow();
    let rows = all_rows(&s);
    // 两个子项 DirectlySelected
    for id in [e1, e2] {
        let row = (0..rows.row_count())
            .map(|i| rows.row_data(i).unwrap())
            .find(|r| r.item_id.to_string() == id.to_string())
            .expect("子项应存在");
        assert_eq!(
            row.selected,
            aweft_gui::bridge::types::SlintSelectionState::DirectlySelected,
            "范围覆盖的子项应 DirectlySelected"
        );
    }
    // 组头 IndirectlySelected（子项全选但未直接点选组头）
    let group_row = (0..rows.row_count())
        .map(|i| rows.row_data(i).unwrap())
        .find(|r| r.item_id.to_string() == group_id.to_string())
        .expect("应有组头行");
    assert_eq!(
        group_row.selected,
        aweft_gui::bridge::types::SlintSelectionState::IndirectlySelected,
        "范围选中整组子项后组头应 IndirectlySelected（未直接点选）"
    );
}

#[test]
fn ui_reselect_single_refreshes_group_header_state() {
    // 验证：组头 Indirectly（子项全选）后重新单选一个子项，
    // 组头应变 Unselected（select_single 必须刷新组头派生状态）。
    i_slint_backend_testing::init_no_event_loop();
    let (store, instance) = setup();

    let anchor = first_event_id(&store);
    let (group_id, e1, e2) = {
        let mut store_mut = store.borrow_mut();
        let mut structure = store_mut.state_mut().project.structure_mut();
        let gid = structure
            .insert_group_after(anchor, "刷新组".into())
            .unwrap();
        let id1 = structure
            .create_event(
                EventData {
                    start_time: chrono::TimeDelta::try_milliseconds(500).unwrap(),
                    end_time: chrono::TimeDelta::try_milliseconds(900).unwrap(),
                    actor: None,
                    text: "子1".into(),
                    segment_timing: None,
                    style: "Default".into(),
                },
                ItemLocation::InGroup(gid, 0),
            )
            .unwrap();
        let id2 = structure
            .create_event(
                EventData {
                    start_time: chrono::TimeDelta::try_milliseconds(1000).unwrap(),
                    end_time: chrono::TimeDelta::try_milliseconds(1400).unwrap(),
                    actor: None,
                    text: "子2".into(),
                    segment_timing: None,
                    style: "Default".into(),
                },
                ItemLocation::InGroup(gid, 1),
            )
            .unwrap();
        drop(structure);
        store_mut.rebuild_line_editor_models();
        (gid, id1, id2)
    };

    // 范围选中整组 → 组头 Indirectly
    instance
        .global::<LineEditorCallbacks>()
        .invoke_select_single(e1.to_string().into());
    instance
        .global::<LineEditorCallbacks>()
        .invoke_select_range(e2.to_string().into());
    {
        let s = store.borrow();
        let rows = all_rows(&s);
        let g = (0..rows.row_count())
            .map(|i| rows.row_data(i).unwrap())
            .find(|r| r.item_id.to_string() == group_id.to_string())
            .unwrap();
        assert_eq!(
            g.selected,
            aweft_gui::bridge::types::SlintSelectionState::IndirectlySelected,
            "前置：范围选中整组 → 组头 Indirectly"
        );
    }

    // 重新单选 e1（只选一个子项）→ 组头应刷新为 Unselected
    instance
        .global::<LineEditorCallbacks>()
        .invoke_select_single(e1.to_string().into());
    {
        let s = store.borrow();
        let rows = all_rows(&s);
        let g = (0..rows.row_count())
            .map(|i| rows.row_data(i).unwrap())
            .find(|r| r.item_id.to_string() == group_id.to_string())
            .unwrap();
        assert_eq!(
            g.selected,
            aweft_gui::bridge::types::SlintSelectionState::Unselected,
            "重新单选一个子项后组头应变 Unselected（未全选）"
        );
        // e2 被取消选中
        let e2_row = (0..rows.row_count())
            .map(|i| rows.row_data(i).unwrap())
            .find(|r| r.item_id.to_string() == e2.to_string())
            .unwrap();
        assert_eq!(
            e2_row.selected,
            aweft_gui::bridge::types::SlintSelectionState::Unselected,
            "重新单选后 e2 应被取消"
        );
    }
}

#[test]
fn ui_select_outside_group_refreshes_group_header() {
    // 验证：组内所有事件全选（组头 Indirectly）后，选组外事件 →
    // 组头应变 Unselected（refresh 需遍历 selected 中已设组头态的项，
    // 不能只依赖 selected_event_counts——组头子项清空后不在计数里）。
    i_slint_backend_testing::init_no_event_loop();
    let (store, instance) = setup();

    // setup 有根事件（组外）。加组 + 组内 2 事件。
    let outside = first_event_id(&store);
    let (group_id, e1, e2) = {
        let mut store_mut = store.borrow_mut();
        let mut structure = store_mut.state_mut().project.structure_mut();
        let gid = structure.insert_group_after(outside, "组".into()).unwrap();
        let id1 = structure
            .create_event(
                EventData {
                    start_time: chrono::TimeDelta::try_milliseconds(500).unwrap(),
                    end_time: chrono::TimeDelta::try_milliseconds(900).unwrap(),
                    actor: None,
                    text: "内1".into(),
                    segment_timing: None,
                    style: "Default".into(),
                },
                ItemLocation::InGroup(gid, 0),
            )
            .unwrap();
        let id2 = structure
            .create_event(
                EventData {
                    start_time: chrono::TimeDelta::try_milliseconds(1000).unwrap(),
                    end_time: chrono::TimeDelta::try_milliseconds(1400).unwrap(),
                    actor: None,
                    text: "内2".into(),
                    segment_timing: None,
                    style: "Default".into(),
                },
                ItemLocation::InGroup(gid, 1),
            )
            .unwrap();
        drop(structure);
        store_mut.rebuild_line_editor_models();
        (gid, id1, id2)
    };

    // 范围选中整组（组内全选）→ 组头 Indirectly
    instance
        .global::<LineEditorCallbacks>()
        .invoke_select_single(e1.to_string().into());
    instance
        .global::<LineEditorCallbacks>()
        .invoke_select_range(e2.to_string().into());
    {
        let s = store.borrow();
        let rows = all_rows(&s);
        let g = (0..rows.row_count())
            .map(|i| rows.row_data(i).unwrap())
            .find(|r| r.item_id.to_string() == group_id.to_string())
            .unwrap();
        assert_eq!(
            g.selected,
            aweft_gui::bridge::types::SlintSelectionState::IndirectlySelected,
            "前置：范围选中整组 → 组头 Indirectly"
        );
    }

    // 选组外事件 → 组头应刷新为 Unselected
    instance
        .global::<LineEditorCallbacks>()
        .invoke_select_single(outside.to_string().into());
    {
        let s = store.borrow();
        let rows = all_rows(&s);
        let g = (0..rows.row_count())
            .map(|i| rows.row_data(i).unwrap())
            .find(|r| r.item_id.to_string() == group_id.to_string())
            .unwrap();
        assert_eq!(
            g.selected,
            aweft_gui::bridge::types::SlintSelectionState::Unselected,
            "选组外事件后组头应 Unselected（组内子项已全部取消）"
        );
    }
}
