//! HACK: 测试字幕数据的播种（临时演示数据，随真实工程/媒体流程落地后整块删除）。
//!
//! 与 `AppWindowManager::new` 里的调用顺序配套：浮动动画 → 节点图编辑器初始化 → 字幕行。
//! 这里只写权威 model，不碰派生层；派生层由随后的编辑器初始化统一投影。

use crate::store::AppStore;
use crate::store::node_graph::{new_app_chain, new_style_chain};
use aweft_model::animation::action::{Action, ActionCurve};
use aweft_model::animation::fcurve::FCurve;
use aweft_model::animation::keyframe::{BezierHandle, Keyframe};
use aweft_model::animation::strip::{NlaStrip, NlaTrack, StripType};
use aweft_model::animation::{AnimData, PropertyPath};
use aweft_model::application_graph::{ApplicationNodeHandle, ApplicationNodeKind, ApplyAnimation};
use aweft_model::node_graph::{Node, NodeId, SocketValue};

/// HACK: 给初始化测试字幕数据接一条「上下浮动」动画（与 `create_test_subtitle_data` 配套）。
///
/// 四段播种，全在权威 model，不涉及 ASS 概念：
///  `Action`（X 轴 `PositionRatioX` 常量居中、Y 轴 `PositionRatioY` 往复）
/// → `AnimData`（单轨单片断，`StripPlacement::Full` 把一次往复拉伸铺满每行自身时长）
/// → 样式链（默认样式，供应用动画取静态基线）
/// → 应用链（匹配角色空 pattern = 命中每一行 → 应用动画 → 组输出）。
///
/// 位置走「排版尺寸的分数」而不是排版像素：随视口等比缩放，和 `\move` 的像素单位解耦
/// （两种单位的换算点唯一在 `aweft_convert::animate_to_ass::Cursor::anchor_at`）。
///
/// 必须在 `initialize_node_graph_editors` 之前调用：链菜单、第 0 条链的装载与默认布局
/// 都从 model 派生，事后插入得手动重跑投影。
pub fn create_test_subtitle_animation(store: &mut AppStore) {
    let anim_data_id = {
        let library = &mut store.state.project.project_mut().library;

        // 往复一次 2 秒；手柄只横不纵，切线在每个关键帧上水平，端点处减速。
        let handle = 0.5 / 3.0;
        let ease = |time, value| {
            Keyframe::bezier(
                time,
                value,
                BezierHandle::new(-handle, 0.0),
                BezierHandle::new(handle, 0.0),
            )
        };
        let fcurve = |keyframes| FCurve {
            keyframes,
            ..Default::default()
        };
        let mut float_action = Action::new("上下浮动");
        // 往复一次的周期；片断按 Full 铺满每行时长，所以这个周期只定义波形。
        let cycle = 2.0;
        float_action.duration = cycle;
        float_action.curves = vec![
            // X 轴常量居中：位置两轴都要有值，缺省的 0 会把行贴到左边缘。
            ActionCurve::new(
                PropertyPath::PositionRatioX,
                fcurve(vec![ease(0.0, 0.5), ease(2.0, 0.5)]),
            ),
            ActionCurve::new(
                PropertyPath::PositionRatioY,
                fcurve(vec![
                    ease(0.0, 0.72),
                    ease(0.5, 0.60),
                    ease(1.0, 0.72),
                    ease(1.5, 0.60),
                    ease(2.0, 0.72),
                ]),
            ),
        ];
        let action_id = library.animation_library.insert_action(float_action);

        let mut track = NlaTrack::new("浮动");
        let mut strip = NlaStrip::new_action("上下浮动", action_id);
        // 显式给 action 时间窗：`new_action` 留的 0.0/0.0 在编脚本时是零宽窗口，
        // 整行会退化成一个静态快照（只有 NLA 采样路径经 `action_range()` 归一化）。
        if let StripType::Action {
            actstart, actend, ..
        } = &mut strip.strip_type
        {
            (*actstart, *actend) = (0.0, cycle);
        }
        track.strips = vec![strip];
        let mut anim_data = AnimData::new("字幕浮动");
        anim_data.tracks = vec![track];
        library.animation_library.insert_anim_data(anim_data)
    };

    let style_chain = new_style_chain("默认样式");
    let style_id = style_chain.id;
    store
        .state
        .project
        .project_mut()
        .library
        .node_graphs
        .insert(style_id, style_chain);

    // 复用默认应用链的形状（上下文输入 → 匹配角色 → 样式引用 → 应用匹配 → 组输出），
    // 再挂一个应用动画节点接进同一个组输出口。空 pattern 让匹配角色对每行都成立
    // （`str::contains("")` 恒真）。
    let mut graph = new_app_chain("字幕上下浮动", Some(style_id), Some(""));
    let apply: ApplicationNodeHandle<ApplyAnimation> = graph
        .add_node(ApplicationNodeKind::ApplyAnimation, &[], &[])
        .unwrap();
    let style_out = sole_node(&graph.nodes, |kind| {
        matches!(kind, ApplicationNodeKind::AppRef(_))
    })
    .outputs[0];
    graph.connect_raw(style_out, apply.inputs.style.id).unwrap();
    let matched = sole_node(&graph.nodes, |kind| {
        matches!(kind, ApplicationNodeKind::MatchActor)
    })
    .outputs[0];
    graph
        .connect_raw(matched, apply.inputs.condition.id)
        .unwrap();
    let group_out = sole_node(&graph.nodes, |kind| {
        matches!(kind, ApplicationNodeKind::GroupOutput)
    })
    .inputs[0];
    graph
        .connect_raw(apply.outputs.application.id, group_out)
        .unwrap();
    graph
        .set_socket_value(
            apply.inputs.animation.id,
            Some(SocketValue::Animation(anim_data_id)),
        )
        .unwrap();
    store
        .state
        .project
        .project_mut()
        .style_layer
        .graphs
        .push(graph);
}

/// 按谓词取图里唯一的那个节点。
fn sole_node<'a>(
    nodes: &'a std::collections::BTreeMap<NodeId, Node<ApplicationNodeKind>>,
    predicate: impl Fn(&ApplicationNodeKind) -> bool,
) -> &'a Node<ApplicationNodeKind> {
    nodes
        .values()
        .find(|node| predicate(&node.kind))
        .expect("默认应用链里有且只有一个这样的节点")
}

#[cfg(test)]
mod tests {
    use super::*;
    use aweft_model::sequence::{EventData, ItemLocation};
    use chrono::TimeDelta;

    /// 播种的四段链条要一路通到 ASS 出口：断言针对脚本字节（预览交给 libass 的就是这份），
    /// 每行都该带 `\move`，否则字幕仍钉在原地。
    #[test]
    fn seeded_chain_moves_every_row() {
        i_slint_backend_testing::init_no_event_loop();
        let instance = crate::bridge::types::MainWindow::new().unwrap();
        let mut store = AppStore::new(&instance);
        create_test_subtitle_animation(&mut store);

        let project = store.state.project.project_mut();
        project
            .sequence
            .create_event(
                EventData {
                    start_time: TimeDelta::milliseconds(0),
                    end_time: TimeDelta::milliseconds(5000),
                    actor: None,
                    text: "浮动一行".to_string(),
                    segment_timing: None,
                    style: String::new(),
                },
                ItemLocation::Root(0),
            )
            .unwrap();

        let ass = aweft_convert::sequence_to_ass(
            project,
            (1920, 1080),
            &aweft_convert::ConvertOptions::default(),
        )
        .unwrap();
        let script = String::from_utf8(ass.to_bytes().unwrap()).unwrap();
        let dialogues: Vec<&str> = script
            .lines()
            .filter(|line| line.starts_with("Dialogue:"))
            .collect();

        // 往复曲线会被拟合拆成多段，而 `\move` 每行只能生效一个，所以每段各成一行。
        assert!(
            dialogues.len() > 1,
            "浮动的一个周期应拆成多行，实际只拿到 {}: {script}",
            dialogues.len()
        );
        // 上限兜住拟合退化成「逐格切段」：那会一行一帧地刷屏，脚本体积爆掉。
        assert!(
            dialogues.len() <= 40,
            "拆行过多，拟合可能已退化：{} 行",
            dialogues.len()
        );
        let mut ys: Vec<f64> = Vec::new();
        for line in &dialogues {
            let (_, after) = line
                .split_once("\\move(")
                .unwrap_or_else(|| panic!("每行都该有 \\move: {line}"));
            let (args, _) = after.split_once(')').expect("\\move 应有右括号");
            let nums: Vec<f64> = args
                .split(',')
                .filter_map(|value| value.parse::<f64>().ok())
                .collect();
            // \move(x1,y1,x2,y2,t1,t2)
            assert_eq!(nums.len(), 6, "{line}");
            // X 常量 0.5 → 排版宽度中点：顺带钉住分数→像素的换算点。
            for &x in &[nums[0], nums[2]] {
                assert!((x - 960.0).abs() <= 1e-3, "x 应恒为排版宽度中点: {line}");
            }
            ys.extend([nums[1], nums[3]]);
        }
        let span = ys.iter().fold(f64::MIN, |acc, &y| acc.max(y))
            - ys.iter().fold(f64::MAX, |acc, &y| acc.min(y));
        assert!(
            span > 100.0,
            "端点纵向跨度应达排版高度的成（>100px），实际 {span}: {ys:?}"
        );
    }
}
