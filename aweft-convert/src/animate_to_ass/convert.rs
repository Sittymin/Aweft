//! strip / action 转换：把 NLA 动画数据切分成若干「段」。
//!
//! 「段」是拟合单位：每个段是一段能用单个幂函数 `\t`（或一条 `\move` 弦）在
//! 误差内表达的时间区间，DP 最小化的是拟合片段数。每个段映射为一条 ASS 行，
//! 因为 libass 中位置标签（`\move`/`\pos`）每行只能生效一个，后续的会被忽略；
//! 而 `\t` 本可以在同一行内连续链式表达——把无位置的相邻段合并进同一行属于
//! 可选的后续优化，不影响正确性。
//!
//! 分段本身委托给 crate 级的通用算法 [`segmentation`](crate::segmentation)，
//! 这里只负责「某段时间区间能否一次拟合」以及「把切分路径映射成标签段」。
//!
//! [`segmentation`]: crate::segmentation

use std::collections::HashMap;

use aweft_ass::types::tags::StyleTag;
use aweft_model::animation::action::ActionCurve;
use aweft_model::animation::anim_data::PropertyPath;
use aweft_model::animation::fcurve::evaluate_fcurve;
use aweft_model::animation::keyframe::{merge_keyframe_times, Keyframe};
use aweft_model::animation::strip::{NlaStrip, StripType};
use aweft_model::animation::{AnimData, AnimationLibrary};
use aweft_model::ids::ActionId;
use aweft_model::node_graph::style::StyleResult;

use super::{
    AnimAssResult, ConvertOptions, Cursor, ProgressDetail, ProgressEvent, TimeMapper, TimedTags,
    curve_global_range, has_modifiers, try_build_segment,
};
use crate::segmentation::{
    UniformGrid, force_single_step_path, min_segmentation, min_segmentation_traced,
    reconstruct_path,
};

type FallbackFrames = Vec<(f64, Vec<StyleTag>)>;

pub fn convert_anim_data(
    _base_style: &StyleResult,
    cursor: &mut Cursor,
    anim_data: &AnimData,
    library: &AnimationLibrary,
    duration: f64,
    opts: &ConvertOptions,
) -> AnimAssResult {
    let mut segments = Vec::new();
    let mut fallbacks = Vec::new();

    let has_solo = anim_data.tracks.iter().any(|t| t.solo);

    let mut ordered: Vec<(&NlaStrip, f64, f64)> = anim_data
        .tracks
        .iter()
        .flat_map(|track| {
            track
                .strips
                .iter()
                .map(move |s| (s, track.muted, track.solo))
        })
        .filter(|(s, muted, solo)| !s.muted && !muted && !(has_solo && !solo))
        .map(|(s, _, _)| {
            let (st, en) = s.placement.resolve(duration);
            (s, st, en)
        })
        .collect();
    ordered.sort_by(|a, b| a.1.partial_cmp(&b.1).unwrap_or(std::cmp::Ordering::Equal));

    for (strip, strip_start, strip_end) in &ordered {
        match convert_strip(strip, *strip_start, *strip_end, library, cursor, opts) {
            Ok(mut s) => segments.append(&mut s),
            Err(fb) => fallbacks.extend(fb),
        }
    }

    AnimAssResult {
        segments,
        fallbacks,
    }
}

fn convert_strip(
    strip: &NlaStrip,
    strip_start: f64,
    strip_end: f64,
    library: &AnimationLibrary,
    cursor: &mut Cursor,
    opts: &ConvertOptions,
) -> Result<Vec<TimedTags>, FallbackFrames> {
    match &strip.strip_type {
        StripType::Action {
            action,
            actstart,
            actend,
        } => convert_action_strip(
            *action,
            *actstart,
            *actend,
            strip_start,
            strip_end,
            library,
            cursor,
            opts,
        ),
        // TODO: 衔接也需要处理
        StripType::Transition => Ok(Vec::new()),
        StripType::Meta { strips } => {
            let mut all = Vec::new();
            for child in strips {
                if child.muted {
                    continue;
                }
                let (cs, ce) = child.placement.resolve(strip_end - strip_start);
                match convert_strip(
                    child,
                    strip_start + cs,
                    strip_start + ce,
                    library,
                    cursor,
                    opts,
                ) {
                    Ok(mut s) => all.append(&mut s),
                    Err(fb) => return Err(fb),
                }
            }
            Ok(all)
        }
    }
}

/// 转换单个 action strip：在 action 时间轴上均匀采样格点，用通用最短路径算法
/// 找出「误差达标的最少段数」，再把每段映射成 ASS 标签段。
fn convert_action_strip(
    action_id: ActionId,
    actstart: f64,
    actend: f64,
    strip_start: f64,
    strip_end: f64,
    library: &AnimationLibrary,
    cursor: &mut Cursor,
    opts: &ConvertOptions,
) -> Result<Vec<TimedTags>, FallbackFrames> {
    let action = match library.get_action(action_id) {
        Some(a) => a,
        None => return Ok(Vec::new()),
    };

    // 与 model 采用相同的默认值：若 actend <= actstart，则使用 actstart + action.duration
    let actend = if actend <= actstart {
        actstart + action.duration.max(1e-9)
    } else {
        actend
    };
    let action_dur = (actend - actstart).max(1e-9);
    let strip_dur = (strip_end - strip_start).max(1e-9);
    let time = TimeMapper {
        actstart,
        action_dur,
        strip_start,
        strip_dur,
    };

    let curve_map: HashMap<PropertyPath, &ActionCurve> = action
        .curves
        .iter()
        .map(|curve| (curve.property.clone(), curve))
        .collect();

    // 收集所有动画曲线的关键帧时间，裁到 action 时间窗内并补上边界。
    let Some(times) = collect_segment_times(&action.curves, actstart, actend) else {
        return Ok(Vec::new());
    };

    let tolerance = opts.error_tolerance;
    // 更大的 n 会迅速增加计算成本（N^2），如果需要更密就需要想算法
    let n = times.len().saturating_mul(16).clamp(256, 1024);
    let t_start = times[0];
    let t_end = times[times.len() - 1];
    let span = t_end - t_start;
    let grid = UniformGrid::new(t_start, t_end, n);

    // 每个动画属性在整条 strip 时间跨度上的全局值域。拟合以该值域为基准判断，
    // 因此落在局部极值上的段（段内 (v1-v0) ≈ 0）不会被强制切成细条。
    let granges: HashMap<PropertyPath, f64> = curve_map
        .iter()
        .map(|(prop, curve)| (prop.clone(), curve_global_range(&curve.curve, t_start, t_end)))
        .collect();

    // Action 时长为 0，或所有关键帧都在一个点：直接输出单个瞬间快照，
    // 避免无效的零宽 DP 计算。
    if span <= 1e-12 {
        let snapshot =
            single_snapshot_segment(t_start, t_end, &curve_map, &granges, &time, cursor);
        advance_cursor_to_end(&curve_map, cursor, t_end);
        return Ok(snapshot);
    }

    // 探测用基 cursor 的副本：颜色会就地写回副本，不影响真实链路；位置只读基值。
    log::info!("DP segmenting action {action_id} (n={n}, tolerance={tolerance})...");
    let is_valid = |start: usize, end: usize| {
        let mut probe = cursor.clone();
        let (_, diagnostics) = try_build_segment(
            grid.time_at(start),
            grid.time_at(end),
            &curve_map,
            &granges,
            &time,
            &mut probe,
        );
        diagnostics.max_err <= tolerance
    };

    let mut parent = match &opts.progress_sender {
        None => min_segmentation(grid.len(), is_valid, |_, _| {}),
        Some(sender) => {
            // 曲线图取全局摆幅最大的那条通道：读第一条的话，一个只有 Y 在动的上下浮动
            // 会画成一根平线，图上根本看不出拟合的是什么。
            let busiest = curve_map
                .iter()
                .max_by(|(_, ac_a), (_, ac_b)| {
                    granges[&ac_a.property].total_cmp(&granges[&ac_b.property])
                })
                .map(|(_, ac)| &ac.curve);
            let curve_samples: Vec<(f64, f64)> = (0..=128)
                .map(|i| {
                    let t = t_start + (i as f64 / 128.0) * span;
                    let v = busiest.as_ref().map_or(0.0, |ac| evaluate_fcurve(ac, t));
                    (t, v)
                })
                .collect();
            let grid_times: Vec<f64> = (0..=grid.len()).map(|i| grid.time_at(i)).collect();
            let _ = sender.send(ProgressEvent::ActionStarted {
                action_id,
                name: action.name.clone(),
                n: grid.len(),
                grid_times,
                curve_samples,
            });
            let n = grid.len();
            let s = sender.clone();
            let diagnostic = opts.progress_detail == ProgressDetail::Diagnostic;
            if diagnostic {
                min_segmentation_traced(n, is_valid, move |end, valid, chosen| {
                    let _ = s.send(ProgressEvent::DpProgress {
                        end,
                        n,
                        chosen,
                        valid: valid.to_vec(),
                    });
                })
            } else {
                min_segmentation(n, is_valid, move |end, chosen| {
                    let _ = s.send(ProgressEvent::DpProgress {
                        end,
                        n,
                        chosen,
                        valid: Vec::new(),
                    });
                })
            }
        }
    };
    if parent[n] == usize::MAX {
        log::warn!(
            "action {action_id}: no segmentation within tolerance {tolerance} over span {span:.6}s; \
             degrading to per-grid segments (precision may exceed tolerance)"
        );
        force_single_step_path(&mut parent);
    }
    // 收尾路径必须在兜底之后发：不可达时 `parent` 里还是 `usize::MAX`，
    // `reconstruct_path` 会拿它当下标直接越界 panic。
    if let Some(sender) = &opts.progress_sender {
        let _ = sender.send(ProgressEvent::SegmentsFound {
            path: reconstruct_path(&parent),
        });
    }

    let path = reconstruct_path(&parent);
    let segments = build_segments_from_path(&path, &grid, &curve_map, &granges, &time, cursor);
    advance_cursor_to_end(&curve_map, cursor, t_end);
    Ok(group_into_lines(segments))
}

/// 把拟合段合并成 ASS 行，使行数理论最小。
///
/// libass 里位置标签（`\move`/`\pos`）每行只能生效一个，而 `\t` 可以在同一行内
/// 连续链式表达。因此：没有位置动画 → 1 行；否则行数 = 位置段数。其余无位置的
/// 拟合段被并入就近的行，只追加其 `\t` 变换（行首段已设置初始静态值，再补静态
/// 标签会在行首错误地覆盖它）。
///
/// 行内标签使用 strip 绝对时间（与 `\move` 一致），序列化时按行起点换算成相对时间。
fn group_into_lines(segments: Vec<TimedTags>) -> Vec<TimedTags> {
    let mut lines: Vec<TimedTags> = Vec::new();
    let mut current: Option<TimedTags> = None;
    for segment in segments {
        let segment_has_move = segment
            .tags
            .iter()
            .any(|tag| matches!(tag, StyleTag::Move { .. }));
        let line_has_move = current
            .as_ref()
            .map(|line| line.tags.iter().any(|tag| matches!(tag, StyleTag::Move { .. })))
            .unwrap_or(false);

        // 行内最多一个 \move：当前行已有 \move 而这段又要 \move → 该段另起一行。
        if line_has_move && segment_has_move {
            lines.push(current.take().unwrap());
        }

        let line = match current.as_mut() {
            None => {
                current = Some(segment);
                continue;
            }
            Some(line) => line,
        };
        // 合并：只追加 \t 变换与本行尚未拥有的 \move；静态标签已由行首段设置，
        // 再补会在行首错误地覆盖初始值。
        line.end_ms = segment.end_ms;
        for tag in segment.tags {
            match tag {
                StyleTag::Transform { .. } => line.tags.push(tag),
                StyleTag::Move { .. } if !line_has_move => line.tags.push(tag),
                _ => {}
            }
        }
    }
    if let Some(line) = current {
        lines.push(line);
    }
    lines
}

/// 收集所有无修饰动画曲线的关键帧时间点，裁剪到 action 时间窗 `[actstart, actend]`
/// 内，并补上窗口边界。返回 None 表示没有可用的动画内容。
fn collect_segment_times(
    curves: &[ActionCurve],
    actstart: f64,
    actend: f64,
) -> Option<Vec<f64>> {
    // 合并所有无修饰动画曲线的关键帧时间点
    // HACK: 似乎目前还没有处理 Modifier 的情况
    let all_kfs: Vec<&[Keyframe]> = curves
        .iter()
        .filter(|curve| !has_modifiers(&curve.curve))
        .map(|curve| curve.curve.keyframes.as_slice())
        .collect();
    if all_kfs.iter().all(|k| k.is_empty()) {
        return None;
    }

    // 获得各关键帧时间点
    let merged = merge_keyframe_times(&all_kfs);
    if merged.len() < 2 {
        return None;
    }

    // 剔除在时间范围外的关键帧
    // TODO: 可能应该内部添加啥的就自动保证，而不是在外部检查
    let mut times: Vec<f64> = merged
        .into_iter()
        .filter(|&t| t >= actstart - 1e-9 && t <= actend + 1e-9)
        .collect();
    if times.len() < 2 {
        return None;
    }
    if (times[0] - actstart).abs() > 1e-9 {
        times.insert(0, actstart);
    }
    if (times[times.len() - 1] - actend).abs() > 1e-9 {
        times.push(actend);
    }
    Some(times)
}

/// strip 走完才推进 cursor：把终点时刻解析出的锚点折成像素基值，下一条 strip 从这里
/// 接着走。这条 strip 没有位置曲线时不动 cursor——位置槽原样继承，折一次会把样式的
/// 分数基值永久变成像素偏移。
fn advance_cursor_to_end(
    curve_map: &HashMap<PropertyPath, &ActionCurve>,
    cursor: &mut Cursor,
    t_end: f64,
) {
    let axes = Cursor::position_curves(curve_map);
    if axes
        .iter()
        .all(|axis| axis.offset.is_none() && axis.ratio.is_none())
    {
        return;
    }
    cursor.advance_to(cursor.anchor_at(axes, t_end));
}

/// 把切分路径（格点对序列）转换成最终的时间段 + 标签段列表。
fn build_segments_from_path(
    path: &[(usize, usize)],
    grid: &UniformGrid,
    curve_map: &HashMap<PropertyPath, &ActionCurve>,
    granges: &HashMap<PropertyPath, f64>,
    time: &TimeMapper,
    cursor: &mut Cursor,
) -> Vec<TimedTags> {
    let mut result = Vec::new();
    for (start, end) in path {
        let t0 = grid.time_at(*start);
        let t1 = grid.time_at(*end);
        let (tags, _) = try_build_segment(t0, t1, curve_map, granges, time, cursor);
        if !tags.is_empty() {
            result.push(TimedTags {
                start_ms: time.to_strip(t0) * 1000.0,
                end_ms: time.to_strip(t1) * 1000.0,
                tags,
            });
        }
    }
    result
}

/// 零宽动作：构建单个瞬间快照段（若没有任何标签则返回空）。
fn single_snapshot_segment(
    t0: f64,
    t1: f64,
    curve_map: &HashMap<PropertyPath, &ActionCurve>,
    granges: &HashMap<PropertyPath, f64>,
    time: &TimeMapper,
    cursor: &mut Cursor,
) -> Vec<TimedTags> {
    let (tags, _) = try_build_segment(t0, t1, curve_map, granges, time, cursor);
    if tags.is_empty() {
        Vec::new()
    } else {
        vec![TimedTags {
            start_ms: time.to_strip(t0) * 1000.0,
            end_ms: time.to_strip(t1) * 1000.0,
            tags,
        }]
    }
}
