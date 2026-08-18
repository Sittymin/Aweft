use super::CurveEditorState;
use crate::bridge::types::{SlintBezierHandle, SlintInterpolation, SlintInterpolationState};
use crate::depsgraph::Tag;

impl CurveEditorState {
    /// 设置全部选中关键帧的插值类型。选中集为空则忽略。
    /// 插值变化会改变手柄数量，需要全量重建。
    pub fn set_selected_keyframes_interpolation(&mut self, interpolation: SlintInterpolation) -> Vec<Tag> {
        // 插值变更影响"选中关键帧"：先收集受影响通道（数据层）
        let curves: Vec<usize> = self
            .editing_keyframes
            .iter()
            .filter(|(_, kfs)| kfs.iter().any(|k| k.selected))
            .map(|(&curve, _)| curve)
            .collect();

        // 第一步：线性/常量 → 贝塞尔的关键帧补默认控制柄（原控制柄为 0，否则看不见也拖不动）
        if interpolation == SlintInterpolation::Bezier {
            for curve in &curves {
                let Some(kfs) = self.editing_keyframes.get(curve) else { continue };
                // 按时间排序（模型下标, 时间），用于给新贝塞尔关键帧生成默认控制柄
                let mut sorted: Vec<(usize, f32)> = kfs.iter().enumerate().map(|(i, k)| (i, k.time)).collect();
                sorted.sort_by(|a, b| a.1.partial_cmp(&b.1).unwrap_or(std::cmp::Ordering::Equal));
                let updates: Vec<(usize, SlintBezierHandle, SlintBezierHandle)> = kfs
                    .iter()
                    .enumerate()
                    .filter(|(_, kf)| kf.selected && kf.interpolation != interpolation)
                    .filter_map(|(i, kf)| {
                        let pos = sorted.iter().position(|(mi, _)| *mi == i)?;
                        let t = sorted[pos].1;
                        let prev_t = if pos > 0 { sorted[pos - 1].1 } else { t };
                        let next_t = if pos + 1 < sorted.len() { sorted[pos + 1].1 } else { t };
                        let mut handle_in = kf.handle_in.clone();
                        // 入柄：仅有左侧段是贝塞尔时才会显示；这里预置，右侧邻居切贝塞尔时即可用
                        if pos > 0 && handle_in.time_offset == 0.0 && handle_in.value_offset == 0.0 {
                            handle_in = SlintBezierHandle {
                                time_offset: -(t - prev_t) / 3.0,
                                value_offset: 0.0,
                            };
                        }
                        let mut handle_out = kf.handle_out.clone();
                        // 出柄：本关键帧为贝塞尔 → 右侧段为贝塞尔，默认外伸 1/3 间距
                        if pos + 1 < sorted.len()
                            && handle_out.time_offset == 0.0
                            && handle_out.value_offset == 0.0
                        {
                            handle_out = SlintBezierHandle {
                                time_offset: (next_t - t) / 3.0,
                                value_offset: 0.0,
                            };
                        }
                        Some((i, handle_in, handle_out))
                    })
                    .collect();
                if let Some(kfs) = self.editing_keyframes.get_mut(curve) {
                    for (i, handle_in, handle_out) in updates {
                        kfs[i].handle_in = handle_in;
                        kfs[i].handle_out = handle_out;
                    }
                }
            }
        }

        // 第二步：写回插值类型（数据层）
        let mut changed = false;
        for curve in &curves {
            let Some(kfs) = self.editing_keyframes.get_mut(curve) else { continue };
            for kf in kfs.iter_mut() {
                if kf.selected && kf.interpolation != interpolation {
                    kf.interpolation = interpolation;
                    changed = true;
                }
            }
        }
        if !changed {
            return Vec::new();
        }
        vec![Tag::AnimationKeyframesChanged {
            curves,
            summary_ms: vec![],
            dragged_curve: None,
            dragged_summary: None,
        }]
    }

    /// 全部选中关键帧的数量（跨通道）。
    pub fn selected_keyframe_count(&self) -> usize {
        self.editing_keyframes
            .values()
            .map(|kfs| kfs.iter().filter(|k| k.selected).count())
            .sum()
    }

    /// 所有选中关键帧的插值聚合状态。
    pub fn selected_interpolation_state(&self) -> SlintInterpolationState {
        let mut first: Option<SlintInterpolationState> = None;
        let mut all_same = true;
        'outer: for kfs in self.editing_keyframes.values() {
            for kf in kfs {
                if kf.selected {
                    let state = match kf.interpolation {
                        SlintInterpolation::Bezier => SlintInterpolationState::Bezier,
                        SlintInterpolation::Linear => SlintInterpolationState::Linear,
                        SlintInterpolation::Hold => SlintInterpolationState::Hold,
                    };
                    match first {
                        None => first = Some(state),
                        Some(prev) if prev != state => {
                            all_same = false;
                            break 'outer;
                        }
                        _ => {}
                    }
                }
            }
        }
        match first {
            Some(state) if all_same => state,
            Some(_) => SlintInterpolationState::Mixed,
            None => SlintInterpolationState::None,
        }
    }
}
