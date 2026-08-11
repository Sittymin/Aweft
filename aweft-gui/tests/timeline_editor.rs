//! 时间线编辑器回归测试：块（TimelineBlock）由"可见行"模型驱动渲染。
//! 验证核心性质：
//!   * 每个有有效时间范围（end-sec > start-sec）的可见行渲染一个块；
//!   * 块宽度 = (end - start) * 时间轴缩放（世界时间 → 像素定位）；
//!   * 折叠分组后其子行从模型消失 → 块随之消失（左侧列表与画布泳道共用可见模型）；
//!   * 根级收起后整条时间线折叠到第 0 条根级泳道，用全部字幕行片段块
//!     （timeline-root-blocks）渲染，与分组收起共用 TimelineSubBlocks。
use i_slint_backend_testing::{init_integration_test_with_system_time, ElementHandle};
use slint::ComponentHandle;

slint::slint! {
    import { SlintSubtitleRow, SlintSubtitleRowType, SlintSelectionState, SlintTimelineSubBlock } from "../ui/core/types.slint";
    import { TimelineEditor } from "../ui/areas/timeline/timeline_editor.slint";

    export component Harness inherits Window {
        width: 800px;
        height: 400px;
        in-out property <[SlintSubtitleRow]> rows;
        in-out property <[SlintTimelineSubBlock]> timeline-root-blocks;
        in-out property <bool> root-collapsed: false;
        in-out property <float> current-time: 3.0;

        TimelineEditor {
            rows: root.rows;
            timeline-root-blocks: root.timeline-root-blocks;
            root-collapsed <=> root.root-collapsed;
            current-time: root.current-time;
        }
    }
}

fn sub_block(id: &str, name: &str, start: f32, end: f32) -> SlintTimelineSubBlock {
    SlintTimelineSubBlock {
        item_id: id.into(),
        name: name.into(),
        start_sec: start,
        end_sec: end,
    }
}

fn group_row(id: &str, name: &str, start: f32, end: f32, collapsed: bool, count: i32) -> SlintSubtitleRow {
    group_row_with_blocks(id, name, start, end, collapsed, count)
}

fn group_row_with_blocks(id: &str, name: &str, start: f32, end: f32, collapsed: bool, count: i32) -> SlintSubtitleRow {
    SlintSubtitleRow {
        row_type: SlintSubtitleRowType::GroupHeader,
        depth: 0,
        ancestor_groups: slint::ModelRc::from(std::rc::Rc::new(slint::VecModel::from(Vec::<slint::SharedString>::new()))),
        start_time: "".into(),
        end_time: "".into(),
        start_sec: start,
        end_sec: end,
        child_blocks: slint::ModelRc::from(std::rc::Rc::new(slint::VecModel::from(vec![
            SlintTimelineSubBlock { item_id: "e1".into(), name: "你好".into(), start_sec: 2.0, end_sec: 3.0 },
            SlintTimelineSubBlock { item_id: "e2".into(), name: "再见".into(), start_sec: 4.0, end_sec: 6.0 },
        ]))),
        item_id: id.into(),
        selected: SlintSelectionState::Unselected,
        total_row_count: count,
        text: "".into(),
        visable: true,
        group_name: name.into(),
        collapsed,
    }
}

fn event_row(id: &str, text: &str, start: f32, end: f32) -> SlintSubtitleRow {
    event_row_at(id, text, start, end, 1)
}

fn event_row_at(id: &str, text: &str, start: f32, end: f32, depth: i32) -> SlintSubtitleRow {
    SlintSubtitleRow {
        row_type: SlintSubtitleRowType::Event,
        depth,
        ancestor_groups: slint::ModelRc::from(std::rc::Rc::new(slint::VecModel::from(Vec::<slint::SharedString>::new()))),
        start_time: "".into(),
        end_time: "".into(),
        start_sec: start,
        end_sec: end,
        child_blocks: slint::ModelRc::from(std::rc::Rc::new(slint::VecModel::from(Vec::<SlintTimelineSubBlock>::new()))),
        item_id: id.into(),
        selected: SlintSelectionState::Unselected,
        total_row_count: 1,
        text: text.into(),
        visable: true,
        group_name: "".into(),
        collapsed: false,
    }
}

#[test]
fn timeline_blocks_follow_visible_rows() {
    // 测试后端整个进程只能初始化一次，所有场景放同一个测试里。
    init_integration_test_with_system_time();

    slint::spawn_local(async move {
        let app = Harness::new().unwrap();

        // 分组头块覆盖组内所有行范围 [1,5]，两个字幕行块 [2,3] [4,6] → 3 块全可见
        // （根级展开时根级泳道为空条带，不额外产生块）
        app.set_rows(slint::ModelRc::from(std::rc::Rc::new(slint::VecModel::from(vec![
            group_row("g", "第一章", 1.0, 5.0, false, 3),
            event_row("e1", "你好", 2.0, 3.0),
            event_row("e2", "再见", 4.0, 6.0),
        ]))));
        app.set_timeline_root_blocks(slint::ModelRc::from(std::rc::Rc::new(slint::VecModel::from(vec![
            sub_block("e1", "你好", 2.0, 3.0),
            sub_block("e2", "再见", 4.0, 6.0),
        ]))));
        app.window().request_redraw();

        let blocks: Vec<_> = ElementHandle::find_by_element_type_name(&app, "TimelineBlock").collect();
        assert_eq!(blocks.len(), 3, "3 个有时间的可见行应渲染 3 块，得 {}", blocks.len());

        // 世界时间 → 像素：块宽度 = (end - start) * 缩放（默认 nav-zoom = 30）
        let w_group = blocks[0].size().width as i32;
        assert_eq!(w_group, 120, "组头块 [1,5] 宽 = 4s × 30px = 120，得 {w_group}");
        let w_event = blocks[2].size().width as i32;
        assert_eq!(w_event, 60, "字幕行块 [4,6] 宽 = 2s × 30px = 60，得 {w_event}");

        // 折叠分组：子行从可见模型消失 → 组头行渲染组内字幕行子块（合并到一行）
        app.set_rows(slint::ModelRc::from(std::rc::Rc::new(slint::VecModel::from(vec![
            group_row_with_blocks("g", "第一章", 1.0, 5.0, true, 3),
        ]))));
        app.set_timeline_root_blocks(slint::ModelRc::from(std::rc::Rc::new(slint::VecModel::from(vec![
            sub_block("e1", "你好", 2.0, 3.0),
            sub_block("e2", "再见", 4.0, 6.0),
        ]))));
        app.window().request_redraw();
        let folded = ElementHandle::find_by_element_type_name(&app, "TimelineBlock").count();
        assert_eq!(folded, 2, "折叠后组头泳道应渲染 2 个子块（[2,3] [4,6]），得 {folded}");

        // 根级收起：整条时间线折叠到第 0 条根级泳道，显示全部字幕行片段块；
        // 其余行隐藏（不可见元素不计数）
        app.set_root_collapsed(true);
        app.window().request_redraw();
        let root_folded = ElementHandle::find_by_element_type_name(&app, "TimelineBlock").count();
        assert_eq!(root_folded, 2, "根级收起后根级泳道应渲染 2 个字幕行块（[2,3] [4,6]），得 {root_folded}");
        app.set_root_collapsed(false);
        app.window().request_redraw();

        // 空时间范围（end <= start）的可见行不渲染块
        app.set_rows(slint::ModelRc::from(std::rc::Rc::new(slint::VecModel::from(vec![
            group_row("g", "空组", 0.0, 0.0, false, 0),
        ]))));
        app.set_timeline_root_blocks(slint::ModelRc::from(std::rc::Rc::new(slint::VecModel::from(Vec::<SlintTimelineSubBlock>::new()))));
        app.window().request_redraw();
        let none = ElementHandle::find_by_element_type_name(&app, "TimelineBlock").count();
        assert_eq!(none, 0, "空时间范围不应渲染块，得 {none}");

        // 折叠列随 depth 缩进：同 depth 时 group/event 文本左对齐，
        // 不同 depth 时文本随层级右移（紧贴组缩进）。
        // 列表第 0 行为根级栏，文本对齐断言取其后的分组行/事件行。
        // 同 depth(0)：group 与 event 文本必须同一 x
        app.set_rows(slint::ModelRc::from(std::rc::Rc::new(slint::VecModel::from(vec![
            group_row("g", "第一章", 1.0, 5.0, false, 2),
            event_row_at("e", "你好", 2.0, 3.0, 0),
        ]))));
        app.set_timeline_root_blocks(slint::ModelRc::from(std::rc::Rc::new(slint::VecModel::from(vec![
            sub_block("e", "你好", 2.0, 3.0),
        ]))));
        app.window().request_redraw();
        let texts: Vec<_> = ElementHandle::find_by_element_type_name(&app, "Text").collect();
        let xg = texts[1].absolute_position().x;
        let xe = texts[2].absolute_position().x;
        assert_eq!(xg as i32, xe as i32, "同 depth(0) 时 group/event 文本应左对齐");

        // 不同 depth：event(depth=1) 文本比 group(depth=0) 右移 16px（紧贴组缩进）
        app.set_rows(slint::ModelRc::from(std::rc::Rc::new(slint::VecModel::from(vec![
            group_row("g", "第一章", 1.0, 5.0, false, 2),
            event_row("e", "你好", 2.0, 3.0),
        ]))));
        app.set_timeline_root_blocks(slint::ModelRc::from(std::rc::Rc::new(slint::VecModel::from(vec![
            sub_block("e", "你好", 2.0, 3.0),
        ]))));
        app.window().request_redraw();
        let texts: Vec<_> = ElementHandle::find_by_element_type_name(&app, "Text").collect();
        let x0 = texts[1].absolute_position().x as i32;
        let x1 = texts[2].absolute_position().x as i32;
        assert_eq!(x1 - x0, 16, "不同 depth 时文本紧贴组缩进（右移 16px），得 group={x0} event={x1}");
    });
}

