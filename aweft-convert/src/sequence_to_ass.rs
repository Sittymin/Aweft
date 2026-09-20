//! 核心序列数据 → 完整 ASS 脚本：视口预览与文件导出共用这一条转换。
//!
//! 预览把这里产出的 `ASSFile` 经 `ASSFile::to_bytes` 交给 libass，导出把同一份
//! `ASSFile` 写成文件。两条路出自同一个内存结构，字节不会分叉。

use std::collections::HashMap;

use aweft_ass::types::event::{Dialogue, Event, TextSegment};
use aweft_ass::types::style::StyleWithoutName;
use aweft_ass::types::tags::StyleTag;
use aweft_ass::types::time::AssTime;
use aweft_ass::types::ASSFile;
use aweft_model::application_graph::{ApplicationError, ApplicationGraphMatch, LineContext};
use aweft_model::ids::{ItemId, NodeGraphId};
use aweft_model::library::ProjectLibrary;
use aweft_model::node_graph::style::StyleResult;
use aweft_model::node_graph::EvalContext;
use aweft_model::project::Project;
use aweft_model::sequence::{EventData, ItemContent, Sequence};
use chrono::TimeDelta;

use crate::animate_to_ass::{match_to_ass_tags, ConvertOptions, TimedTags};

/// 所有生成行统一引用的样式名。逐行的静态样式不建独立 Style，而是以覆盖标签写进
/// 行文本：`match_to_ass_tags` 已把样式基线预置进每个段（animate_to_ass.rs:306）。
const DEFAULT_STYLE: &str = "Default";

/// 把项目的序列与样式层转成一份完整脚本。
///
/// `frame_size` 是渲染帧尺寸（视口侧取主视频的显示像素尺寸），写进
/// `[Script Info]` 的 PlayResX/PlayResY；libass 按它解释边距与对齐锚点。
pub fn sequence_to_ass(
    project: &mut Project,
    frame_size: (u32, u32),
    opts: &ConvertOptions,
) -> Result<ASSFile, ApplicationError> {
    // 拆开借用：sequence 全程只读，而 node_graphs 与 style_layer 求值时要写各自
    // 的求值缓存（NodeGraph::evaluate 与 ApplicationLayer::evaluate_matches 都收 &mut self）。
    let Project {
        sequence,
        library,
        style_layer,
        ..
    } = project;

    let style_results = evaluate_style_results(library)?;

    let mut ass = ASSFile::new();
    ass.script_info.layout_res_x = frame_size.0;
    ass.script_info.layout_res_y = frame_size.1;
    ass.styles
        .insert(DEFAULT_STYLE.to_string(), StyleWithoutName::default());

    for (index, event) in visible_events(sequence).into_iter().enumerate() {
        let duration = event.end_time - event.start_time;
        if duration <= TimeDelta::zero() {
            continue;
        }
        let duration_ms = duration.num_milliseconds() as f64;
        let start_ms = event.start_time.num_milliseconds() as f64;

        let matches = style_layer.evaluate_matches(
            &LineContext {
                actor: actor_name(library, event),
                line_text: event.text.clone(),
                line_index: index as i64,
            },
            &style_results,
        )?;
        let converted = match_to_ass_tags(
            &merge_matches(matches),
            &library.animation_library,
            frame_size,
            duration_ms / 1000.0,
            opts,
        );
        if !converted.fallbacks.is_empty() {
            log::warn!(
                "字幕行 {:?} 有 {} 处动画无法表达成 ASS 标签，这些帧不进脚本",
                event.text,
                converted.fallbacks.len()
            );
        }

        let text = ass_text(&event.text);
        let mut segments = converted.segments;
        if segments.is_empty() {
            // 既无样式基线也无动画时 match_to_ass_tags 返回空段（animate_to_ass.rs:331），
            // 补一个覆盖整行的空标签段，这一行才不会从脚本里消失。
            segments.push(TimedTags {
                start_ms: 0.0,
                end_ms: duration_ms,
                tags: Vec::new(),
            });
        }
        for segment in segments {
            ass.events.push(Event::Dialogue(dialogue(
                start_ms + segment.start_ms,
                start_ms + segment.end_ms,
                segment.tags,
                &text,
            )));
        }
    }

    Ok(ass)
}

/// 预先求值所有样式链：应用图的 AppRef 节点按被引用图的求值结果取样式
/// （aweft-model/src/application_graph.rs:228）。
fn evaluate_style_results(
    library: &mut ProjectLibrary,
) -> Result<HashMap<NodeGraphId, StyleResult>, ApplicationError> {
    let mut results = HashMap::new();
    for (id, graph) in &mut library.node_graphs {
        results.insert(*id, graph.evaluate(&EvalContext::style())?);
    }
    Ok(results)
}

/// 前序遍历取出参与渲染的字幕行，顺序与行编辑器一致。
///
/// 只按字幕行自身的 `visible` 过滤：分组的 `visible` 目前只作用于分组头那一行
/// 自己的显示（aweft-gui/src/store/sequence_rows_model.rs:643,674），不向子项传播。
fn visible_events(sequence: &Sequence) -> Vec<&EventData> {
    let mut out = Vec::new();
    collect_events(&sequence.root_items, sequence, &mut out);
    out
}

fn collect_events<'a>(ids: &'a [ItemId], sequence: &'a Sequence, out: &mut Vec<&'a EventData>) {
    for id in ids {
        let Some(item) = sequence.get(*id) else {
            continue;
        };
        match &item.content {
            ItemContent::Event(event) if item.visible => out.push(event),
            ItemContent::Group(group) => collect_events(&group.children, sequence, out),
            _ => {}
        }
    }
}

/// 把命中的多张应用图合并成逐行的一份样式加一个动画引用。
///
/// 叠加顺序同 `ApplicationLayer::evaluate`（后者覆盖前者）。动画只取最后命中的那个：
/// 两段动画同时驱动同一属性时，ASS 标签表达不出它们的合成结果。
fn merge_matches(matches: Vec<ApplicationGraphMatch>) -> ApplicationGraphMatch {
    let mut merged = ApplicationGraphMatch::default();
    for matched in matches {
        merged.style = merged.style.merge_over(matched.style);
        if matched.animation.is_some() {
            merged.animation = matched.animation;
        }
    }
    merged
}

fn actor_name(library: &ProjectLibrary, event: &EventData) -> String {
    event
        .actor
        .and_then(|id| library.actor_name(id))
        .unwrap_or_default()
        .to_string()
}

/// 换行必须转成 `\N`：Dialogue 是行式格式，正文里出现真实换行会把脚本切断。
fn ass_text(text: &str) -> String {
    text.replace('\n', "\\N")
}

/// 毫秒取整到厘秒——ASS 时间戳的精度就是厘秒（`H:MM:SS.CC`）。
///
/// 相邻段共享边界不出字两次也不留缝：libass 的事件判定是
/// `Start <= now < Start + Duration`（ass_render.c:3384），段的尾巴是开区间。
fn ass_time(ms: f64) -> AssTime {
    AssTime::from_centiseconds((ms / 10.0).round() as i64)
}

fn dialogue(start_ms: f64, end_ms: f64, tags: Vec<StyleTag>, text: &str) -> Dialogue {
    let mut content = Vec::with_capacity(2);
    if !tags.is_empty() {
        content.push(TextSegment::Overrides(tags));
    }
    content.push(TextSegment::Text(text.to_string()));

    Dialogue {
        layer: 0,
        start: ass_time(start_ms),
        end: ass_time(end_ms),
        style: DEFAULT_STYLE.to_string(),
        name: None,
        margin_l: 0,
        margin_r: 0,
        margin_v: 0,
        effect: None,
        content,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use aweft_model::sequence::ItemLocation;

    fn event(start_ms: i64, end_ms: i64, text: &str) -> EventData {
        EventData {
            start_time: TimeDelta::milliseconds(start_ms),
            end_time: TimeDelta::milliseconds(end_ms),
            actor: None,
            text: text.to_string(),
            segment_timing: None,
            style: String::new(),
        }
    }

    /// 走完整链路：序列 → ASSFile → 脚本字节。断言全部针对字节，
    /// 因为预览交给 libass 的就是这份字节。
    fn script_text(project: &mut Project) -> String {
        let opts = ConvertOptions::default();
        let ass = sequence_to_ass(project, (1920, 1080), &opts).unwrap();
        String::from_utf8(ass.to_bytes().unwrap()).unwrap()
    }

    fn dialogue_lines(text: &str) -> Vec<&str> {
        text.lines()
            .filter(|line| line.starts_with("Dialogue:"))
            .collect()
    }

    #[test]
    fn frame_size_and_row_times_reach_the_script() {
        let mut project = Project::new();
        project
            .sequence
            .create_event(event(1500, 4250, "第一行"), ItemLocation::Root(0))
            .unwrap();

        let text = script_text(&mut project);
        assert!(text.contains("PlayResX: 1920"), "{text}");
        assert!(text.contains("PlayResY: 1080"), "{text}");
        assert_eq!(
            dialogue_lines(&text),
            vec!["Dialogue:0,0:00:01.50,0:00:04.25,Default,,0,0,0,,第一行"],
        );
    }

    #[test]
    fn rows_are_collected_preorder_and_hidden_rows_are_out() {
        let mut project = Project::new();
        project
            .sequence
            .create_event(event(0, 1000, "根上"), ItemLocation::Root(0))
            .unwrap();
        let group = project
            .sequence
            .create_group(
                aweft_model::sequence::GroupData {
                    name: "组".to_string(),
                    collapsed: false,
                    color: None,
                    children: Vec::new(),
                },
                ItemLocation::Root(1),
            )
            .unwrap();
        project
            .sequence
            .create_event(event(1000, 2000, "组内"), ItemLocation::InGroup(group, 0))
            .unwrap();
        let hidden = project
            .sequence
            .create_event(event(2000, 3000, "隐藏"), ItemLocation::Root(2))
            .unwrap();
        project.sequence.get_mut(hidden).unwrap().visible = false;

        assert_eq!(
            dialogue_lines(&script_text(&mut project)),
            vec![
                "Dialogue:0,0:00:00.00,0:00:01.00,Default,,0,0,0,,根上",
                "Dialogue:0,0:00:01.00,0:00:02.00,Default,,0,0,0,,组内",
            ],
        );
    }

    #[test]
    fn default_style_scales_are_percent() {
        let mut project = Project::new();
        project
            .sequence
            .create_event(event(0, 1000, "一行"), ItemLocation::Root(0))
            .unwrap();

        let text = script_text(&mut project);
        let style = text
            .lines()
            .find(|line| line.starts_with("Style: "))
            .expect("脚本里没有 Style 行");
        // Format 顺序：Name,Fontname,Fontsize,四种颜色,Bold,Italic,Underline,
        // StrikeOut,ScaleX,ScaleY,... → ScaleX 下标 11、ScaleY 下标 12
        let fields: Vec<&str> = style.trim_start_matches("Style: ").split(',').collect();
        assert_eq!(fields.get(11), Some(&"100"), "{style}");
        assert_eq!(fields.get(12), Some(&"100"), "{style}");
    }

    #[test]
    fn newline_in_row_text_becomes_ass_line_break() {
        let mut project = Project::new();
        project
            .sequence
            .create_event(event(0, 1000, "上\n下"), ItemLocation::Root(0))
            .unwrap();

        assert_eq!(
            dialogue_lines(&script_text(&mut project)),
            vec![r"Dialogue:0,0:00:00.00,0:00:01.00,Default,,0,0,0,,上\N下"],
        );
    }
}
