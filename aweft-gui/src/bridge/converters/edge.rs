use crate::bridge::types::{key_to_slint_area_key, ScrEdgeDisplayData, ScrEdgeTouchAreaData, SlintKey};
use crate::screen::vertex::ScrVertKey;
use crate::store::state::AppState;
use std::collections::{HashMap, HashSet};

/// 内部边数据结构，包含计算出的所有信息
struct EdgeSegmentData {
    x: f32,
    y: f32,
    width: f32,
    height: f32,
    vert_keys: Vec<SlintKey>,
}

/// 统一的边数据计算函数
fn compute_edges_data(state: &AppState) -> Vec<EdgeSegmentData> {
    // 1) 先将所有非边框的、轴对齐的边转换为"线段"并去重
    let mut seen_segments: HashSet<(bool, u32, u32, u32)> = HashSet::new();
    let mut grouped_segments: HashMap<(bool, u32), Vec<(u32, u32, ScrVertKey, ScrVertKey)>> =
        HashMap::new();

    for (edge_key, edge) in &state.screen_map.edges {
        if edge.border {
            continue;
        }
        let v1 = state.screen_map.verts.get(edge.v1).unwrap_or_else(|| {
            panic!("Edge: {:#?} 存在无效Vert: {:#?}", edge_key, edge.v1)
        });
        let v2 = state.screen_map.verts.get(edge.v2).unwrap_or_else(|| {
            panic!("Edge: {:#?} 存在无效Vert: {:#?}", edge_key, edge.v2)
        });

        let is_horizontal = v1.vec.y == v2.vec.y;
        let is_vertical = v1.vec.x == v2.vec.x;

        match (is_horizontal, is_vertical) {
            (true, false) => {
                let y = v1.vec.y;
                let (x1, x2, ks, ke) = if v1.vec.x <= v2.vec.x {
                    (v1.vec.x, v2.vec.x, edge.v1, edge.v2)
                } else {
                    (v2.vec.x, v1.vec.x, edge.v2, edge.v1)
                };
                if seen_segments.insert((true, y, x1, x2)) {
                    grouped_segments
                        .entry((true, y))
                        .or_default()
                        .push((x1, x2, ks, ke));
                }
            }
            (false, true) => {
                let x = v1.vec.x;
                let (y1, y2, ks, ke) = if v1.vec.y <= v2.vec.y {
                    (v1.vec.y, v2.vec.y, edge.v1, edge.v2)
                } else {
                    (v2.vec.y, v1.vec.y, edge.v2, edge.v1)
                };
                if seen_segments.insert((false, x, y1, y2)) {
                    grouped_segments
                        .entry((false, x))
                        .or_default()
                        .push((y1, y2, ks, ke));
                }
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

    // 2) 在每个组内按范围合并相邻/重叠的段
    let half_edge_width = state.ui_settings.get_half_edge_width();
    let scale_factor = state.ui_settings.get_scale_factor();

    let mut result: Vec<EdgeSegmentData> = Vec::new();

    for ((is_horizontal, constant), mut spans) in grouped_segments {
        spans.sort_by(|a, b| match a.0.cmp(&b.0) {
            std::cmp::Ordering::Equal => a.1.cmp(&b.1),
            other => other,
        });

        let mut merged: Vec<(u32, u32, Vec<ScrVertKey>)> = Vec::new();
        let mut current: Option<(u32, u32, Vec<ScrVertKey>)> = None;

        for (start, end, ks, ke) in spans {
            if let Some((cur_s, mut cur_e, mut keys)) = current {
                use std::cmp::Ordering;
                match start.cmp(&cur_e) {
                    Ordering::Greater => {
                        merged.push((cur_s, cur_e, keys));
                        current = Some((start, end, vec![ks, ke]));
                    }
                    Ordering::Equal | Ordering::Less => {
                        if end > cur_e {
                            cur_e = end;
                        }
                        keys.push(ks);
                        keys.push(ke);
                        current = Some((cur_s, cur_e, keys));
                    }
                }
            } else {
                current = Some((start, end, vec![ks, ke]));
            }
        }
        if let Some(seg) = current {
            merged.push(seg);
        }

        // 3) 生成UI矩形数据（逻辑像素坐标）
        for (start, end, mut keys) in merged {
            keys.sort_by_key(|k| {
                let v = state.screen_map.verts.get(*k).unwrap();
                if is_horizontal {
                    v.vec.x
                } else {
                    v.vec.y
                }
            });
            keys.dedup();
            let vert_keys_vec: Vec<SlintKey> = keys
                .into_iter()
                .map(|k| key_to_slint_area_key(k))
                .collect();

            if is_horizontal {
                let width = ((end - start) as f64 / scale_factor) as f32;
                let height = (((half_edge_width * 2.0) / scale_factor) + 1.0) as f32;
                let x = (start as f64 / scale_factor) as f32;
                let y = ((constant as f64 - half_edge_width) / scale_factor) as f32;
                result.push(EdgeSegmentData {
                    x,
                    y,
                    width,
                    height,
                    vert_keys: vert_keys_vec,
                });
            } else {
                let height = ((end - start) as f64 / scale_factor) as f32;
                let width = (((half_edge_width * 2.0) / scale_factor) + 1.0) as f32;
                let x = ((constant as f64 - half_edge_width) / scale_factor) as f32;
                let y = (start as f64 / scale_factor) as f32;
                result.push(EdgeSegmentData {
                    x,
                    y,
                    width,
                    height,
                    vert_keys: vert_keys_vec,
                });
            }
        }
    }

    result
}

/// 转换为显示数据
pub fn convert_edges_display(state: &AppState) -> Vec<ScrEdgeDisplayData> {
    compute_edges_data(state)
        .into_iter()
        .map(|edge| ScrEdgeDisplayData {
            x: edge.x,
            y: edge.y,
            width: edge.width,
            height: edge.height,
            active: false,
        })
        .collect()
}

/// 转换为触摸区域数据
pub fn convert_edges_touch(state: &AppState) -> Vec<ScrEdgeTouchAreaData> {
    compute_edges_data(state)
        .into_iter()
        .map(|edge| ScrEdgeTouchAreaData {
            x: edge.x,
            y: edge.y,
            width: edge.width,
            height: edge.height,
            vert_keys: edge.vert_keys.as_slice().into(),
        })
        .collect()
}

/// 同时转换显示和触摸数据
pub fn convert_edges_both(state: &AppState) -> (Vec<ScrEdgeDisplayData>, Vec<ScrEdgeTouchAreaData>) {
    let edges_data = compute_edges_data(state);

    let display_data = edges_data
        .iter()
        .map(|edge| ScrEdgeDisplayData {
            x: edge.x,
            y: edge.y,
            width: edge.width,
            height: edge.height,
            active: false,
        })
        .collect();

    let touch_data = edges_data
        .into_iter()
        .map(|edge| ScrEdgeTouchAreaData {
            x: edge.x,
            y: edge.y,
            width: edge.width,
            height: edge.height,
            vert_keys: edge.vert_keys.as_slice().into(),
        })
        .collect();

    (display_data, touch_data)
}


