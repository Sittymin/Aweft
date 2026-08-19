use anyhow::{Context, Result, anyhow};
use slint::{Model, ModelRc, VecModel};

use crate::bridge::slint_area_key_to_key;
use crate::bridge::types::{
    ScrEdgeTouchAreaData, ScrEdgeTouchAreaDisplayData, SlintKey, key_to_slint_key,
};
use crate::screen::edge::ScrEdgeKey;
use crate::screen::vertex::ScrVertKey;
use crate::store::AppStore;
use crate::store::state::AppState;
use std::collections::{HashMap, HashSet};

/// 内部边数据结构，包含计算出的所有信息
struct EdgeSegmentData {
    x: f32,
    y: f32,
    width: f32,
    height: f32,
    edge_keys: Vec<SlintKey>,
    vert_keys: Vec<SlintKey>,
}
#[derive(Hash, PartialEq, PartialOrd, Ord, Eq)]
struct EdgeAxis {
    is_horizontal: bool,
    // 与线段轴向垂直的偏移，水平情况下是 y 坐标
    offset: u32,
}
struct SegmentRef {
    start_pos: u32,
    end_pos: u32,
    // 合并后可能会有多个
    vert_keys: HashSet<ScrVertKey>,
    edge_keys: HashSet<ScrEdgeKey>,
}

/// 统一的边数据计算函数
fn compute_edges_data(state: &AppState) -> Vec<EdgeSegmentData> {
    // 用于合并同方向相连接的两个 Edge
    let mut grouped_segments: HashMap<EdgeAxis, Vec<SegmentRef>> = HashMap::new();

    for (edge_key, edge) in &state.screen_map.edges {
        if edge.border {
            continue;
        }

        let v1 = state
            .screen_map
            .verts
            .get(edge.v1)
            .unwrap_or_else(|| panic!("Edge: {:#?} 存在无效Vert: {:#?}", edge_key, edge.v1));
        let v2 = state
            .screen_map
            .verts
            .get(edge.v2)
            .unwrap_or_else(|| panic!("Edge: {:#?} 存在无效Vert: {:#?}", edge_key, edge.v2));

        let is_horizontal = v1.vec.y == v2.vec.y;
        let is_vertical = v1.vec.x == v2.vec.x;

        match (is_horizontal, is_vertical) {
            (true, false) => {
                let y = v1.vec.y;

                let (x1, x2, vert_keys) = if v1.vec.x <= v2.vec.x {
                    (v1.vec.x, v2.vec.x, HashSet::from([edge.v1, edge.v2]))
                } else {
                    (v2.vec.x, v1.vec.x, HashSet::from([edge.v1, edge.v2]))
                };

                grouped_segments
                    .entry(EdgeAxis {
                        is_horizontal,
                        offset: y,
                    })
                    .or_default()
                    .push(SegmentRef {
                        start_pos: x1,
                        end_pos: x2,
                        vert_keys,
                        edge_keys: HashSet::from([edge_key]),
                    });
            }
            (false, true) => {
                let x = v1.vec.x;
                let (y1, y2, vert_keys) = if v1.vec.y <= v2.vec.y {
                    (v1.vec.y, v2.vec.y, HashSet::from([edge.v1, edge.v2]))
                } else {
                    (v2.vec.y, v1.vec.y, HashSet::from([edge.v1, edge.v2]))
                };
                grouped_segments
                    .entry(EdgeAxis {
                        is_horizontal,
                        offset: x,
                    })
                    .or_default()
                    .push(SegmentRef {
                        start_pos: y1,
                        end_pos: y2,
                        vert_keys,
                        edge_keys: HashSet::from([edge_key]),
                    });
            }
            (false, false) => {
                panic!(
                    "存在非水平或垂直 Edge 数据: edge_key={:?}, v1=({}, {}), v2=({}, {})",
                    edge_key, v1.vec.x, v1.vec.y, v2.vec.x, v2.vec.y
                );
            }
            (true, true) => {
                panic!("Edge 的两个顶点重合，长度为零: edge_key={:?}", edge_key);
            }
        }
    }

    let mut result: Vec<EdgeSegmentData> = Vec::new();

    for (edge_axis, mut spans) in grouped_segments {
        // 在每个组内按范围合并有重合或相连接的段

        // 升序排序片段
        spans.sort_by(|a, b| match a.start_pos.cmp(&b.start_pos) {
            std::cmp::Ordering::Equal => a.end_pos.cmp(&b.end_pos),
            other => other,
        });

        let merged = merge_segments(spans);

        // 生成此轴向 UI 矩形数据放入到 result
        for seg in merged {
            let vert_keys_vec: Vec<SlintKey> =
                seg.vert_keys.iter().map(|k| key_to_slint_key(k)).collect();

            let edge_keys_vec: Vec<SlintKey> =
                seg.edge_keys.iter().map(|k| key_to_slint_key(k)).collect();

            let (x, y, width, height) = rect_for_edge(
                edge_axis.is_horizontal,
                seg.start_pos,
                seg.end_pos,
                edge_axis.offset,
                &state.theme_settings,
            );

            result.push(EdgeSegmentData {
                x,
                y,
                width,
                height,
                vert_keys: vert_keys_vec,
                edge_keys: edge_keys_vec,
            });
        }
    }

    result
}

/// 合并同一轴向（EdgeAxis）的共线段。
///
/// 输入按 `start_pos` 升序。合并规则：
/// - 当前段 `start <= prev.end`（重合或相接）：合并；
/// - 当前段 `start == prev.end + 1`（相差 1px，顶点坐标经分割/舍入可能产生
///   的相邻间隙）：**仍视为连续合并**——否则"几乎共线"的边会显示成两段；
/// - 否则提交 prev，开始新段。
fn merge_segments(spans: Vec<SegmentRef>) -> Vec<SegmentRef> {
    // 相邻容差：两段共线段端点相差 ≤1px 仍视为连续。
    const TOUCH_TOLERANCE: u32 = 1;

    let mut merged: Vec<SegmentRef> = Vec::new();
    let mut preview: Option<SegmentRef> = None;

    for span in spans {
        // 与上一个
        if let Some(mut preview_span) = preview {
            let gap = span.start_pos.saturating_sub(preview_span.end_pos);
            if span.start_pos > preview_span.end_pos && gap > TOUCH_TOLERANCE {
                // 中间存在分段且超出相邻容差（非连续，preview_span 没有
                // 继续可合并的部分了）
                merged.push(preview_span);
                preview = Some(span);
            } else {
                // 片段重合、相接或相差 ≤ 容差：合并
                preview_span.vert_keys.extend(span.vert_keys);
                preview_span.edge_keys.extend(span.edge_keys);
                preview = Some(SegmentRef {
                    start_pos: span.start_pos.min(preview_span.start_pos),
                    end_pos: span.end_pos.max(preview_span.end_pos),
                    vert_keys: preview_span.vert_keys,
                    edge_keys: preview_span.edge_keys,
                });
            }
        }
        // 第一个
        else {
            preview = Some(span);
        }
    }
    // 补上最后一个
    if let Some(seg) = preview {
        merged.push(seg);
    }
    merged
}

// 将传入的 edge_keys 当作整体生成 UI 数据
fn compute_segment_for_edges(
    state: &AppState,
    edge_keys: &[ScrEdgeKey],
) -> Result<(ScrEdgeTouchAreaDisplayData, ScrEdgeTouchAreaData)> {
    let mut start: u32;
    let mut end: u32;
    let mut vert_keys = HashSet::new();
    let edge_axis: EdgeAxis;
    let is_horizontal: bool;

    // 计算一些不需要在循环当中计算的数据
    {
        let first_edge_key = edge_keys.first().context("传入空数组")?;
        let edge = state
            .screen_map
            .edges
            .get(*first_edge_key)
            .unwrap_or_else(|| panic!("无效 EdgeKey: {:#?}", first_edge_key));
        let v1 =
            state.screen_map.verts.get(edge.v1).unwrap_or_else(|| {
                panic!("Edge: {:#?} 存在无效Vert: {:#?}", first_edge_key, edge.v1)
            });
        let v2 =
            state.screen_map.verts.get(edge.v2).unwrap_or_else(|| {
                panic!("Edge: {:#?} 存在无效Vert: {:#?}", first_edge_key, edge.v2)
            });
        is_horizontal = v1.vec.y == v2.vec.y;
        let is_vertical = v1.vec.x == v2.vec.x;
        if (is_horizontal && is_vertical) || (!is_horizontal && !is_vertical) {
            return Err(anyhow!("Edge: {:#?} 的点重叠或有斜向偏移", edge));
        }
        if is_horizontal {
            edge_axis = EdgeAxis {
                is_horizontal,
                offset: v1.vec.y,
            };
            start = v1.vec.x.min(v2.vec.x);
            end = v1.vec.x.max(v2.vec.x);
        } else {
            edge_axis = EdgeAxis {
                is_horizontal,
                offset: v1.vec.x,
            };
            start = v1.vec.y.min(v2.vec.y);
            end = v1.vec.y.max(v2.vec.y);
        }
    }

    for &edge_key in edge_keys {
        let edge = state
            .screen_map
            .edges
            .get(edge_key)
            .unwrap_or_else(|| panic!("无效 EdgeKey: {:#?}", edge_key));
        let v1 = state
            .screen_map
            .verts
            .get(edge.v1)
            .unwrap_or_else(|| panic!("Edge: {:#?} 存在无效Vert: {:#?}", edge_key, edge.v1));
        let v2 = state
            .screen_map
            .verts
            .get(edge.v2)
            .unwrap_or_else(|| panic!("Edge: {:#?} 存在无效Vert: {:#?}", edge_key, edge.v2));
        vert_keys.insert(edge.v1);
        vert_keys.insert(edge.v2);
        if is_horizontal {
            start = start.min(v1.vec.x.min(v2.vec.x));
            end = end.max(v1.vec.x.max(v2.vec.x));
        } else {
            start = start.min(v1.vec.y.min(v2.vec.y));
            end = end.max(v1.vec.y.max(v2.vec.y));
        }
    }

    let edge_keys_slint: Vec<_> = edge_keys.iter().map(|k| key_to_slint_key(k)).collect();
    let vert_keys_slint: Vec<_> = vert_keys.iter().map(|k| key_to_slint_key(k)).collect();

    let (x, y, width, height) = rect_for_edge(
        is_horizontal,
        start,
        end,
        edge_axis.offset,
        &state.theme_settings,
    );

    let edge_keys_model = ModelRc::new(VecModel::from(edge_keys_slint));
    let vert_keys_model = ModelRc::new(VecModel::from(vert_keys_slint));

    let display = ScrEdgeTouchAreaDisplayData {
        x,
        y,
        width,
        height,
        active: false,
        edge_keys: edge_keys_model.clone(),
    };
    let touch = ScrEdgeTouchAreaData {
        x,
        y,
        width,
        height,
        vert_keys: vert_keys_model,
        edge_keys: edge_keys_model,
    };
    Ok((display, touch))
}

// 计算 Edge UI 矩形（物理像素 → 逻辑像素，走 ThemeSettings::to_logical_px）
fn rect_for_edge(
    is_horizontal: bool,
    start: u32,
    end: u32,
    offset: u32,
    theme_settings: &crate::bridge::types::ThemeSettings,
) -> (f32, f32, f32, f32) {
    let half_edge_width = theme_settings.get_half_edge_width();
    // 边宽 = 2×半宽（物理→逻辑）+1 中心补偿。注意：这是唯一先加后转的
    // 逻辑转换（其余都走 to_logical_px）——因为 +1 补偿必须在 ÷scale 后、
    // 取整前加，保证边始终至少 1 逻辑像素可见。
    let edge_thickness = (half_edge_width * 2.0 / theme_settings.get_scale_factor() + 1.0) as f32;
    if is_horizontal {
        let width = theme_settings.to_logical_px(end.saturating_sub(start) as f64);
        let height = edge_thickness;
        let x = theme_settings.to_logical_px(start as f64);
        let y = theme_settings.to_logical_px(offset as f64 - half_edge_width);
        (x, y, width, height)
    } else {
        let height = theme_settings.to_logical_px(end.saturating_sub(start) as f64);
        let width = edge_thickness;
        let x = theme_settings.to_logical_px(offset as f64 - half_edge_width);
        let y = theme_settings.to_logical_px(start as f64);
        (x, y, width, height)
    }
}

/// 同时转换显示和触摸数据
/// 此函数会计算合并边（开销大）
pub fn convert_all_edges_both(
    state: &AppState,
) -> (Vec<ScrEdgeTouchAreaDisplayData>, Vec<ScrEdgeTouchAreaData>) {
    let edges_data = compute_edges_data(state);

    let display_data = edges_data
        .iter()
        .map(|edge| ScrEdgeTouchAreaDisplayData {
            x: edge.x,
            y: edge.y,
            width: edge.width,
            height: edge.height,
            active: false,
            edge_keys: ModelRc::new(VecModel::from(edge.edge_keys.clone())),
        })
        .collect();

    let touch_data = edges_data
        .into_iter()
        .map(|edge| ScrEdgeTouchAreaData {
            x: edge.x,
            y: edge.y,
            width: edge.width,
            height: edge.height,
            vert_keys: ModelRc::new(VecModel::from(edge.vert_keys)),
            edge_keys: ModelRc::new(VecModel::from(edge.edge_keys)),
        })
        .collect();

    (display_data, touch_data)
}

// 根据输入 edge_keys 找到需更新的行 index，并为每行重算数据
// HACK: TouchArea 版本的和 DisPlay 版本的应该索引一致吧？
pub fn recompute_edges_by_indices(
    store: &AppStore,
    keys: &[ScrEdgeKey],
) -> Vec<(usize, ScrEdgeTouchAreaDisplayData, ScrEdgeTouchAreaData)> {
    let ui = store.ui_models();
    let state = store.state();

    // 先用 map 找 index -> 该行持有的全部 edge_keys（用行里的 edge_keys，而不是输入 keys 合并）
    let mut idx_map: HashMap<usize, Vec<ScrEdgeKey>> = HashMap::new();
    for &k in keys {
        if let Some(&idx) = ui.scredge_touch_display_index_map.get(&k) {
            // 从行数据读取 edge_keys（合并后的真实集合）
            if let Some(row) = ui.scredge_displays().row_data(idx) {
                let row_edge_keys: Vec<ScrEdgeKey> = row
                    .edge_keys
                    .iter()
                    .map(|sk| slint_area_key_to_key(&sk))
                    .collect();
                idx_map.insert(idx, row_edge_keys);
            }
        }
    }

    let mut out = Vec::new();
    for (idx, edge_list) in idx_map {
        if let Ok((d, t)) = compute_segment_for_edges(state, &edge_list) {
            out.push((idx, d, t));
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    fn span(start: u32, end: u32) -> SegmentRef {
        SegmentRef {
            start_pos: start,
            end_pos: end,
            vert_keys: HashSet::new(),
            edge_keys: HashSet::new(),
        }
    }

    #[test]
    fn contiguous_segments_merge() {
        // 精确相接：0-10 + 10-20 → 0-20
        let merged = merge_segments(vec![span(0, 10), span(10, 20)]);
        assert_eq!(merged.len(), 1);
        assert_eq!(merged[0].start_pos, 0);
        assert_eq!(merged[0].end_pos, 20);
    }

    #[test]
    fn overlapping_segments_merge() {
        // 重合：0-15 + 10-20 → 0-20
        let merged = merge_segments(vec![span(0, 15), span(10, 20)]);
        assert_eq!(merged.len(), 1);
        assert_eq!(merged[0].start_pos, 0);
        assert_eq!(merged[0].end_pos, 20);
    }

    #[test]
    fn near_contiguous_with_1px_gap_merges() {
        // "几乎共线"：0-10 + 11-20（相差 1px）→ 合并为 0-20
        let merged = merge_segments(vec![span(0, 10), span(11, 20)]);
        assert_eq!(merged.len(), 1, "1px 间隙的共线段应合并");
        assert_eq!(merged[0].start_pos, 0);
        assert_eq!(merged[0].end_pos, 20);
    }

    #[test]
    fn separated_segments_do_not_merge() {
        // 明显分离：0-10 + 15-25（差 5px）→ 两段
        let merged = merge_segments(vec![span(0, 10), span(15, 25)]);
        assert_eq!(merged.len(), 2);
    }
}
