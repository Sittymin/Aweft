//! 通道操作（reducer 侧）：只改数据层（`channels_data`/选中集）并返回 tag；
//! channels 模型行同步、连接线/汇总重建由投影（projection.rs）完成。

use log::warn;
use slint::{Color, Model};

use super::GraphEditorState;
use crate::bridge::types::{SlintColorMode, SlintSelectionModifier};
use crate::depsgraph::Tag;

impl GraphEditorState {
    /// 摄影表点击压缩视图中的一行：把显示行映射回真实通道下标再选中。
    pub fn dope_channel_pressed(
        &mut self,
        view_row: usize,
        modifier: SlintSelectionModifier,
    ) -> Vec<Tag> {
        let real = self
            .dope_filter
            .row_data(view_row)
            .map(|c| c.curve_index as usize)
            .unwrap_or(view_row);
        self.channel_pressed(real, modifier)
    }

    /// 通道是否可编辑（锁）。
    pub(crate) fn channel_editable(&self, curve: usize) -> bool {
        self.channels_data
            .get(curve)
            .map(|c| c.editable)
            .unwrap_or(false)
    }

    pub(crate) fn channel_selected(&self, curve: usize) -> bool {
        self.channels_data
            .get(curve)
            .map(|c| c.selected)
            .unwrap_or(false)
    }

    /// 设置通道选中（数据层；channels 模型行由投影同步）。
    pub(crate) fn set_channel_selected(&mut self, curve: usize, selected: bool) {
        let Some(ch) = self.channels_data.get_mut(curve) else {
            warn!("set_channel_selected: 无效通道下标 {}", curve);
            return;
        };
        ch.selected = selected;
    }

    /// 清除全部通道选中（数据层）。
    pub(crate) fn clear_channel_selection(&mut self) {
        for ch in &mut self.channels_data {
            ch.selected = false;
        }
    }

    /// 通道/曲线按下：none=普通，shift=区间选择，ctrl=切换。
    /// 只改数据层，返回选中变化 tag。
    pub(crate) fn channel_pressed(
        &mut self,
        curve: usize,
        modifier: SlintSelectionModifier,
    ) -> Vec<Tag> {
        if curve >= self.channels_data.len() {
            warn!(
                "channel_pressed: 通道下标 {} 超出范围 ({})",
                curve,
                self.channels_data.len()
            );
            return Vec::new();
        }
        match modifier {
            // Shift：从锚点（最近一次普通/Shift 点击）到当前通道全选中，其余保留
            SlintSelectionModifier::Shift => {
                let anchor =
                    if (0..self.channels_data.len() as isize).contains(&self.selection_anchor) {
                        self.selection_anchor
                    } else {
                        curve as isize
                    };
                let (lo, hi) = if anchor <= curve as isize {
                    (anchor, curve as isize)
                } else {
                    (curve as isize, anchor)
                };
                for i in lo..=hi {
                    self.set_channel_selected(i as usize, true);
                }
                self.selection_anchor = curve as isize;
                self.current_channel = curve as isize;
            }
            // Ctrl：切换该通道选中，其余保留
            SlintSelectionModifier::Ctrl => {
                let new_sel = !self.channel_selected(curve);
                self.set_channel_selected(curve, new_sel);
                if new_sel {
                    self.current_channel = curve as isize;
                }
                // Ctrl 不更新区间锚点
            }
            // 普通：清除全部选中，只选当前通道
            SlintSelectionModifier::None => {
                self.clear_channel_selection();
                self.set_channel_selected(curve, true);
                self.selection_anchor = curve as isize;
                self.current_channel = curve as isize;
            }
        }
        vec![Tag::AnimationSelectionChanged { summary_ms: vec![] }]
    }

    /// 已选中通道数量。
    pub fn selected_channel_count(&self) -> usize {
        self.channels_data.iter().filter(|c| c.selected).count()
    }

    /// 清除激活通道与全部通道选中（画布空白点击）。
    pub fn deselect_channel(&mut self) -> Vec<Tag> {
        self.current_channel = -1;
        self.selection_anchor = -1;
        self.pending_channel_selection.clear();
        self.clear_channel_selection();
        vec![Tag::AnimationSelectionChanged { summary_ms: vec![] }]
    }

    /// 把按下关键帧时记录的期望通道选中写回数据层。
    /// 只能在关键帧 grab 结束（松开）后调用，否则外层模型更新会破坏 Slint 拖拽抓取
    /// （channels 行同步由投影在 Selection tag 时执行）。
    pub(crate) fn apply_pending_channel_selection(&mut self) -> Vec<Tag> {
        let pending = std::mem::take(&mut self.pending_channel_selection);
        self.clear_channel_selection();
        for &i in &pending {
            self.set_channel_selected(i, true);
        }
        vec![Tag::AnimationSelectionChanged { summary_ms: vec![] }]
    }

    /// 当前"生效"的选中通道集合 = 已写回数据层的选中 ∪ 尚未应用的 pending 选中。
    /// 连续按下（两次按下之间未松开）时，通道数据可能还没写回，需要把 pending 也算上，
    /// 否则 Shift 追加会丢掉上一个按下的通道。
    ///
    /// NOTE: 鼠标操作中两次按之间必有 release（pending 已写回），此函数实际触发概率很低。
    /// 保留是为理论上"不松手连续操作"的兜底。
    pub(crate) fn effective_selected_channels(&self) -> Vec<usize> {
        self.channels_data
            .iter()
            .enumerate()
            .filter(|(i, c)| c.selected || self.pending_channel_selection.contains(i))
            .map(|(i, _)| i)
            .collect()
    }

    /// 即时同步通道选中**显示**掩码（`selected_display`，投影专用）。
    /// 独立模型，可在拖拽/按下期间安全更新（"显示先动"，不触碰 channels 模型）。
    /// 增量更新：只写变化的行（`set_row_data`），新增通道时 `push` 补齐。
    pub(crate) fn sync_selected_display(&mut self) {
        let n = self.channels_data.len();
        let mut desired = vec![false; n];
        for &i in &self.effective_selected_channels() {
            if i < n {
                desired[i] = true;
            }
        }
        // 补齐新增通道
        while self.selected_display.row_count() < n {
            self.selected_display.push(false);
        }
        for i in 0..n {
            let old = self.selected_display.row_data(i).unwrap_or(false);
            if old != desired[i] {
                self.selected_display.set_row_data(i, desired[i]);
            }
        }
    }

    /// 自动彩虹调色板：按通道下标取色（彩虹色系）。
    pub(super) fn rainbow_color(index: usize) -> Color {
        const PALETTE: [(u8, u8, u8); 6] = [
            (242, 109, 120), // 红
            (240, 193, 78),  // 黄
            (99, 199, 99),   // 绿
            (78, 205, 196),  // 青
            (112, 178, 255), // 蓝
            (170, 122, 214), // 紫
        ];
        let (r, g, b) = PALETTE[index % PALETTE.len()];
        Color::from_rgb_u8(r, g, b)
    }

    /// 设置通道曲线颜色来源（自动彩虹或用户设置）。
    ///
    /// TODO: 用户设置颜色尚未完成——调色盘未实现，user-color 仅来自 push_channel 传入的初始值。
    pub fn set_curve_color_mode(&mut self, curve: usize, mode: SlintColorMode) -> Vec<Tag> {
        if curve >= self.channels_data.len() {
            return Vec::new();
        }
        let Some(ch) = self.channels_data.get_mut(curve) else {
            return Vec::new();
        };
        ch.color_mode = mode;
        ch.color = if mode == SlintColorMode::AutoRainbow {
            Self::rainbow_color(curve)
        } else {
            ch.user_color
        };
        vec![Tag::AnimationChannelChanged { curve }]
    }

    /// 切换通道可见性（眼睛）。数据层翻转；连接线/汇总由投影增量重建。
    pub fn toggle_channel_visible(&mut self, curve: usize) -> Vec<Tag> {
        let Some(ch) = self.channels_data.get_mut(curve) else {
            return Vec::new();
        };
        ch.visible = !ch.visible;
        vec![Tag::AnimationChannelChanged { curve }]
    }

    pub fn toggle_channel_modifier(&mut self, curve: usize) -> Vec<Tag> {
        let Some(ch) = self.channels_data.get_mut(curve) else {
            return Vec::new();
        };
        ch.modifier_enabled = !ch.modifier_enabled;
        vec![Tag::AnimationChannelChanged { curve }]
    }

    pub fn toggle_channel_active(&mut self, curve: usize) -> Vec<Tag> {
        let Some(ch) = self.channels_data.get_mut(curve) else {
            return Vec::new();
        };
        ch.active = !ch.active;
        vec![Tag::AnimationChannelChanged { curve }]
    }

    pub fn toggle_channel_editable(&mut self, curve: usize) -> Vec<Tag> {
        let Some(ch) = self.channels_data.get_mut(curve) else {
            return Vec::new();
        };
        ch.editable = !ch.editable;
        vec![Tag::AnimationChannelChanged { curve }]
    }

    /// 视图缩放变化：更新命中检测容差（缩放越大，像素容差越小，判定越精确）。
    /// 纯 UI 态，不影响派生层，无 tag。
    pub fn set_view(&mut self, zoom_x: f32, zoom_y: f32) -> Vec<Tag> {
        if !zoom_x.is_finite() || zoom_x <= 0.0 {
            return Vec::new();
        }
        self.time_zoom = zoom_x;
        self.value_zoom = zoom_y.max(0.01);
        Vec::new()
    }
}
