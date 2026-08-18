//! 拖拽钳制预览：手柄/关键帧拖拽时，相邻关键帧边界软钳制的有效控制点预览。
//!
//! `preview_handles` 是独立模型（Slint 单独渲染层），只在拖拽期间填充，
//! 松开时清空（handle_released / keyframe_released 已处理）。

use std::collections::HashMap;

use slint::{Color, Model};

use super::CurveEditorState;
use crate::bridge::types::SlintKeyframe;
use super::curve_math::{make_segment, mk_marker};
use crate::bridge::types::{SlintHandleMarker, SlintInterpolation};

impl CurveEditorState {
    /// 计算一个通道内所有被拖关键帧的相邻段"软钳制后"有效控制点。
    /// 关键帧拖拽时：选中的是关键帧，被拖关键帧的相邻手柄需要钳制预览。
    /// 返回 map：(关键帧下标, is_in) → 有效控制点。
    fn clamped_overrides_for_curve(&self, curve: usize) -> HashMap<(usize, bool), (f32, f32)> {
        let mut map = HashMap::new();
        let Some(kfs) = self.editing_keyframes.get(&curve) else {
            return map;
        };
        let color = self
            .channels_data
            .get(curve)
            .map(|c| c.color)
            .unwrap_or_else(|| Color::from_rgb_u8(200, 200, 200));
        let mut kfs: Vec<(usize, SlintKeyframe)> = kfs.iter().cloned().enumerate().collect();
        kfs.sort_by(|a, b| a.1.time.partial_cmp(&b.1.time).unwrap_or(std::cmp::Ordering::Equal));

        let eps = 1e-4;
        let check = |map: &mut HashMap<(usize, bool), (f32, f32)>,
                     midx: usize,
                     is_in: bool,
                     kf: &SlintKeyframe,
                     eff: (f32, f32)| {
            let raw = if is_in {
                (kf.time + kf.handle_in.time_offset, kf.value + kf.handle_in.value_offset)
            } else {
                (kf.time + kf.handle_out.time_offset, kf.value + kf.handle_out.value_offset)
            };
            if (eff.0 - raw.0).abs() > eps || (eff.1 - raw.1).abs() > eps {
                map.insert((midx, is_in), eff);
            }
        };

        // 关键帧拖拽时：对每个被拖关键帧（selected），计算其相邻段的钳制预览
        for (pos, (_midx, kf)) in kfs.iter().enumerate() {
            // 被拖的关键帧：该关键帧被选中（拖拽时所有选中的关键帧一起移动）
            if !kf.selected {
                continue;
            }
            // 左侧段 [prev, dragged]
            if pos > 0 && kfs[pos - 1].1.interpolation == SlintInterpolation::Bezier {
                let seg = make_segment(curve, color, &kfs[pos - 1].1, &kfs[pos].1);
                check(&mut map, kfs[pos - 1].0, false, &kfs[pos - 1].1, (seg.cx1, seg.cy1));
                check(&mut map, kfs[pos].0, true, &kfs[pos].1, (seg.cx2, seg.cy2));
            }
            // 右侧段 [dragged, next]
            if pos < kfs.len() - 1 && kf.interpolation == SlintInterpolation::Bezier {
                let seg = make_segment(curve, color, &kfs[pos].1, &kfs[pos + 1].1);
                check(&mut map, kfs[pos].0, false, &kfs[pos].1, (seg.cx1, seg.cy1));
                check(&mut map, kfs[pos + 1].0, true, &kfs[pos + 1].1, (seg.cx2, seg.cy2));
            }
        }
        map
    }

    /// 按 preview 覆盖生成一个通道的手柄：被钳制的手柄用有效控制点 + preview 标记替换。
    fn channel_handles_with_preview(
        &self,
        curve: usize,
        overrides: &HashMap<(usize, bool), (f32, f32)>,
    ) -> Vec<SlintHandleMarker> {
        let Some(kfs) = self.editing_keyframes.get(&curve) else {
            return Vec::new();
        };
        let color = self
            .channels_data
            .get(curve)
            .map(|c| c.color)
            .unwrap_or_else(|| Color::from_rgb_u8(200, 200, 200));
        let mut kfs: Vec<(usize, SlintKeyframe)> = kfs.iter().cloned().enumerate().collect();
        kfs.sort_by(|a, b| a.1.time.partial_cmp(&b.1.time).unwrap_or(std::cmp::Ordering::Equal));
        let last = kfs.len().saturating_sub(1);

        let mut out = Vec::new();
        for (pos, (midx, kf)) in kfs.iter().enumerate() {
            let key = *midx;
            // 入柄：仅当左侧段是贝塞尔（前一关键帧是贝塞尔）
            if pos > 0 && kfs[pos - 1].1.interpolation == SlintInterpolation::Bezier {
                let sel = self.selected_handles.contains(&(curve, key, true));
                let raw = (kf.time + kf.handle_in.time_offset, kf.value + kf.handle_in.value_offset);
                match overrides.get(&(key, true)) {
                    Some(&eff) => out.push(mk_marker(curve, key, color, kf, eff.0, eff.1, true, true, sel)),
                    None => out.push(mk_marker(curve, key, color, kf, raw.0, raw.1, true, false, sel)),
                }
            }
            // 出柄：本关键帧是贝塞尔 → 右侧段为贝塞尔
            if pos < last && kf.interpolation == SlintInterpolation::Bezier {
                let sel = self.selected_handles.contains(&(curve, key, false));
                let raw = (kf.time + kf.handle_out.time_offset, kf.value + kf.handle_out.value_offset);
                match overrides.get(&(key, false)) {
                    Some(&eff) => out.push(mk_marker(curve, key, color, kf, eff.0, eff.1, false, true, sel)),
                    None => out.push(mk_marker(curve, key, color, kf, raw.0, raw.1, false, false, sel)),
                }
            }
        }
        out
    }

    /// 重建 handles 模型：被拖通道的钳制手柄替换为 preview。
    /// 只重建被拖通道（增量 splice），不触碰其他通道。
    pub(super) fn rebuild_handles_with_preview(&mut self, curve: usize) {
        let overrides = self.clamped_overrides_for_curve(curve);
        let (handles, old_hg, old_he) = {
            let hg = self.handle_offsets[curve];
            let he = self.handle_offsets.get(curve + 1).copied().unwrap_or(self.handles.row_count());
            (self.channel_handles_with_preview(curve, &overrides), hg, he)
        };
        let old_count = old_he - old_hg;
        if old_count == handles.len() {
            for (k, h) in handles.iter().enumerate() {
                self.handles.set_row_data(old_hg + k, h.clone());
            }
        } else {
            for _ in 0..old_count {
                self.handles.remove(old_hg);
            }
            for (k, h) in handles.iter().enumerate() {
                self.handles.insert(old_hg + k, h.clone());
            }
            let delta = handles.len() as isize - old_count as isize;
            for offset in &mut self.handle_offsets[curve + 1..] {
                *offset = (*offset as isize + delta) as usize;
            }
        }
        self.rebuild_dope_connections_for_curve(curve);
    }

    /// 计算被拖手柄的"软钳制后"有效控制点预览标记；未被钳制（有效 == 原始）时返回 None。
    pub(super) fn dragged_handle_preview(&self, curve: usize, key: usize, is_in: bool) -> Option<SlintHandleMarker> {
        let kfs = self.editing_keyframes.get(&curve)?;
        let color = self
            .channels_data
            .get(curve)
            .map(|c| c.color)
            .unwrap_or_else(|| Color::from_rgb_u8(200, 200, 200));
        let mut kfs: Vec<(usize, SlintKeyframe)> = kfs.iter().cloned().enumerate().collect();
        kfs.sort_by(|a, b| a.1.time.partial_cmp(&b.1.time).unwrap_or(std::cmp::Ordering::Equal));
        let pos = kfs.iter().position(|&(i, _)| i == key)?;
        let eps = 1e-4;
        // 出柄 → 右侧段 [dragged, next]（dragged 是贝塞尔才软钳）；入柄 → 左侧段 [prev, dragged]（prev 是贝塞尔）
        let (eff, raw) = if is_in {
            if pos == 0 || kfs[pos - 1].1.interpolation != SlintInterpolation::Bezier {
                return None;
            }
            let seg = make_segment(curve, color, &kfs[pos - 1].1, &kfs[pos].1);
            let kf = &kfs[pos].1;
            (
                (seg.cx2, seg.cy2),
                (kf.time + kf.handle_in.time_offset, kf.value + kf.handle_in.value_offset),
            )
        } else {
            if pos + 1 >= kfs.len() || kfs[pos].1.interpolation != SlintInterpolation::Bezier {
                return None;
            }
            let seg = make_segment(curve, color, &kfs[pos].1, &kfs[pos + 1].1);
            let kf = &kfs[pos].1;
            (
                (seg.cx1, seg.cy1),
                (kf.time + kf.handle_out.time_offset, kf.value + kf.handle_out.value_offset),
            )
        };
        if (eff.0 - raw.0).abs() > eps || (eff.1 - raw.1).abs() > eps {
            Some(mk_marker(curve, key, color, &kfs[pos].1, eff.0, eff.1, is_in, true, false))
        } else {
            None
        }
    }
}
