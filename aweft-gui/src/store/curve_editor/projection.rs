//! 投影：把数据层（`channels_data`/`editing_keyframes`/选中集/搜索/标量）同步到
//! 派生层（Slint 绑定模型 + 标量）。由 depsgraph 的 `op_animation_*` 调用。
//!
//! 投影不修改数据层，只读数据算派生状态，并与当前 UI 模型 diff 后增量更新
//! （`set_row_data` / `insert` / `remove`），对齐 LineEditor 投影规范。

use std::collections::HashSet;

use slint::Model;

use super::CurveEditorState;
use crate::bridge::converters::build_action_menu_items;
use crate::bridge::types::{SlintBezierHandle, SlintKeyframe};
use crate::depsgraph::Tag;
use slint::SharedString;

impl CurveEditorState {
    /// 投影入口：按 tag 语义分派到对应的派生层更新。
    /// 每个 op（`op_animation_*`）先过滤出自己处理的 tag 再调用这里。
    pub(crate) fn apply_tags(&mut self, tags: &[Tag]) {
        for tag in tags {
            match tag {
                Tag::AnimationStructureChanged => self.apply_structure(),
                Tag::AnimationKeyframesChanged {
                    curves,
                    summary_ms,
                    dragged_curve,
                    dragged_summary,
                    handle_preview,
                } => self.apply_keyframes(curves, summary_ms, *dragged_curve, *dragged_summary, handle_preview),
                Tag::AnimationSelectionChanged { summary_ms } => {
                    self.apply_selection(summary_ms)
                }
                Tag::AnimationChannelChanged { curve } => self.apply_channel(*curve),
                Tag::AnimationScalarsChanged => {}
                _ => {}
            }
        }
    }

    // ── 结构变化（装载/重置/搜索） ───────────────────────────────────────

    /// 结构变化：重建 keyframe_models（Slint 嵌套模型）、channels 行（含 keyframes 引用）、
    /// 过滤视图、全部派生模型与 Action 菜单。
    fn apply_structure(&mut self) {
        // 0. 重置后无拖拽上下文：清空拖拽预览
        self.preview_handles.clear();
        // 1. 重建 keyframe_models + channels 行的 keyframes 引用
        self.keyframe_models.clear();
        for (i, ch) in self.channels_data.iter_mut().enumerate() {
            let kfs = self.editing_keyframes.get(&i).cloned().unwrap_or_default();
            let inner: std::rc::Rc<slint::VecModel<SlintKeyframe>> =
                std::rc::Rc::new(slint::VecModel::from(kfs));
            ch.keyframes = slint::ModelRc::from(std::rc::Rc::clone(&inner));
            self.keyframe_models.insert(i, inner);
        }
        self.sync_channel_rows();
        // 2. 过滤视图（graph_segments/graph_handles 的 FilterModel 捕获 matched 集合）
        self.rebuild_filters();
        // 3. 派生模型全量重建
        self.rebuild_all();
        // 4. Action 下拉菜单（由 actions 清单派生）
        self.rebuild_action_menu();
    }

    // ── 关键帧值变化（拖拽/插值/手柄调整） ──────────────────────────────

    /// 关键帧值变化：同步受影响通道的嵌套关键帧模型，重建其曲线段/手柄/连接线，
    /// 重算汇总条带中受影响的 ms 条目；关键帧拖拽时对被拖通道做手柄钳制预览替换。
    fn apply_keyframes(
        &mut self,
        curves: &[usize],
        summary_ms: &[i64],
        dragged_curve: Option<usize>,
        dragged_summary: Option<(usize, f32)>,
        handle_preview: &Option<Vec<(usize, usize, bool)>>,
    ) {
        for &curve in curves {
            // 嵌套模型同步（拖拽期间数量不变，set_row_data 保住 grab）
            self.sync_keyframe_model(curve);
            // 段/手柄/连接线：数量不变原地更新，数量变化 splice
            self.rebuild_curve(curve);
        }
        // 关键帧拖拽的手柄软钳制预览：只对被拖通道替换 handles 区间
        if let Some(curve) = dragged_curve {
            self.rebuild_handles_with_preview(curve);
        }
        // 汇总条带：重算受影响 ms（拖拽含旧 + 新位置）
        for &ms in summary_ms {
            self.summary_update_or_insert(ms);
            self.summary_remove_if_empty(ms);
        }
        // 汇总条带拖拽：被拖标记就地更新时间（不 insert/remove，保住 grab）
        if let Some((index, delta_time)) = dragged_summary
            && let Some(mut sk) = self.summary_keyframes.row_data(index)
        {
            sk.time = super::snap_ms(sk.time + delta_time);
            self.summary_keyframes.set_row_data(index, sk);
        }
        // 手柄拖拽的软钳制预览：重建 preview_handles（只含有效控制点与原始不同的手柄）
        if let Some(handles) = handle_preview {
            let mut preview: Vec<crate::bridge::types::SlintHandleMarker> = Vec::new();
            for &(c, k, is_in) in handles {
                if let Some(marker) = self.dragged_handle_preview(c, k, is_in) {
                    preview.push(marker);
                }
            }
            self.preview_handles.set_vec(preview);
        }
    }

    /// 把通道的关键帧数据同步到嵌套 Slint 模型（逐行 set_row_data，只写变化的行）。
    /// 拖拽期间行数不变；行数变化（结构变化）由 apply_structure 重建。
    fn sync_keyframe_model(&mut self, curve: usize) {
        let Some(data) = self.editing_keyframes.get(&curve).cloned() else {
            return;
        };
        let Some(model) = self.keyframe_models.get(&curve) else {
            return;
        };
        if model.row_count() != data.len() {
            log::warn!(
                "sync_keyframe_model: 通道 {} 行数不一致（模型 {} vs 数据 {}），跳过——应走结构变化",
                curve,
                model.row_count(),
                data.len()
            );
            return;
        }
        for (i, kf) in data.iter().enumerate() {
            if let Some(old) = model.row_data(i) {
                if !keyframes_equal(&old, kf) {
                    model.set_row_data(i, kf.clone());
                }
            }
        }
    }

    // ── 选中变化 ────────────────────────────────────────────────────────

    /// 选中变化：同步 channels 行 selected、selected_display 掩码、嵌套关键帧模型
    /// selected、手柄 selected 视觉，并重算汇总条带受影响 ms 的选中掩码。
    fn apply_selection(&mut self, summary_ms: &[i64]) {
        // 任何选中交互（按下/松开/框选）都是手柄拖拽的终止点：清空拖拽预览
        self.preview_handles.clear();
        self.sync_channel_rows();
        self.sync_selected_display();
        // 嵌套关键帧模型 selected 同步（只写变化的行）
        for (&curve, data) in &self.editing_keyframes {
            if let Some(model) = self.keyframe_models.get(&curve) {
                for (i, kf) in data.iter().enumerate() {
                    if let Some(old) = model.row_data(i)
                        && old.selected != kf.selected
                    {
                        let mut kf2 = old;
                        kf2.selected = kf.selected;
                        model.set_row_data(i, kf2);
                    }
                }
            }
        }
        // 手柄 selected 视觉（扁平模型，逐行更新 selected 字段）
        self.update_handle_selection_visual();
        // 汇总条带：重算受影响 ms 的条目（count/掩码随选中变化）
        for &ms in summary_ms {
            self.summary_update_or_insert(ms);
            self.summary_remove_if_empty(ms);
        }
    }

    // ── 通道属性变化 ────────────────────────────────────────────────────

    /// 通道属性变化：同步该通道行，重建其曲线段/手柄（颜色）与连接线（visible/颜色），
    /// 并重算该通道关键帧所在 ms 的汇总条目（可见性影响聚合）。
    fn apply_channel(&mut self, curve: usize) {
        self.sync_channel_rows();
        self.rebuild_curve(curve);
        if let Some(model) = self.keyframe_models.get(&curve) {
            let affected: HashSet<i64> = (0..model.row_count())
                .filter_map(|i| model.row_data(i).map(|kf| super::ms_of(kf.time)))
                .collect();
            for ms in affected {
                self.summary_update_or_insert(ms);
                self.summary_remove_if_empty(ms);
            }
        }
    }

    /// 更新手柄模型的 selected 字段（按 selected_handles 集合同步视觉）。
    fn update_handle_selection_visual(&mut self) {
        for i in 0..self.handles.row_count() {
            if let Some(mut h) = self.handles.row_data(i) {
                let key = (h.curve_index as usize, h.keyframe_index as usize, h.is_in);
                let new_sel = self.selected_handles.contains(&key);
                if h.selected != new_sel {
                    h.selected = new_sel;
                    self.handles.set_row_data(i, h);
                }
            }
        }
    }

    /// 由 `actions` 清单重建下拉菜单项模型（Action 列表变化时调用）。
    fn rebuild_action_menu(&mut self) {
        let names: Vec<SharedString> = self.actions.iter().map(|a| a.name.clone()).collect();
        self.action_items
            .set_vec(build_action_menu_items(&names));
    }

    // ── 共享同步 ────────────────────────────────────────────────────────

    /// 把数据层的通道行同步到 channels 模型：
    /// 行数不同 → set_vec 全量（仅结构变化时出现）；行数相同 → diff set_row_data（只写变化的行）。
    fn sync_channel_rows(&mut self) {
        if self.channels.row_count() != self.channels_data.len() {
            self.channels.set_vec(self.channels_data.clone());
            return;
        }
        for (i, ch) in self.channels_data.iter().enumerate() {
            let Some(old) = self.channels.row_data(i) else {
                continue;
            };
            if !channel_equal(&old, ch) {
                self.channels.set_row_data(i, ch.clone());
            }
        }
    }
}

/// 关键帧数据是否与模型行相等（字段级比较，Slint 生成类型未实现 PartialEq）。
fn keyframes_equal(a: &SlintKeyframe, b: &SlintKeyframe) -> bool {
    a.time == b.time
        && a.value == b.value
        && a.interpolation == b.interpolation
        && handle_equal(&a.handle_in, &b.handle_in)
        && handle_equal(&a.handle_out, &b.handle_out)
        && a.selected == b.selected
}

fn handle_equal(a: &SlintBezierHandle, b: &SlintBezierHandle) -> bool {
    a.time_offset == b.time_offset && a.value_offset == b.value_offset
}

/// 通道行是否与数据相等（比较除 keyframes 引用外的全部字段）。
fn channel_equal(a: &crate::bridge::types::SlintFCurveChannel, b: &crate::bridge::types::SlintFCurveChannel) -> bool {
    a.curve_index == b.curve_index
        && a.name == b.name
        && a.color == b.color
        && a.user_color == b.user_color
        && a.color_mode == b.color_mode
        && a.visible == b.visible
        && a.modifier_enabled == b.modifier_enabled
        && a.active == b.active
        && a.editable == b.editable
        && a.selected == b.selected
}
