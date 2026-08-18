//! 控制柄交互（reducer 侧）：只改数据层（`editing_keyframes` 手柄偏移、`selected_handles`）
//! 并返回 tag；手柄模型重建/钳制预览由投影（projection.rs）完成。

use super::CurveEditorState;
use super::curve_math::{clamp_handle_own_side, apply_handle_mode};
use crate::bridge::types::{SlintHandleDragMode, SlintHandleMarker, SlintSelectionModifier};
use crate::depsgraph::Tag;

impl CurveEditorState {
    /// 控制柄按下：none=单选，shift=同通道同侧区间扩展，ctrl=切换。
    /// 锁定通道不可交互。只改 selected_handles 数据，返回选中变化 tag。
    pub fn handle_pressed(&mut self, curve: usize, key: usize, is_in: bool, modifier: SlintSelectionModifier) -> Vec<Tag> {
        if !self.channel_editable(curve) {
            return Vec::new();
        }
        let key_in_sel = self.selected_handles.contains(&(curve, key, is_in));
        match modifier {
            SlintSelectionModifier::Shift => {
                let anchor = self.handle_selection_anchor.unwrap_or((curve, key, is_in));
                let (lo, hi) = if anchor.0 == curve && anchor.2 == is_in {
                    (anchor.1.min(key), anchor.1.max(key))
                } else {
                    (key, key)
                };
                for index in lo..=hi {
                    self.selected_handles.insert((curve, index, is_in));
                }
                self.handle_selection_anchor = Some((curve, key, is_in));
            }
            SlintSelectionModifier::Ctrl => {
                if key_in_sel {
                    self.selected_handles.remove(&(curve, key, is_in));
                } else {
                    self.selected_handles.insert((curve, key, is_in));
                }
            }
            SlintSelectionModifier::None => {
                self.handle_selection_anchor = Some((curve, key, is_in));
                if !key_in_sel {
                    self.deselect_all();
                    self.selected_handles.insert((curve, key, is_in));
                }
            }
        }
        // 拖控制柄也聚焦并选中该通道（曲线保持高亮）
        self.channel_pressed(curve, SlintSelectionModifier::None);
        vec![Tag::AnimationSelectionChanged { summary_ms: vec![] }]
    }

    /// 控制柄拖拽：把增量累加到被拖控制柄，并按约束模式处理。
    /// 只改数据层；手柄/曲线重建与软钳制预览走 tag 投影（preview_handles 除外——
    /// 它依赖拖拽上下文，作为拖拽预览由本方法直接写，对齐 OverlayState 语义）。
    pub fn handle_dragged(
        &mut self,
        curve: usize,
        key: usize,
        is_in: bool,
        delta_time: f32,
        delta_value: f32,
        mode: SlintHandleDragMode,
    ) -> Vec<Tag> {
        if !self.channel_editable(curve) {
            return Vec::new();
        }
        if !delta_time.is_finite() || !delta_value.is_finite() {
            return Vec::new();
        }
        // clone 被拖关键帧，避免跨循环的可变借用（数据层最后统一写回）
        let mut kf = match self
            .editing_keyframes
            .get(&curve)
            .and_then(|kfs| kfs.get(key))
        {
            Some(kf) => kf.clone(),
            None => return Vec::new(),
        };

        // 被拖手柄累加增量（自由移动，角度与长度都可变）
        if is_in {
            kf.handle_in.time_offset += delta_time;
            kf.handle_in.value_offset += delta_value;
        } else {
            kf.handle_out.time_offset += delta_time;
            kf.handle_out.value_offset += delta_value;
        };

        // 多选同侧手柄：各自按相同屏幕偏移移动（世界增量一致 → 屏幕位移一致，更符合预期）。
        // 非主手柄同样做"自己关键帧一侧"的硬钳制；软钳制预览对所有被移动的手柄都做。
        let mut synchronized_handles: Vec<(usize, usize, bool)> = Vec::new();
        if (delta_time != 0.0 || delta_value != 0.0) && !self.selected_handles.is_empty() {
            for &(curve_index, keyframe_index, is_incoming) in &self.selected_handles {
                if curve_index == curve && keyframe_index == key && is_incoming == is_in {
                    continue; // 被拖手柄已更新
                }
                if is_incoming != is_in {
                    continue; // 只同步同一侧
                }
                if !self.channel_editable(curve_index) {
                    continue;
                }
                let Some(kf) = self
                    .editing_keyframes
                    .get_mut(&curve_index)
                    .and_then(|kfs| kfs.get_mut(keyframe_index))
                else {
                    continue;
                };
                if is_incoming {
                    kf.handle_in.time_offset += delta_time;
                    kf.handle_in.value_offset += delta_value;
                    clamp_handle_own_side(kf, true);
                } else {
                    kf.handle_out.time_offset += delta_time;
                    kf.handle_out.value_offset += delta_value;
                    clamp_handle_own_side(kf, false);
                }
                // 与主手柄一致的模式：对每个被同步手柄的对手柄同样应用（对齐/原点对称/自由）
                apply_handle_mode(kf, is_incoming, mode);
                clamp_handle_own_side(kf, true);
                clamp_handle_own_side(kf, false);
                synchronized_handles.push((curve_index, keyframe_index, is_incoming));
            }
        }

        // 主手柄：按模式约束其对手柄
        apply_handle_mode(&mut kf, is_in, mode);

        // 硬钳制：被拖柄与对手柄都钳到"自己关键帧"一侧；
        // 相邻关键帧边界为软钳制（渲染期交叉钳制处理）
        clamp_handle_own_side(&mut kf, true);
        clamp_handle_own_side(&mut kf, false);
        // 写回被拖关键帧
        if let Some(kfs) = self.editing_keyframes.get_mut(&curve)
            && let Some(dst) = kfs.get_mut(key)
        {
            *dst = kf;
        }

        // 软钳制预览：主手柄 + 所有被同步手柄的有效控制点（与原始不同才显示）。
        // 拖拽上下文预览，直接写（不经 depsgraph）。
        let mut preview: Vec<SlintHandleMarker> = Vec::new();
        for &(curve_index, keyframe_index, is_incoming) in synchronized_handles.iter().chain(std::iter::once(&(curve, key, is_in))) {
            if let Some(preview_marker) = self.dragged_handle_preview(curve_index, keyframe_index, is_incoming) {
                preview.push(preview_marker);
            }
        }
        self.preview_handles.set_vec(preview);

        // 被拖通道 + 被同步通道的段/手柄重建走投影
        let mut curves: Vec<usize> = vec![curve];
        for &(curve_index, _, _) in &synchronized_handles {
            if !curves.contains(&curve_index) {
                curves.push(curve_index);
            }
        }
        vec![Tag::AnimationKeyframesChanged {
            curves,
            summary_ms: vec![],
            dragged_curve: None,
            dragged_summary: None,
        }]
    }

    /// 控制柄松开。moved=true 表示发生过拖拽（保持多选）；false 为单击 → 折叠为单选该手柄。
    pub fn handle_released(&mut self, curve: usize, key: usize, is_in: bool, moved: bool) -> Vec<Tag> {
        let mut summary_ms = Vec::new();
        if !moved {
            // 单击已选中的手柄：折叠为单选
            for ms in self.deselect_all() {
                summary_ms.push(ms);
            }
            self.selected_handles.insert((curve, key, is_in));
        }
        self.preview_handles.clear();
        vec![Tag::AnimationSelectionChanged { summary_ms }]
    }
}
