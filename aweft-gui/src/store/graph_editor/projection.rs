//! 投影：把数据层（`channels_data`/`editing_keyframes`/选中集/搜索/标量）同步到
//! 派生层（Slint 绑定模型 + 标量）。由 depsgraph 的 `op_animation_*` 调用。
//!
//! 投影不修改数据层，只读数据算派生状态，并与当前 UI 模型 diff 后增量更新
//! （`set_row_data` / `insert` / `remove`），对齐 Sequence 投影规范。
//!
//! 每个 apply_* 只做"对应 tag 语义"的更新，内部按职责拆成单一目的的私有方法。

use std::collections::HashSet;

use slint::Model;

use super::GraphEditorState;
use crate::bridge::converters::build_action_menu_items;
use crate::bridge::types::{
    SlintBezierHandle, SlintExtrapolation, SlintHandleMarker, SlintInterpolation, SlintKeyframe,
};
use crate::depsgraph::{HandleLoc, Tag};
use slint::SharedString;

impl GraphEditorState {
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
                } => self.apply_keyframes(
                    curves,
                    summary_ms,
                    *dragged_curve,
                    *dragged_summary,
                    handle_preview,
                ),
                Tag::AnimationSelectionChanged { summary_ms } => self.apply_selection(summary_ms),
                Tag::AnimationDragEnded => self.apply_drag_ended(),
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
        self.rebuild_extrapolations();
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
        dragged_summary: Option<crate::depsgraph::SummaryDrag>,
        handle_preview: &Option<Vec<HandleLoc>>,
    ) {
        for &curve in curves {
            // 每帧一次排序：外推线（首末）用；补柄不在此做——拖拽中无必要，
            // 拖拽结束（apply_drag_ended）才为"现在应当有柄但缺失"的侧补默认柄
            let sorted = self.sorted_indices(curve);
            // 嵌套模型同步（拖拽期间数量不变，set_row_data 保住 grab）
            self.sync_keyframe_model(curve);
            // 段/手柄/连接线：数量不变原地更新，数量变化 splice
            self.rebuild_curve(curve);
            // 外推线数据（排序后首末）——拖拽期间实时跟随
            self.update_extrapolation(curve, &sorted);
        }
        // 关键帧拖拽的手柄软钳制预览：只对被拖通道替换 handles 区间
        if let Some(curve) = dragged_curve {
            self.rebuild_handles_with_preview(curve);
        }
        self.update_summary_entries(summary_ms);
        // 汇总条带拖拽：被拖标记就地更新时间（不 insert/remove，保住 grab）
        if let Some(drag) = dragged_summary {
            self.update_summary_drag_time(drag.index, drag.delta_time);
        }
        // 手柄拖拽的软钳制预览：重建 preview_handles
        if let Some(handles) = handle_preview {
            self.rebuild_preview_handles(handles);
        }
    }

    /// 通道关键帧按时间排序后的下标（每帧一次，供 ensure/extrapolation 复用）。
    fn sorted_indices(&self, curve: usize) -> Vec<usize> {
        let Some(kfs) = self.editing_keyframes.get(&curve) else {
            return Vec::new();
        };
        let mut sorted: Vec<usize> = (0..kfs.len()).collect();
        sorted.sort_by(|&a, &b| {
            kfs[a]
                .time
                .partial_cmp(&kfs[b].time)
                .unwrap_or(std::cmp::Ordering::Equal)
        });
        sorted
    }

    /// 为贝塞尔关键帧补缺失侧的默认水平柄（恢复编辑不变量，拖拽结束时调用）。
    ///
    /// 边缘关键帧只有一侧柄（另一侧无角度数据）；拖拽让它变成中间关键帧后，
    /// 新的一侧相邻段若为贝塞尔，该侧手柄缺失（偏移为零，看不见也拖不动）。
    /// 边缘延伸是水平的，因此默认柄也水平延伸 1/2 相邻间距（与插值切换补柄一致）。
    /// 幂等：已补过的（偏移非零）不重复补。扫描全部通道（拖拽结束低频，成本可忽略）。
    fn ensure_default_handles(&mut self) {
        let mut changed: Vec<usize> = Vec::new();
        for curve in 0..self.channels_data.len() {
            if self.ensure_default_handles_for_curve(curve) {
                changed.push(curve);
            }
        }
        // 补柄是数据变化：同步嵌套模型 + 重建段/手柄/连接线
        for curve in changed {
            self.sync_keyframe_model(curve);
            self.rebuild_curve(curve);
        }
    }

    /// 单通道补柄（内部）：复用调用方排序结果；返回是否补了柄。
    fn ensure_default_handles_for_curve(&mut self, curve: usize) -> bool {
        let sorted = self.sorted_indices(curve);
        let Some(kfs) = self.editing_keyframes.get(&curve) else {
            return false;
        };
        // 先收集要补的柄（不可变读），再统一写回（避免借用冲突）
        let mut fixes: Vec<(usize, bool, SlintBezierHandle)> = Vec::new();
        for (pos, &idx) in sorted.iter().enumerate() {
            // 入柄：左侧段是贝塞尔才需要；偏移为零 → 补水平柄
            if pos > 0 {
                let prev = &kfs[sorted[pos - 1]];
                if prev.interpolation == SlintInterpolation::Bezier {
                    let kf = &kfs[idx];
                    if kf.handle_in.time_offset == 0.0 && kf.handle_in.value_offset == 0.0 {
                        fixes.push((
                            idx,
                            true,
                            SlintBezierHandle {
                                time_offset: -(kf.time - prev.time) / 2.0,
                                value_offset: 0.0,
                            },
                        ));
                    }
                }
            }
            // 出柄：本关键帧是贝塞尔且右侧有邻居才需要；偏移为零 → 补水平柄
            if pos + 1 < sorted.len() && kfs[idx].interpolation == SlintInterpolation::Bezier {
                let next = &kfs[sorted[pos + 1]];
                let kf = &kfs[idx];
                if kf.handle_out.time_offset == 0.0 && kf.handle_out.value_offset == 0.0 {
                    fixes.push((
                        idx,
                        false,
                        SlintBezierHandle {
                            time_offset: (next.time - kf.time) / 2.0,
                            value_offset: 0.0,
                        },
                    ));
                }
            }
        }
        if fixes.is_empty() {
            return false;
        }
        if let Some(kfs) = self.editing_keyframes.get_mut(&curve) {
            for (idx, is_in, handle) in fixes {
                if is_in {
                    kfs[idx].handle_in = handle;
                } else {
                    kfs[idx].handle_out = handle;
                }
            }
        }
        true
    }

    /// 把通道的关键帧数据同步到嵌套 Slint 模型（逐行 set_row_data，只写变化的行）。
    /// 拖拽期间行数不变；行数变化（结构变化）由 apply_structure 重建。
    fn sync_keyframe_model(&mut self, curve: usize) {
        let Some(data) = self.editing_keyframes.get(&curve) else {
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

    /// 重建单通道的外推线数据：按时间排序取首/末关键帧。
    /// 拖拽期间关键帧模型保序（下标 0 不一定是时间最小），外推线必须读排序结果。
    /// 复用调用方的排序结果（`sorted` = `sorted_indices` 输出），不再重复排序。
    fn update_extrapolation(&mut self, curve: usize, sorted: &[usize]) {
        let Some(kfs) = self.editing_keyframes.get(&curve) else {
            return;
        };
        if sorted.is_empty() {
            return;
        }
        let first = &kfs[sorted[0]];
        let last = &kfs[*sorted.last().unwrap()];
        let entry = SlintExtrapolation {
            first_time: first.time,
            first_value: first.value,
            last_time: last.time,
            last_value: last.value,
        };
        self.extrapolations.set_row_data(curve, entry);
    }

    /// 全量重建外推线数据（结构变化时调用；行数 = 通道数）。
    fn rebuild_extrapolations(&mut self) {
        let n = self.channels_data.len();
        let mut items: Vec<SlintExtrapolation> = Vec::with_capacity(n);
        for curve in 0..n {
            let Some(kfs) = self.editing_keyframes.get(&curve) else {
                items.push(SlintExtrapolation {
                    first_time: 0.0,
                    first_value: 0.0,
                    last_time: 0.0,
                    last_value: 0.0,
                });
                continue;
            };
            if kfs.is_empty() {
                items.push(SlintExtrapolation {
                    first_time: 0.0,
                    first_value: 0.0,
                    last_time: 0.0,
                    last_value: 0.0,
                });
                continue;
            }
            let mut sorted: Vec<&SlintKeyframe> = kfs.iter().collect();
            sorted.sort_by(|a, b| {
                a.time
                    .partial_cmp(&b.time)
                    .unwrap_or(std::cmp::Ordering::Equal)
            });
            items.push(SlintExtrapolation {
                first_time: sorted[0].time,
                first_value: sorted[0].value,
                last_time: sorted[sorted.len() - 1].time,
                last_value: sorted[sorted.len() - 1].value,
            });
        }
        self.extrapolations.set_vec(items);
    }

    // ── 选中变化 ────────────────────────────────────────────────────────

    /// 选中变化：选中视觉同步 + 汇总掩码重算（拖拽整理见 apply_drag_ended）。
    /// 任何选中交互（按下/松开/框选）都是手柄拖拽的终止点：清空拖拽预览。
    fn apply_selection(&mut self, summary_ms: &[i64]) {
        self.preview_handles.clear();
        self.sync_channel_rows();
        self.sync_selected_display();
        self.sync_keyframe_selection();
        self.update_handle_selection_visual();
        self.update_summary_entries(summary_ms);
    }

    /// 拖拽结束整理：恢复关键帧拖拽期间被 preview 替换的 handles 区间 +
    /// 重排乱序关键帧 + 为"现在应当有柄但缺失"的贝塞尔关键帧补默认水平柄。
    /// 独立于选中变化（独立 tag）——其他组件可单独依赖本整理。
    /// 注意：只由关键帧拖拽结束（keyframe_released moved=true）触发；
    /// 手柄拖拽结束不打本 tag（手柄可能被故意钳到 0，不补柄）。
    fn apply_drag_ended(&mut self) {
        self.restore_handle_preview();
        self.resort_keyframes();
        self.ensure_default_handles();
    }

    /// 把关键帧拖拽期间 preview 替换过的 handles 区间恢复为普通手柄（拖拽残留换回）。
    /// preview_handles 模型的清空在 apply_selection（任何选中交互的终止点）。
    fn restore_handle_preview(&mut self) {
        let mut previewed: Vec<usize> = Vec::new();
        for i in 0..self.handles.row_count() {
            if let Some(h) = self.handles.row_data(i)
                && h.preview
            {
                let curve = h.curve_index as usize;
                if !previewed.contains(&curve) {
                    previewed.push(curve);
                }
            }
        }
        for curve in previewed {
            self.rebuild_curve(curve);
        }
    }

    /// 拖拽可能破坏关键帧时间序（模型保序）：松开后重排受影响通道，
    /// 恢复"下标 0 = 时间最小"（嵌套模型与数据同序）。
    /// 拖拽期间（Keyframes tag）不重排——保住 grab 与拖拽抓手下标映射。
    fn resort_keyframes(&mut self) {
        let mut to_resort: Vec<usize> = Vec::new();
        for curve in 0..self.channels_data.len() {
            if let Some(kfs) = self.editing_keyframes.get(&curve)
                && kfs.windows(2).any(|w| w[0].time > w[1].time)
            {
                to_resort.push(curve);
            }
        }
        for curve in to_resort {
            if let Some(kfs) = self.editing_keyframes.get_mut(&curve) {
                kfs.sort_by(|a, b| {
                    a.time
                        .partial_cmp(&b.time)
                        .unwrap_or(std::cmp::Ordering::Equal)
                });
            }
            if let Some(model) = self.keyframe_models.get(&curve) {
                let kfs = self
                    .editing_keyframes
                    .get(&curve)
                    .cloned()
                    .unwrap_or_default();
                model.set_vec(kfs);
            }
            // 关键帧下标变化 → 段/手柄/连接线重建（keyframe_index 字段跟随）
            self.rebuild_curve(curve);
        }
    }

    /// 同步嵌套关键帧模型的 selected 字段（diff 只写变化的行）。
    fn sync_keyframe_selection(&mut self) {
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
    }

    /// 重算汇总条带中受影响 ms 的条目（count/掩码随数据与选中变化）。
    fn update_summary_entries(&mut self, summary_ms: &[i64]) {
        for &ms in summary_ms {
            self.summary_update_or_insert(ms);
            self.summary_remove_if_empty(ms);
        }
    }

    /// 汇总条带拖拽：被拖标记就地更新时间（不 insert/remove，保住 grab）。
    fn update_summary_drag_time(&mut self, index: usize, delta_time: f32) {
        if let Some(mut sk) = self.summary_keyframes.row_data(index) {
            sk.time = super::snap_ms(sk.time + delta_time);
            self.summary_keyframes.set_row_data(index, sk);
        }
    }

    /// 手柄拖拽的软钳制预览：重建 preview_handles（只含有效控制点与原始不同的手柄）。
    fn rebuild_preview_handles(&mut self, handles: &[HandleLoc]) {
        let mut preview: Vec<SlintHandleMarker> = Vec::new();
        for &loc in handles {
            if let Some(marker) = self.dragged_handle_preview(loc.curve, loc.key, loc.is_in) {
                preview.push(marker);
            }
        }
        self.preview_handles.set_vec(preview);
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
            self.update_summary_entries(&affected.into_iter().collect::<Vec<i64>>());
        }
    }

    /// 更新手柄模型的 selected 字段（按 selected_handles 集合同步视觉）。
    fn update_handle_selection_visual(&mut self) {
        for i in 0..self.handles.row_count() {
            if let Some(mut h) = self.handles.row_data(i) {
                let loc = HandleLoc {
                    curve: h.curve_index as usize,
                    key: h.keyframe_index as usize,
                    is_in: h.is_in,
                };
                let new_sel = self.selected_handles.contains(&loc);
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
        self.action_items.set_vec(build_action_menu_items(&names));
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
fn channel_equal(
    a: &crate::bridge::types::SlintFCurveChannel,
    b: &crate::bridge::types::SlintFCurveChannel,
) -> bool {
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
