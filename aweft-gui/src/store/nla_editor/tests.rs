use super::{NlaEditorState, NlaStripData, StripDrag};
use crate::bridge::types::{SlintNlaStrip, SlintSelectionModifier, SlintStripType};
use crate::depsgraph::{NlaStripInsert, NlaStripMove, NlaStripRef, NlaStripReorder, Tag};
use aweft_model::{
    animation::{
        AnimData,
        strip::{NlaStrip, NlaTrack, StripPlacement},
    },
    ids::ActionId,
};
use slint::Model;

fn strip(name: &str, start: f32, end: f32) -> SlintNlaStrip {
    SlintNlaStrip {
        name: name.into(),
        strip_type: SlintStripType::Action,
        start,
        end,
        muted: false,
        selected: false,
        dragging: false,
    }
}

/// 调用 reducer（数据层操作）后手动跑投影（测试中替代 depsgraph evaluate）。
fn project(st: &mut NlaEditorState, tags: Vec<Tag>) {
    let has_structure = tags.iter().any(|t| matches!(t, Tag::NlaStructureChanged));
    let has_track_changed = tags
        .iter()
        .any(|t| matches!(t, Tag::NlaTrackChanged { .. }));
    let has_track_added = tags.iter().any(|t| matches!(t, Tag::NlaTrackAdded { .. }));
    let has_strips = tags.iter().any(|t| {
        matches!(
            t,
            Tag::NlaSelectionChanged { .. }
                | Tag::NlaStripPositionChanged { .. }
                | Tag::NlaStripStructureChanged { .. }
        )
    });
    if has_structure {
        st.apply_structure();
    }
    if has_track_changed {
        st.apply_track_properties(&tags);
    }
    if has_track_added {
        st.apply_track_added(&tags);
    }
    if has_strips {
        st.apply_selection(&tags);
        st.apply_strip_structure(&tags);
        st.apply_position(&tags);
    }
}

#[test]
fn strip_drag_moves_whole_strip() {
    let mut st = NlaEditorState::default();
    st.push_track("t", vec![strip("A", 1.0, 4.0), strip("B", 6.0, 9.0)], false);
    st.tracks.row_data(0).unwrap();

    let mut tags = st.strip_pressed(0, 0, SlintSelectionModifier::None);
    tags.extend(st.strip_dragged(0, 0, 2.0, 0.0));
    project(&mut st, tags);

    let s = st.strip_models[0].row_data(0).unwrap();
    assert_eq!(s.start, 3.0);
    assert_eq!(s.end, 6.0);
    assert!(s.selected);
    assert!(s.dragging);

    let tags = st.strip_released(0, 0);
    project(&mut st, tags);
    assert!(st.drag.is_none());
    assert!(!st.strip_models[0].row_data(0).unwrap().dragging);
}

#[test]
fn strip_pick_selects_and_records_drag() {
    let mut st = NlaEditorState::default();
    st.push_track("t", vec![strip("A", 1.0, 4.0), strip("B", 6.0, 9.0)], false);

    let (hit, tags) = st.strip_pick(2.5, 0, SlintSelectionModifier::None);
    assert_eq!(hit, 0);
    project(&mut st, tags);
    let s = st.strip_models[0].row_data(0).unwrap();
    assert!(s.selected);
    assert!(!st.strip_models[0].row_data(1).unwrap().selected);

    let tags = st.strip_dragged(0, 0, 1.5, 0.0);
    project(&mut st, tags);
    let s = st.strip_models[0].row_data(0).unwrap();
    assert_eq!(s.start, 2.5);
    assert_eq!(s.end, 5.5);
}

#[test]
fn strip_pick_misses_return_neg_one() {
    let mut st = NlaEditorState::default();
    st.push_track("t", vec![strip("A", 1.0, 4.0)], false);

    assert_eq!(st.strip_pick(7.0, 0, SlintSelectionModifier::None).0, -1);
    assert_eq!(st.strip_pick(2.0, 5, SlintSelectionModifier::None).0, -1);
    assert_eq!(st.strip_pick(2.0, -1, SlintSelectionModifier::None).0, -1);
}

#[test]
fn strip_pick_skips_action_track() {
    let mut st = NlaEditorState::default();
    // 全局 Action 轨：NLA 中只能看不能编辑，即使时间命中也返回 -1
    st.push_track("全局 Action", vec![strip("x", 1.0, 4.0)], true);

    assert_eq!(st.strip_pick(2.5, 0, SlintSelectionModifier::None).0, -1);
}

#[test]
fn strip_pressed_selection_is_single() {
    let mut st = NlaEditorState::default();
    st.push_track("t", vec![strip("A", 1.0, 4.0), strip("B", 6.0, 9.0)], false);

    let tags = st.strip_pressed(0, 0, SlintSelectionModifier::None);
    project(&mut st, tags);
    let tags = st.strip_pressed(0, 1, SlintSelectionModifier::None);
    project(&mut st, tags);

    assert!(st.strip_models[0].row_data(1).unwrap().selected);
    assert!(!st.strip_models[0].row_data(0).unwrap().selected);
}

#[test]
fn strip_pressed_additive_toggles() {
    let mut st = NlaEditorState::default();
    st.push_track("t", vec![strip("A", 1.0, 4.0), strip("B", 6.0, 9.0)], false);

    let tags = st.strip_pressed(0, 0, SlintSelectionModifier::Ctrl);
    project(&mut st, tags);
    let tags = st.strip_pressed(0, 1, SlintSelectionModifier::Ctrl);
    project(&mut st, tags);
    let s0 = st.strip_models[0].row_data(0).unwrap();
    let s1 = st.strip_models[0].row_data(1).unwrap();
    assert!(s0.selected);
    assert!(s1.selected, "追加选中不清除已有选中");

    let tags = st.strip_pressed(0, 0, SlintSelectionModifier::Ctrl);
    project(&mut st, tags);
    assert!(
        !st.strip_models[0].row_data(0).unwrap().selected,
        "再次切换取消"
    );
}

#[test]
fn track_toggles_update_rows() {
    let mut st = NlaEditorState::default();
    st.push_track("t", vec![strip("A", 1.0, 4.0)], false);

    let tags = st.toggle_track_enabled(0);
    project(&mut st, tags);
    let tags = st.toggle_track_locked(0);
    project(&mut st, tags);
    let t = st.tracks.row_data(0).unwrap();
    assert!(!t.enabled);
    assert!(t.locked);
    assert!(!st.tracks_filter.row_data(0).unwrap().enabled);
    assert!(st.tracks_filter.row_data(0).unwrap().locked);
    assert_eq!(
        st.strip_models[0].row_count(),
        1,
        "轨道属性更新不能断开片断模型"
    );
    assert_eq!(st.strip_models[0].row_data(0).unwrap().name, "A");
}

#[test]
fn filter_tracks_by_name() {
    let mut st = NlaEditorState::default();
    st.push_track("Idle", vec![], false);
    st.push_track("Run", vec![strip("R", 0.0, 1.0)], false);
    st.push_track("Idle 2", vec![], false);

    st.filter_tracks("Idle");
    st.apply_structure();
    assert_eq!(st.tracks_filter.row_count(), 2);
    assert_eq!(st.tracks_filter.row_data(0).unwrap().orig_index, 0);
    assert_eq!(st.tracks_filter.row_data(1).unwrap().orig_index, 2);

    // 命中经过滤视图行号映射回权威轨道
    st.filter_tracks("");
    st.apply_structure();
    let (hit, _) = st.strip_pick(0.5, 1, SlintSelectionModifier::None);
    assert_eq!(hit, 1 * 10000 + 0);
}

#[test]
fn push_down_uses_existing_track_when_space() {
    let mut st = NlaEditorState::default();
    st.push_track("全局 Action", vec![], true); // 轨道0 只读
    st.push_track("Idle", vec![], false); // 轨道1 空，可容纳
    st.push_track("Run", vec![], false);
    st.toggle_track_locked(2); // 轨道2 锁定

    let inserted = st.push_down_insert("主 Action", 1.0, 5.0, ActionId::default());
    let inserted = inserted.unwrap();
    assert_eq!(inserted.track, 1);
    assert_eq!(inserted.strip, 0);
    assert!(!inserted.new_track);
    project(
        &mut st,
        vec![Tag::NlaStripStructureChanged {
            moved: Vec::new(),
            reordered: Vec::new(),
            inserted: vec![NlaStripInsert { track: 1, index: 0 }],
        }],
    );

    assert_eq!(
        st.strip_models[0].row_count(),
        0,
        "全局 Action 轨不接收片断"
    );
    assert_eq!(st.strip_models[2].row_count(), 0, "锁定轨不接收片断");
    assert_eq!(st.strip_models[1].row_count(), 1, "空轨道可直接容纳");
    let s = st.strip_models[1].row_data(0).unwrap();
    assert_eq!(s.name, "主 Action");
    assert_eq!(s.start, 1.0);
    assert_eq!(s.end, 5.0);
    assert!(!s.dragging);
}

#[test]
fn push_down_inserts_sorted_into_fitting_track() {
    let mut st = NlaEditorState::default();
    st.push_track("Idle", vec![strip("A", 3.0, 6.0)], false);

    // [7,10] 与 A[3,6] 不重叠 → 插入轨道0（按 start 排序在 A 后）
    let inserted = st.push_down_insert("Action", 7.0, 10.0, ActionId::default());
    let inserted = inserted.unwrap();
    assert_eq!(inserted.track, 0);
    assert_eq!(inserted.strip, 1);
    assert!(!inserted.new_track);
    project(
        &mut st,
        vec![Tag::NlaStripStructureChanged {
            moved: Vec::new(),
            reordered: Vec::new(),
            inserted: vec![NlaStripInsert { track: 0, index: 1 }],
        }],
    );
    assert_eq!(st.tracks.row_count(), 1, "已有轨道能容纳则新建");
    assert_eq!(st.strip_models[0].row_count(), 2);
    assert_eq!(st.strip_models[0].row_data(0).unwrap().name, "A");
    assert_eq!(st.strip_models[0].row_data(1).unwrap().name, "Action");
}

#[test]
fn push_down_creates_new_track_when_no_space() {
    let mut st = NlaEditorState::default();
    st.push_track("Idle", vec![strip("A", 3.0, 6.0)], false);

    // [1,4] 与 A[3,6] 重叠 → 已有轨道放不下 → 新建轨道
    let inserted = st.push_down_insert("Action", 1.0, 4.0, ActionId::default());
    let inserted = inserted.unwrap();
    assert_eq!(inserted.track, 1);
    assert_eq!(inserted.strip, 0);
    assert!(inserted.new_track);
    project(&mut st, vec![Tag::NlaTrackAdded { track: 1 }]);
    assert_eq!(st.tracks.row_count(), 2);
    assert_eq!(st.strip_models[0].row_count(), 1, "原轨道不动");
    assert_eq!(st.strip_models[1].row_count(), 1);
    let s = st.strip_models[1].row_data(0).unwrap();
    assert_eq!(s.name, "Action");
    assert_eq!(s.start, 1.0);
    assert_eq!(s.end, 4.0);
}

#[test]
fn push_down_creates_track_even_if_all_readonly() {
    let mut st = NlaEditorState::default();
    st.push_track("全局 Action", vec![], true);
    let inserted = st.push_down_insert("X", 0.0, 1.0, ActionId::default());
    let inserted = inserted.unwrap();
    assert_eq!(inserted.track, 1);
    assert_eq!(inserted.strip, 0);
    assert!(inserted.new_track);
    project(&mut st, vec![Tag::NlaTrackAdded { track: 1 }]);
    assert_eq!(st.tracks.row_count(), 2, "只读轨放不下则新建");
    assert_eq!(st.strip_models[1].row_data(0).unwrap().name, "X");
}

#[test]
fn global_action_sync_updates_name_and_has_data() {
    let mut st = NlaEditorState::default();
    st.push_track("全局 Action", vec![], true);
    st.push_track("Idle", vec![], false);

    let tags = st.set_global_action("主 Action", true);
    project(&mut st, tags);
    let t = st.tracks.row_data(0).unwrap();
    assert_eq!(t.name, "主 Action");
    assert!(t.has_data);
    assert!(!st.tracks.row_data(1).unwrap().has_data, "普通轨道不受影响");
    assert!(
        st.tracks_filter.row_data(0).unwrap().has_data,
        "过滤投影同步"
    );

    // 下插后变空：无数据 → 插入按钮隐藏
    let tags = st.set_global_action("主 Action", false);
    project(&mut st, tags);
    assert!(!st.tracks.row_data(0).unwrap().has_data);

    // 空 Action（无名称）→ 显示"<无动作>"
    let tags = st.set_global_action("", false);
    project(&mut st, tags);
    assert_eq!(st.tracks.row_data(0).unwrap().name, "<无动作>");
}

#[test]
fn locked_track_is_not_editable() {
    let mut st = NlaEditorState::default();
    st.push_track("t", vec![strip("A", 1.0, 4.0)], false);
    st.toggle_track_locked(0);

    // 锁定轨道：点击命中返回 -1（不能选中/拖拽）
    assert_eq!(st.strip_pick(2.5, 0, SlintSelectionModifier::None).0, -1);
    // 直接拖拽也被忽略
    let tags = st.strip_pressed(0, 0, SlintSelectionModifier::None);
    project(&mut st, tags);
    let tags = st.strip_dragged(0, 0, 2.0, 0.0);
    project(&mut st, tags);
    let s = st.strip_models[0].row_data(0).unwrap();
    assert_eq!(s.start, 1.0);
    assert_eq!(s.end, 4.0);
}

#[test]
fn vertical_drag_moves_strip_to_other_track() {
    let mut st = NlaEditorState::default();
    st.push_track("t0", vec![strip("A", 1.0, 4.0)], false);
    st.push_track("t1", vec![], false);

    // 垂直位移 0.6 行：片段中心越过轨道边界 → 拖动中瞬间跳到轨道 1
    let tags = st.strip_pressed(0, 0, SlintSelectionModifier::None);
    project(&mut st, tags);
    let tags = st.strip_dragged(0, 0, 0.0, 0.6);
    assert_eq!(st.editing_strips[0].len(), 0, "数据层已跨轨");
    project(&mut st, tags);
    assert_eq!(st.strip_models[0].row_count(), 0, "拖动中即应瞬间跨轨");
    assert_eq!(st.strip_models[1].row_count(), 1);
    let s = st.strip_models[1].row_data(0).unwrap();
    assert_eq!(s.name, "A");
    assert_eq!(s.start, 1.0);
    assert!(s.dragging);

    let tags = st.strip_released(0, 0);
    project(&mut st, tags);
    assert_eq!(st.strip_models[1].row_count(), 1);
    assert!(!st.strip_models[1].row_data(0).unwrap().dragging);
}

#[test]
fn vertical_drag_within_half_row_keeps_track() {
    let mut st = NlaEditorState::default();
    st.push_track("t0", vec![strip("A", 1.0, 4.0)], false);
    st.push_track("t1", vec![], false);

    let tags = st.strip_pressed(0, 0, SlintSelectionModifier::None);
    project(&mut st, tags);
    let tags = st.strip_dragged(0, 0, 0.0, 0.4);
    project(&mut st, tags); // 中心未越过边界 → 保持轨道 0
    let tags = st.strip_released(0, 0);
    project(&mut st, tags);

    assert_eq!(st.strip_models[0].row_count(), 1);
    assert_eq!(st.strip_models[1].row_count(), 0);
}

#[test]
fn horizontal_drag_clamped_to_avoid_overlap() {
    let mut st = NlaEditorState::default();
    st.push_track("t", vec![strip("A", 1.0, 4.0), strip("B", 5.0, 9.0)], false);

    // B 向左拖 3 秒：候选 [2,6] 与 A[1,4] 重叠，候选中点(4)在 A 中点(2.5)右侧 → 停 A 右缘 start=4
    let tags = st.strip_pressed(0, 1, SlintSelectionModifier::None);
    project(&mut st, tags);
    let tags = st.strip_dragged(0, 1, -3.0, 0.0);
    project(&mut st, tags);
    let s = st.strip_models[0].row_data(1).unwrap();
    assert_eq!(s.start, 4.0);
    assert_eq!(s.end, 8.0);

    let tags = st.strip_released(0, 1);
    project(&mut st, tags);
    assert_eq!(st.strip_models[0].row_data(1).unwrap().start, 4.0);
}

#[test]
fn horizontal_drag_keeps_order_on_right_side() {
    let mut st = NlaEditorState::default();
    st.push_track("t", vec![strip("A", 4.0, 7.0), strip("B", 0.0, 2.0)], false);

    // B 原在 A 左侧，向右拖：候选 [4,6] 与 A[4,7] 重叠，候选中点(5)在 A 中点(5.5)左侧 → 停 A 左缘 start=2
    st.strip_pressed(0, 1, SlintSelectionModifier::None);
    let tags = st.strip_dragged(0, 1, 4.0, 0.0);
    project(&mut st, tags);
    // reorder 后 B(start=2) 在 A(start=4) 前 → index 0
    let b = st.strip_models[0].row_data(0).unwrap();
    assert_eq!(b.name, "B");
    assert_eq!(b.start, 2.0);
    assert_eq!(b.end, 4.0);
}

#[test]
fn narrow_gap_is_not_filled() {
    let mut st = NlaEditorState::default();
    st.push_track(
        "t",
        vec![
            strip("A", 1.0, 3.0),
            strip("C", 6.0, 8.0),
            strip("B", 9.0, 14.0),
        ],
        false,
    );

    // B 宽 5，A-C 之间空隙 (3,6) 宽 3 < 5。B 向左拖进入该空隙 → 放不下，不塞入，
    // 保持片段当前 start=9
    st.strip_pressed(0, 2, SlintSelectionModifier::None);
    st.strip_dragged(0, 2, -5.0, 0.0); // 候选 [4,9] 在 A-C 之间
    let b = (0..st.strip_models[0].row_count())
        .find_map(|i| st.strip_models[0].row_data(i).filter(|s| s.name == "B"))
        .unwrap();
    assert_eq!(b.start, 9.0, "窄空隙放不下，片段应保持原位不被塞入");
    assert_eq!(b.end, 14.0);
}

#[test]
fn clamp_uses_candidate_side_not_original() {
    let mut st = NlaEditorState::default();
    st.push_track(
        "t",
        vec![strip("A", 10.0, 13.0), strip("B", 0.0, 2.0)],
        false,
    );

    // B 原在 A 左侧，但拖到 A 右侧附近（候选 [11,13]，中点 12 > A 中点 11.5）
    // → 停到 A **右缘** start=13（按候选当前位置判断，而非按下时的左侧）
    st.strip_pressed(0, 1, SlintSelectionModifier::None);
    let tags = st.strip_dragged(0, 1, 11.0, 0.0);
    project(&mut st, tags);
    let s = st.strip_models[0].row_data(1).unwrap();
    assert_eq!(s.start, 13.0);
    assert_eq!(s.end, 15.0);
}

#[test]
fn vertical_drag_cannot_move_into_action_or_locked_track() {
    let mut st = NlaEditorState::default();
    st.push_track("全局 Action", vec![strip("x", 0.0, 10.0)], true); // 轨道0 只读
    st.push_track("t1", vec![strip("A", 1.0, 4.0)], false); // 轨道1
    st.push_track("t2", vec![], false);
    st.toggle_track_locked(2); // 轨道2 锁定

    // 从轨道1 向上拖 → 目标轨道0（全局 Action，只读）→ 不移动
    let tags = st.strip_pressed(1, 0, SlintSelectionModifier::None);
    project(&mut st, tags);
    let tags = st.strip_dragged(1, 0, 0.0, -0.6);
    project(&mut st, tags);
    let tags = st.strip_released(1, 0);
    project(&mut st, tags);
    assert_eq!(st.strip_models[1].row_count(), 1);
    assert_eq!(st.strip_models[0].row_count(), 1, "只读轨不能接收片断");

    // 从轨道1 向下拖 → 目标轨道2（锁定）→ 不移动
    let tags = st.strip_pressed(1, 0, SlintSelectionModifier::None);
    project(&mut st, tags);
    let tags = st.strip_dragged(1, 0, 0.0, 0.6);
    project(&mut st, tags);
    let tags = st.strip_released(1, 0);
    project(&mut st, tags);
    assert_eq!(st.strip_models[1].row_count(), 1);
    assert_eq!(st.strip_models[2].row_count(), 0, "锁定轨不能接收片断");
}

#[test]
fn cross_track_move_horizontal_clamped_by_target_gaps() {
    let mut st = NlaEditorState::default();
    st.push_track("t0", vec![strip("A", 1.0, 4.0)], false);
    st.push_track("t1", vec![strip("C", 0.0, 3.0)], false);

    // 从轨道0 向下拖到轨道1：拖动中瞬间跨轨，候选 [1,4] 与 C[0,3] 重叠
    //（候选中点 2.5 > C 中点 1.5）→ 停 C 右缘 start=3
    let tags = st.strip_pressed(0, 0, SlintSelectionModifier::None);
    project(&mut st, tags);
    let tags = st.strip_dragged(0, 0, 0.0, 0.6);
    project(&mut st, tags);
    assert_eq!(st.strip_models[0].row_count(), 0, "拖动中已瞬间跨轨");
    let a = (0..st.strip_models[1].row_count())
        .find_map(|i| st.strip_models[1].row_data(i).filter(|s| s.name == "A"))
        .expect("A 应已移到轨道1");
    assert_eq!(a.start, 3.0);
    st.strip_released(0, 0);
    assert_eq!(st.strip_models[0].row_count(), 0);
}

/// 回归：水平穿越重排（d.strip 变化）后再跨轨——投影 remove 必须用拖动中的
/// 当前下标（非按下下标），否则移错片断导致两层错乱（误激活/拖不动/取消不了选中）。
#[test]
fn cross_track_after_horizontal_reorder_moves_correct_strip() {
    let mut st = NlaEditorState::default();
    st.push_track(
        "t0",
        vec![strip("A", 1.0, 4.0), strip("B", 5.0, 9.0)],
        false,
    );
    st.push_track("t1", vec![strip("C", 0.0, 3.0)], false);

    // 按下 A（下标 0）
    let tags = st.strip_pressed(0, 0, SlintSelectionModifier::None);
    project(&mut st, tags);

    // 水平穿越：A 拖到 [11,14]，reinsert 后 A 在 B 之后（当前下标 1）
    let tags = st.strip_dragged(0, 0, 10.0, 0.0);
    project(&mut st, tags);
    assert_eq!(st.editing_strips[0][1].view.name, "A", "A 已重排到下标 1");

    // 跨轨：按下下标(0) ≠ 当前下标(1)，投影 remove 必须用当前下标
    let tags = st.strip_dragged(0, 0, 10.0, 0.6);
    project(&mut st, tags);

    // 数据层与派生层一致：t0 只剩 B，t1 = [C, A]
    assert_eq!(st.editing_strips[0].len(), 1, "源轨只剩 B");
    assert_eq!(st.editing_strips[0][0].view.name, "B");
    assert_eq!(st.editing_strips[1].len(), 2);
    assert_eq!(
        st.editing_strips[1][0].view.name, "C",
        "C 在前（start=0 < 11）"
    );
    assert_eq!(st.editing_strips[1][1].view.name, "A");
    assert_eq!(st.strip_models[0].row_count(), 1, "派生层源轨同步");
    assert_eq!(st.strip_models[0].row_data(0).unwrap().name, "B");
    assert_eq!(st.strip_models[1].row_count(), 2, "派生层目标轨同步");
    assert_eq!(st.strip_models[1].row_data(0).unwrap().name, "C");
    assert_eq!(st.strip_models[1].row_data(1).unwrap().name, "A");
    assert_eq!(
        st.strip_models[1].row_data(1).unwrap().start,
        11.0,
        "A 水平位置保持"
    );
    assert!(
        st.strip_models[1].row_data(1).unwrap().dragging,
        "拖动标记跟随 A"
    );
    assert!(
        !st.strip_models[0].row_data(0).unwrap().dragging,
        "B 不受影响"
    );

    // 继续拖动（跨轨后按下下标仍回传 0，匹配 press 继续工作）
    let tags = st.strip_dragged(0, 0, 11.0, 0.6);
    project(&mut st, tags);
    let a = (0..st.strip_models[1].row_count())
        .find_map(|i| st.strip_models[1].row_data(i).filter(|s| s.name == "A"))
        .unwrap();
    assert_eq!(a.start, 12.0, "跨轨后仍可继续水平拖动");

    // 松开：清拖动标记，派生层同步
    let tags = st.strip_released(0, 0);
    project(&mut st, tags);
    let a = (0..st.strip_models[1].row_count())
        .find_map(|i| st.strip_models[1].row_data(i).filter(|s| s.name == "A"))
        .unwrap();
    assert!(!a.dragging, "松开后退出移动色");
    assert_eq!(st.editing_strips[1].len(), 2, "数据层与派生层保持一致");
}

// —— model 接线 ——
fn test_anim_data() -> AnimData {
    let mut data = AnimData::new("测试");
    data.base_action = Some(ActionId::default());
    let mut t1 = NlaTrack::new("Idle");
    let mut s1 = NlaStrip::new_action("Idle A", ActionId::default());
    s1.placement = StripPlacement::Absolute {
        start: 1.0,
        end: 5.0,
    };
    t1.strips = vec![s1];
    let mut t2 = NlaTrack::new("Run");
    let mut s2 = NlaStrip::new_action("奔跑", ActionId::default());
    s2.placement = StripPlacement::Full;
    t2.strips = vec![s2];
    data.tracks = vec![t1, t2];
    data
}

#[test]
fn load_from_model_builds_two_layer_state() {
    let mut st = NlaEditorState::default();
    let anim_data = test_anim_data();
    st.load_from_model(&anim_data, 10.0);
    // 派生层由投影构建（单测手动触发，集成测试走 depsgraph）
    st.apply_structure();

    // 数据层：第 0 轨全局 Action + 2 条 model 轨道
    assert_eq!(st.track_count(), 3);
    assert!(st.track_view[0].is_action);
    assert_eq!(st.track_view[1].name, "Idle");
    assert_eq!(st.track_view[2].name, "Run");
    assert_eq!(st.editing_strips[1].len(), 1);
    assert_eq!(st.editing_strips[2].len(), 1);

    // 派生层同步
    assert_eq!(st.tracks.row_count(), 3);
    assert_eq!(st.strip_models[1].row_count(), 1);
    let s = st.strip_models[1].row_data(0).unwrap();
    assert_eq!(s.name, "Idle A");
    assert_eq!(s.start, 1.0);
    assert_eq!(s.end, 5.0);
    // Full placement 按 target_duration=10 解析
    let s = st.strip_models[2].row_data(0).unwrap();
    assert_eq!(s.start, 0.0);
    assert_eq!(s.end, 10.0);
    // model 克隆保留（commit 时合并）
    assert_eq!(st.editing_strips[1][0].model.name, "Idle A");
    assert_eq!(st.anim_data_id, Some(anim_data.id));
}

#[test]
fn commit_to_model_updates_placement_and_muted() {
    let mut st = NlaEditorState::default();
    let mut anim_data = test_anim_data();
    st.load_from_model(&anim_data, 10.0);
    st.apply_structure();

    // 编辑：移动片断 + 翻转启用
    let tags = st.strip_pressed(1, 0, SlintSelectionModifier::None);
    project(&mut st, tags);
    let tags = st.strip_dragged(1, 0, 2.0, 0.0);
    project(&mut st, tags);
    let tags = st.toggle_track_enabled(1);
    project(&mut st, tags);
    st.commit_to_model(&mut anim_data);

    assert_eq!(
        anim_data.tracks[0].muted, true,
        "enabled=false ↔ muted=true"
    );
    assert_eq!(anim_data.tracks[0].strips.len(), 1);
    let s = &anim_data.tracks[0].strips[0];
    assert_eq!(
        s.placement,
        StripPlacement::Absolute {
            start: 3.0,
            end: 7.0
        }
    );
    // 未编辑字段保留
    assert_eq!(s.name, "Idle A");
    assert!(s.control_curves.influence.is_none());
}

#[test]
fn commit_to_model_preserves_unedited_track() {
    let mut st = NlaEditorState::default();
    let mut anim_data = test_anim_data();
    st.load_from_model(&anim_data, 10.0);
    st.apply_structure();

    st.commit_to_model(&mut anim_data);
    let s = &anim_data.tracks[1].strips[0];
    assert_eq!(
        s.placement,
        StripPlacement::Absolute {
            start: 0.0,
            end: 10.0
        },
        "Full 解析后提交为 Absolute"
    );
}

#[test]
fn strip_pick_maps_filter_row_to_authoritative() {
    let mut st = NlaEditorState::default();
    let mut anim_data = test_anim_data();
    st.load_from_model(&anim_data, 10.0);

    // 过滤后只剩 Idle 轨（过滤视图行 0 → 权威轨 1）
    st.filter_tracks("Idle");
    st.apply_structure();
    assert_eq!(st.tracks_filter.row_count(), 1);
    let (hit, tags) = st.strip_pick(3.0, 0, SlintSelectionModifier::None);
    assert_eq!(hit, 1 * 10000 + 0);
    project(&mut st, tags);
    assert!(st.strip_models[1].row_data(0).unwrap().selected);
}
