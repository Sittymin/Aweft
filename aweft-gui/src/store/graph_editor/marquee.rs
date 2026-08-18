use std::collections::HashSet;

use slint::Model;

use super::GraphEditorState;
use super::curve_math::segment_intersects_rect;
use crate::depsgraph::{HandleLoc, Tag};
use super::ms_of;
use crate::bridge::types::SlintSelectionModifier;

impl GraphEditorState {
    // ── 共享框选核心逻辑 ──────────────────────────────────────────────────

    /// 框选核心逻辑：在指定通道范围内，按时间+值范围选中关键帧。
    ///
    /// - `channels`：要扫描的通道范围（dope 用行区间，graph 用全部可见）
    /// - `time_range`：(t_lo, t_hi) 世界时间范围
    /// - `value_range`：(v_lo, v_hi) 世界值范围，None 表示不限值（dope 摄影表只有时间轴）
    /// - `modifier`：选择模式（None=替换，Shift=加选，Ctrl=切换）
    ///
    /// 返回 (命中的通道列表, 受影响的 ms 集合)。
    fn marquee_select_keyframes(
        &mut self,
        channels: impl Iterator<Item = usize>,
        time_range: (f32, f32),
        value_range: Option<(f32, f32)>,
        modifier: SlintSelectionModifier,
    ) -> (Vec<usize>, HashSet<i64>) {
        let (t_lo, t_hi) = time_range;
        let replacing = modifier == SlintSelectionModifier::None;
        let toggling = modifier == SlintSelectionModifier::Ctrl;

        if replacing {
            self.deselect_all();
            self.pending_channel_selection.clear();
        }

        let mut hit_channels: Vec<usize> = Vec::new();
        let mut affected_ms: HashSet<i64> = HashSet::new();

        for curve in channels {
            let visible = self
                .channels_data
                .get(curve)
                .map(|c| c.visible)
                .unwrap_or(false);
            if !visible {
                continue;
            }
            let Some(kfs) = self.editing_keyframes.get_mut(&curve) else {
                continue;
            };
            let mut changed = false;
            let mut has_kf_in_box = false;
            for kf in kfs.iter_mut() {
                let in_time = kf.time >= t_lo && kf.time <= t_hi;
                let in_value = value_range
                    .map(|(v_lo, v_hi)| kf.value >= v_lo && kf.value <= v_hi)
                    .unwrap_or(true);
                if !in_time || !in_value {
                    continue;
                }
                has_kf_in_box = true;
                affected_ms.insert(ms_of(kf.time));
                let new_sel = if toggling { !kf.selected } else { true };
                if kf.selected != new_sel {
                    kf.selected = new_sel;
                    changed = true;
                }
            }
            // Ctrl 切换：只要框内有关键帧就加入（通道也要 toggle）
            if changed || (toggling && has_kf_in_box) {
                hit_channels.push(curve);
            }
        }

        // 通道选中
        match modifier {
            SlintSelectionModifier::None => {
                self.clear_channel_selection();
                for &c in &hit_channels {
                    self.set_channel_selected(c, true);
                }
            }
            SlintSelectionModifier::Shift => {
                for &c in &hit_channels {
                    self.set_channel_selected(c, true);
                }
            }
            SlintSelectionModifier::Ctrl => {
                for &c in &hit_channels {
                    let cur = self.channel_selected(c);
                    self.set_channel_selected(c, !cur);
                }
            }
        }

        (hit_channels, affected_ms)
    }

    // ── 摄影表框选 ────────────────────────────────────────────────────────

    /// 摄影表空白处框选：按世界时间范围 [x_lo, x_hi] 与行（通道）区间 [row_lo, row_hi]
    /// 选中范围内、且通道可见的关键帧（菱形中心落在框内）。
    /// 只在框选结束（grab 已结束）后调用，可直接更新 channels 外层模型。
    pub fn dope_marquee_select(
        &mut self,
        x_lo: f32,
        x_hi: f32,
        row_lo: i32,
        row_hi: i32,
        modifier: SlintSelectionModifier,
    ) -> Vec<Tag> {
        let n = self.channels_data.len() as i32;
        if n <= 0 {
            return Vec::new();
        }
        let r_lo = row_lo.max(0).min(n - 1);
        let r_hi = row_hi.max(0).min(n - 1);
        let (r_lo, r_hi) = (r_lo.min(r_hi), r_lo.max(r_hi));
        let (t_lo, t_hi) = if x_lo <= x_hi { (x_lo, x_hi) } else { (x_hi, x_lo) };
        if t_hi < t_lo {
            return Vec::new();
        }

        let channels = r_lo as usize..=r_hi as usize;
        let (hit_channels, affected_ms) = self.marquee_select_keyframes(
            channels,
            (t_lo, t_hi),
            None, // 摄影表只按时间选，不按值
            modifier,
        );

        if let Some(&first) = hit_channels.first() {
            self.current_channel = first as isize;
        }
        vec![Tag::AnimationSelectionChanged {
            summary_ms: affected_ms.into_iter().collect(),
        }]
    }

    // ── 曲线编辑器框选 ────────────────────────────────────────────────────

    /// 曲线编辑器空白处框选：按世界时间范围 [x_lo, x_hi] × 值范围 [y_lo, y_hi]
    /// 选中框内的**关键帧**（菱形中心）与**贝塞尔控制柄**（控制点），跳过不可见通道。
    /// modifier：None=替换，Shift=加选，Ctrl=切换。
    /// 只在框选结束（grab 已结束）后调用，可直接更新 channels 外层模型。
    pub fn graph_marquee_select(
        &mut self,
        x_lo: f32,
        x_hi: f32,
        y_lo: f32,
        y_hi: f32,
        modifier: SlintSelectionModifier,
    ) -> Vec<Tag> {
        let (t_lo, t_hi) = if x_lo <= x_hi { (x_lo, x_hi) } else { (x_hi, x_lo) };
        let (v_lo, v_hi) = if y_lo <= y_hi { (y_lo, y_hi) } else { (y_hi, y_lo) };

        let toggling = modifier == SlintSelectionModifier::Ctrl;

        // 关键帧选中（通过共享逻辑）
        let (mut hit_channels, affected_ms) = self.marquee_select_keyframes(
            0..self.channels_data.len(),
            (t_lo, t_hi),
            Some((v_lo, v_hi)),
            modifier,
        );

        // 手柄：控制点（关键帧 + 偏移）落入框内
        for curve in 0..self.channels_data.len() {
            let visible = self
                .channels_data
                .get(curve)
                .map(|c| c.visible)
                .unwrap_or(false);
            if !visible {
                continue;
            }
            let Some(kfs) = self.editing_keyframes.get(&curve) else {
                continue;
            };
            for (j, kf) in kfs.iter().enumerate() {
                for si in [true, false] {
                    let (ho_t, ho_v) = if si {
                        (kf.handle_in.time_offset, kf.handle_in.value_offset)
                    } else {
                        (kf.handle_out.time_offset, kf.handle_out.value_offset)
                    };
                    let hx = kf.time + ho_t;
                    let hy = kf.value + ho_v;
                    if hx >= t_lo && hx <= t_hi && hy >= v_lo && hy <= v_hi {
                        let loc = HandleLoc { curve, key: j, is_in: si };
                        if toggling {
                            if self.selected_handles.contains(&loc) {
                                self.selected_handles.remove(&loc);
                            } else {
                                self.selected_handles.insert(loc);
                            }
                        } else {
                            self.selected_handles.insert(loc);
                        }
                    }
                }
            }
        }

        // 框住曲线本身也选中所在通道：判断该通道是否有任一段曲线与框选矩形相交
        for row in 0..self.graph_segments.row_count() {
            let Some(seg) = self.graph_segments.row_data(row) else {
                continue;
            };
            let curve = seg.curve_index as usize;
            if hit_channels.contains(&curve) {
                continue;
            }
            let visible = self
                .channels_data
                .get(curve)
                .map(|c| c.visible)
                .unwrap_or(false);
            if !visible {
                continue;
            }
            if segment_intersects_rect(&seg, t_lo, t_hi, v_lo, v_hi) {
                hit_channels.push(curve);
            }
        }

        // 通道选中（补充曲线相交的通道）
        match modifier {
            SlintSelectionModifier::None => {
                self.clear_channel_selection();
                for &c in &hit_channels {
                    self.set_channel_selected(c, true);
                }
            }
            SlintSelectionModifier::Shift => {
                for &c in &hit_channels {
                    self.set_channel_selected(c, true);
                }
            }
            SlintSelectionModifier::Ctrl => {
                for &c in &hit_channels {
                    let cur = self.channel_selected(c);
                    self.set_channel_selected(c, !cur);
                }
            }
        }

        if let Some(&c) = hit_channels.first() {
            self.current_channel = c as isize;
        }
        vec![Tag::AnimationSelectionChanged {
            summary_ms: affected_ms.into_iter().collect(),
        }]
    }
}
