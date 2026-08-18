//! 控制柄交互（reducer 侧）：只改数据层（`editing_keyframes` 手柄偏移、`selected_handles`）
//! 并返回 tag；手柄模型重建/钳制预览（preview_handles）由投影（projection.rs）完成。

use super::CurveEditorState;
use super::curve_math::{clamp_handle_own_side, apply_handle_mode};
use crate::bridge::types::{SlintHandleDragMode, SlintSelectionModifier};
use crate::depsgraph::{HandleLoc, Tag};

impl CurveEditorState {
    /// 控制柄按下：none=单选，shift=同通道同侧区间扩展，ctrl=切换。
    /// 锁定通道不可交互。只改 selected_handles 数据，返回选中变化 tag。
    pub fn handle_pressed(&mut self, curve: usize, key: usize, is_in: bool, modifier: SlintSelectionModifier) -> Vec<Tag> {
        if !self.channel_editable(curve) {
            return Vec::new();
        }
        let key_in_sel = self.selected_handles.contains(&HandleLoc { curve, key, is_in });
        match modifier {
            SlintSelectionModifier::Shift => {
                let anchor = self
                    .handle_selection_anchor
                    .unwrap_or(HandleLoc { curve, key, is_in });
                let (lo, hi) = if anchor.curve == curve && anchor.is_in == is_in {
                    (anchor.key.min(key), anchor.key.max(key))
                } else {
                    (key, key)
                };
                for index in lo..=hi {
                    self.selected_handles.insert(HandleLoc { curve, key: index, is_in });
                }
                self.handle_selection_anchor = Some(HandleLoc { curve, key, is_in });
            }
            SlintSelectionModifier::Ctrl => {
                if key_in_sel {
                    self.selected_handles.remove(&HandleLoc { curve, key, is_in });
                } else {
                    self.selected_handles.insert(HandleLoc { curve, key, is_in });
                }
            }
            SlintSelectionModifier::None => {
                self.handle_selection_anchor = Some(HandleLoc { curve, key, is_in });
                if !key_in_sel {
                    self.deselect_all();
                    self.selected_handles.insert(HandleLoc { curve, key, is_in });
                }
            }
        }
        // 拖控制柄也聚焦并选中该通道（曲线保持高亮）
        self.channel_pressed(curve, SlintSelectionModifier::None);
        vec![Tag::AnimationSelectionChanged { summary_ms: vec![] }]
    }

    /// 控制柄拖拽：把增量累加到被拖控制柄，并按约束模式处理。
    /// 只改数据层；手柄/曲线重建与软钳制预览（preview_handles）走 tag 投影。
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
        let mut synchronized_handles: Vec<HandleLoc> = Vec::new();
        if (delta_time != 0.0 || delta_value != 0.0) && !self.selected_handles.is_empty() {
            for &loc in &self.selected_handles {
                if loc.curve == curve && loc.key == key && loc.is_in == is_in {
                    continue; // 被拖手柄已更新
                }
                if loc.is_in != is_in {
                    continue; // 只同步同一侧
                }
                if !self.channel_editable(loc.curve) {
                    continue;
                }
                let Some(kf) = self
                    .editing_keyframes
                    .get_mut(&loc.curve)
                    .and_then(|kfs| kfs.get_mut(loc.key))
                else {
                    continue;
                };
                if loc.is_in {
                    kf.handle_in.time_offset += delta_time;
                    kf.handle_in.value_offset += delta_value;
                    clamp_handle_own_side(kf, true);
                } else {
                    kf.handle_out.time_offset += delta_time;
                    kf.handle_out.value_offset += delta_value;
                    clamp_handle_own_side(kf, false);
                }
                // 与主手柄一致的模式：对每个被同步手柄的对手柄同样应用（对齐/原点对称/自由）
                apply_handle_mode(kf, loc.is_in, mode);
                clamp_handle_own_side(kf, true);
                clamp_handle_own_side(kf, false);
                synchronized_handles.push(loc);
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

        // 被拖通道 + 被同步通道的段/手柄重建走投影；
        // 被移动的手柄列表（被拖 + 同步 + 各自对向柄）一并交给投影重建 preview_handles。
        // 对向柄在 Aligned（共线）/Symmetric（原点对称）模式下被 apply_handle_mode 约束，
        // 可能越过相邻关键帧边界被软钳制——同样需要预览。
        let mut curves: Vec<usize> = vec![curve];
        let mut handle_preview: Vec<HandleLoc> = vec![
            HandleLoc { curve, key, is_in },
            HandleLoc { curve, key, is_in: !is_in },
        ];
        for &loc in &synchronized_handles {
            if !curves.contains(&loc.curve) {
                curves.push(loc.curve);
            }
            for loc in [loc, HandleLoc { curve: loc.curve, key: loc.key, is_in: !loc.is_in }] {
                if !handle_preview.contains(&loc) {
                    handle_preview.push(loc);
                }
            }
        }
        vec![Tag::AnimationKeyframesChanged {
            curves,
            summary_ms: vec![],
            dragged_curve: None,
            dragged_summary: None,
            handle_preview: Some(handle_preview),
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
            self.selected_handles.insert(HandleLoc { curve, key, is_in });
        }
        // 手柄拖拽结束不打 AnimationDragEnded：补柄（ensure_default_handles）只适用于
        // 关键帧拖拽改变边缘性的场景；手柄可能被故意钳到 0 长度，不该被覆盖。
        // preview_handles 由投影在选中变化时清空。
        vec![Tag::AnimationSelectionChanged { summary_ms }]
    }
}
