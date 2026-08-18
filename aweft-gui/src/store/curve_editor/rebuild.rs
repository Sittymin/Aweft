//! 派生数据重建：由关键帧数据推导曲线段/控制柄/时长范围，并增量更新 Slint 模型。
//!
//! `rebuild_all` 仅用于初始化/重置（load_action / reset_action）；
//! 运行时交互使用 `rebuild_curve`（数量不变 set_row_data，数量变化 splice）。

use slint::{Color, Model};

use super::CurveEditorState;
use super::curve_math::make_segment;
use crate::bridge::types::{
    SlintHandleMarker, SlintInterpolation, SlintKeyframe, SlintCurveSegment,
};
use crate::depsgraph::HandleLoc;

impl CurveEditorState {
    /// 全量重算派生数据。**仅用于初始化/重置**（load_action / reset_action）；
    /// 运行时交互应使用 `rebuild_curve`，不应调用此方法。
    pub(super) fn rebuild_all(&mut self) {
        let n = self.channels_data.len();
        let mut segments: Vec<SlintCurveSegment> = Vec::new();
        let mut handles: Vec<SlintHandleMarker> = Vec::new();
        let mut seg_offsets = vec![0usize; n + 1];
        let mut hdl_offsets = vec![0usize; n + 1];
        for curve in 0..n {
            let (curve_segs, curve_hdls) = self.curve_elements(curve);
            segments.extend(curve_segs);
            handles.extend(curve_hdls);
            seg_offsets[curve + 1] = segments.len();
            hdl_offsets[curve + 1] = handles.len();
        }
        self.segment_offsets = seg_offsets;
        self.handle_offsets = hdl_offsets;
        self.segments.set_vec(segments);
        self.handles.set_vec(handles);
        self.rebuild_dope_connections();
        self.rebuild_summary();
        self.rebuild_action_range();
    }

    /// 局部更新一个通道的曲线段与控制柄。
    /// 数量不变 → set_row_data（零重建）；数量变化 → splice 移除旧区间插入新数据并更新后续偏移。
    pub(super) fn rebuild_curve(&mut self, curve: usize) {
        if self.segment_offsets.len() <= curve + 1 {
            return;
        }
        let (segments, handles) = self.curve_elements(curve);

        // 段：数量相同原地 set_row_data，不同则 splice
        let sg = self.segment_offsets[curve];
        let se = self.segment_offsets[curve + 1];
        if se - sg == segments.len() {
            for (k, seg) in segments.iter().enumerate() {
                self.segments.set_row_data(sg + k, seg.clone());
            }
        } else {
            for _ in 0..(se - sg) {
                self.segments.remove(sg);
            }
            for (k, seg) in segments.iter().enumerate() {
                self.segments.insert(sg + k, seg.clone());
            }
            let delta = segments.len() as isize - (se - sg) as isize;
            for offset in &mut self.segment_offsets[curve + 1..] {
                *offset = (*offset as isize + delta) as usize;
            }
        }

        // 手柄：同上
        let hg = self.handle_offsets[curve];
        let he = self.handle_offsets[curve + 1];
        if he - hg == handles.len() {
            for (k, h) in handles.iter().enumerate() {
                self.handles.set_row_data(hg + k, h.clone());
            }
        } else {
            for _ in 0..(he - hg) {
                self.handles.remove(hg);
            }
            for (k, h) in handles.iter().enumerate() {
                self.handles.insert(hg + k, h.clone());
            }
            let delta = handles.len() as isize - (he - hg) as isize;
            for offset in &mut self.handle_offsets[curve + 1..] {
                *offset = (*offset as isize + delta) as usize;
            }
        }

        // 摄影表连接线：只重建被改变通道的连接（增量 splice）
        self.rebuild_dope_connections_for_curve(curve);
    }

    /// 计算一个通道的曲线段与控制柄（不修改任何模型，供重建用）。
    ///
    /// 关键帧可能因拖拽而暂时乱序：这里先按时间排序再推导曲线段/端点手柄，
    /// 保证曲线始终是时间的函数；但模型顺序不变（拖拽抓手仍按原模型下标命中）。
    fn curve_elements(
        &self,
        curve: usize,
    ) -> (Vec<SlintCurveSegment>, Vec<SlintHandleMarker>) {
        let Some(kfs) = self.editing_keyframes.get(&curve) else {
            return (Vec::new(), Vec::new());
        };
        let color = self
            .channels_data
            .get(curve)
            .map(|c| c.color)
            .unwrap_or_else(|| Color::from_rgb_u8(200, 200, 200));

        // (模型下标, 关键帧引用)，按时间排序
        let mut kfs: Vec<(usize, &SlintKeyframe)> = kfs.iter().enumerate().collect();
        kfs.sort_by(|a, b| a.1.time.partial_cmp(&b.1.time).unwrap_or(std::cmp::Ordering::Equal));

        let mut segments = Vec::new();
        for i in 0..kfs.len().saturating_sub(1) {
            segments.push(make_segment(curve, color, &kfs[i].1, &kfs[i + 1].1));
        }

        let mut handles = Vec::new();
        let last = kfs.len().saturating_sub(1);
        for (pos, (model_idx, kf)) in kfs.iter().enumerate() {
            // 入柄：仅当左侧段是贝塞尔（前一关键帧是贝塞尔）才有意义
            if pos > 0 && kfs[pos - 1].1.interpolation == SlintInterpolation::Bezier {
                handles.push(SlintHandleMarker {
                    curve_index: curve as i32,
                    keyframe_index: *model_idx as i32,
                    is_in: true,
                    color,
                    key_time: kf.time,
                    key_value: kf.value,
                    handle_time: kf.time + kf.handle_in.time_offset,
                    handle_value: kf.value + kf.handle_in.value_offset,
                    preview: false,
                    selected: self.selected_handles.contains(&HandleLoc { curve, key: *model_idx, is_in: true }),
                });
            }
            // 出柄：本关键帧是贝塞尔 → 右侧段为贝塞尔
            if pos < last && kf.interpolation == SlintInterpolation::Bezier {
                handles.push(SlintHandleMarker {
                    curve_index: curve as i32,
                    keyframe_index: *model_idx as i32,
                    is_in: false,
                    color,
                    key_time: kf.time,
                    key_value: kf.value,
                    handle_time: kf.time + kf.handle_out.time_offset,
                    handle_value: kf.value + kf.handle_out.value_offset,
                    preview: false,
                    selected: self.selected_handles.contains(&HandleLoc { curve, key: *model_idx, is_in: false }),
                });
            }
        }
        (segments, handles)
    }

    /// 由全部关键帧时间推导 Action 时长范围（时长内亮、时长外暗）。
    fn rebuild_action_range(&mut self) {
        let mut min_t = f32::INFINITY;
        let mut max_t = f32::NEG_INFINITY;
        for kfs in self.editing_keyframes.values() {
            for kf in kfs {
                min_t = min_t.min(kf.time);
                max_t = max_t.max(kf.time);
            }
        }
        if min_t.is_finite() {
            self.action_start = min_t;
            self.action_end = max_t;
        } else {
            self.action_start = 0.0;
            self.action_end = 0.0;
        }
    }
}
