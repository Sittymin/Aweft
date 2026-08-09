//! 曲线编辑器（Graph Editor）Rust 侧：持有 FCurve 通道视图模型与全部编辑操作。
//!
//! UI 先行阶段：数据为临时测试数据（由 `window_manager::create_test_animation_data`
//! 填充），尚未接线 aweft-model 的 `animation::Action/FCurve/Keyframe`。
//! 结构与 model 对应：`SlintFCurveChannel` ↔ `ActionCurve + FCurve`，
//! `SlintKeyframe` ↔ `Keyframe`（含 `BezierHandle` / `Interpolation`）。
//!
//! 权威数据存在 `keyframe_models`（每个通道一个稳定的 `VecModel<SlintKeyframe>`）；
//! `channels` 是绑定 Slint 的投影（其 `keyframes` 字段指向同一 inner 模型）。
//! `segments` / `handles` / 标尺刻度为派生数据，随编辑局部更新（`set_row_data`），
//! 避免拖拽中重建交互组件。

use std::collections::HashMap;
use std::rc::Rc;

use slint::{Color, Model, ModelRc, SharedString, VecModel};

use crate::bridge::types::{
    SlintBezierHandle, SlintCurveSegment, SlintFCurveChannel, SlintHandleMarker, SlintInterpolation,
    SlintKeyframe, SlintRulerTick, SlintSegmentKind,
};

/// 曲线编辑器状态
pub struct CurveEditorState {
    /// 通道视图模型（绑定 Slint；`keyframes` 指向 `keyframe_models` 中同一 inner 模型）
    pub channels: Rc<VecModel<SlintFCurveChannel>>,
    /// 派生曲线段（Path 逐段渲染）
    pub segments: Rc<VecModel<SlintCurveSegment>>,
    /// 派生贝塞尔控制柄
    pub handles: Rc<VecModel<SlintHandleMarker>>,
    /// 时间标尺刻度
    pub time_ticks: Rc<VecModel<SlintRulerTick>>,
    /// 值标尺刻度
    pub value_ticks: Rc<VecModel<SlintRulerTick>>,
    /// 当前时间（播放头）
    pub current_time: f64,
    /// 当前 Action 名称（header 栏中区显示；UI 先行阶段为占位，后续接 model 的 Action 名）
    pub action_name: SharedString,
    /// Action 时长范围（画布明暗分段用，由关键帧时间推导）
    pub action_start: f32,
    pub action_end: f32,
    /// 当前激活通道（点击曲线/关键帧/控制柄/通道列表行设置；-1 = 无激活；侧边栏据此显示哪条曲线）
    pub current_channel: isize,
    /// Shift 区间选择的锚点（最近一次普通/Shift 点击的通道；Ctrl 切换不更新）
    selection_anchor: isize,
    /// 手柄拖拽时相邻关键帧软钳制的预览：被拖柄的有效控制点（与原始位置不同）
    pub preview_handles: Rc<VecModel<SlintHandleMarker>>,
    /// 当前视图缩放（px/单位）：刻度步长随缩放动态选取（放大细化、缩小变粗）
    pub time_zoom: f32,
    pub value_zoom: f32,
    /// 视图平移（px）与可视尺寸（px）：刻度覆盖整个可见区域（标尺满屏、数量有界）
    pub pan_x: f32,
    pub pan_y: f32,
    pub view_width: f32,
    pub view_height: f32,

    /// 权威关键帧数据：通道下标 → 稳定的 inner VecModel
    keyframe_models: HashMap<usize, Rc<VecModel<SlintKeyframe>>>,
    /// 曲线段在 segments 模型中的偏移：offset[k] = 第 k 个通道的段起始下标（长度 = 通道数+1）
    segment_offsets: Vec<usize>,
    /// 控制柄在 handles 模型中的偏移（同上）
    handle_offsets: Vec<usize>,
}

impl Default for CurveEditorState {
    fn default() -> Self {
        Self {
            channels: Rc::new(VecModel::default()),
            segments: Rc::new(VecModel::default()),
            handles: Rc::new(VecModel::default()),
            time_ticks: Rc::new(VecModel::default()),
            value_ticks: Rc::new(VecModel::default()),
            current_time: 0.0,
            action_name: "主 Action".into(),
            action_start: 0.0,
            action_end: 0.0,
            current_channel: -1,
            selection_anchor: -1,
            preview_handles: Rc::new(VecModel::default()),
            // 与 Slint GraphEditor 的 nav-zoom-x/y 初始值一致（见 graph_editor.slint）
            time_zoom: 30.0,
            value_zoom: 2.0,
            pan_x: 0.0,
            pan_y: 0.0,
            // 启动时尚未拿到画布尺寸，先用常规屏幕尺寸兜底生成首屏刻度
            view_width: 800.0,
            view_height: 600.0,
            keyframe_models: HashMap::new(),
            segment_offsets: Vec::new(),
            handle_offsets: Vec::new(),
        }
    }
}

impl CurveEditorState {
    /// 追加一个通道（含关键帧）。通道内关键帧按 time 排序（调用方保证）。
    /// 颜色默认“自动彩虹”：显示色取自彩虹调色板（按通道下标），传入的 color 作为用户色。
    ///
    /// 注意：`color` 会被当作「用户设置」模式的用户色保存；由于调色盘尚未实现，
    /// 这目前也是用户色唯一的来源（测试数据传显式颜色即为此）。
    pub fn push_channel(&mut self, name: impl Into<SharedString>, color: Color, keyframes: Vec<SlintKeyframe>) {
        let idx = self.keyframe_models.len();
        let inner: Rc<VecModel<SlintKeyframe>> = Rc::new(VecModel::from(keyframes));
        self.keyframe_models.insert(idx, Rc::clone(&inner));
        self.channels.push(SlintFCurveChannel {
            name: name.into(),
            color: Self::rainbow_color(idx),
            user_color: color,
            color_mode: 0, // 自动彩虹
            visible: true,
            modifier_enabled: true,
            active: true,
            editable: true,
            selected: false,
            keyframes: ModelRc::from(inner),
        });
    }

    /// 全量重算派生数据（新增/删除通道或关键帧后调用）。
    pub fn rebuild_all(&mut self) {
        let n = self.keyframe_models.len();
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
        self.rebuild_action_range();
        self.rebuild_time_ticks();
        self.rebuild_value_ticks();
    }

    /// 由全部关键帧时间推导 Action 时长范围（时长内亮、时长外暗）。
    fn rebuild_action_range(&mut self) {
        let mut min_t = f32::INFINITY;
        let mut max_t = f32::NEG_INFINITY;
        for model in self.keyframe_models.values() {
            for i in 0..model.row_count() {
                if let Some(kf) = model.row_data(i) {
                    min_t = min_t.min(kf.time);
                    max_t = max_t.max(kf.time);
                }
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

    /// 计算一个通道的曲线段与控制柄（不修改任何模型，供重建用）。
    ///
    /// 关键帧可能因拖拽而暂时乱序：这里先按时间排序再推导曲线段/端点手柄，
    /// 保证曲线始终是时间的函数；但模型顺序不变（拖拽抓手仍按原模型下标命中）。
    fn curve_elements(
        &self,
        curve: usize,
    ) -> (Vec<SlintCurveSegment>, Vec<SlintHandleMarker>) {
        let Some(kf_model) = self.keyframe_models.get(&curve) else {
            return (Vec::new(), Vec::new());
        };
        let color = self
            .channels
            .row_data(curve)
            .map(|c| c.color)
            .unwrap_or_else(|| Color::from_rgb_u8(200, 200, 200));

        // (模型下标, 关键帧)，按时间排序
        let mut kfs: Vec<(usize, SlintKeyframe)> = (0..kf_model.row_count())
            .filter_map(|i| kf_model.row_data(i).map(|k| (i, k)))
            .collect();
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
                });
            }
        }
        (segments, handles)
    }

    /// 通道关键帧是否按时间有序（拖拽可能破坏顺序）。
    fn is_sorted(&self, curve: usize) -> bool {
        let Some(model) = self.keyframe_models.get(&curve) else {
            return true;
        };
        for i in 1..model.row_count() {
            if let (Some(a), Some(b)) = (model.row_data(i - 1), model.row_data(i)) {
                if a.time > b.time {
                    return false;
                }
            }
        }
        true
    }

    /// 局部更新一个通道的曲线段与控制柄（结构不变时使用，避免重建交互组件）。
    fn rebuild_curve(&mut self, curve: usize) {
        if self.segment_offsets.len() <= curve + 1 {
            return;
        }
        let (segments, handles) = self.curve_elements(curve);

        let sg = self.segment_offsets[curve];
        let se = self.segment_offsets[curve + 1];
        if se - sg == segments.len() {
            for (k, seg) in segments.iter().enumerate() {
                self.segments.set_row_data(sg + k, seg.clone());
            }
        }
        let hg = self.handle_offsets[curve];
        let he = self.handle_offsets[curve + 1];
        if he - hg == handles.len() {
            for (k, h) in handles.iter().enumerate() {
                self.handles.set_row_data(hg + k, h.clone());
            }
        }
    }

    // ── 交互回调 ──────────────────────────────────────────────

    /// 通道是否可编辑（锁）。
    fn channel_editable(&self, curve: usize) -> bool {
        self.channels.row_data(curve).map(|c| c.editable).unwrap_or(false)
    }

    /// 关键帧按下：激活所在通道 + 选中（additive=Shift/Ctrl 追加）。锁定通道不可交互。
    pub fn keyframe_pressed(&mut self, curve: usize, key: usize, additive: bool) {
        if !self.channel_editable(curve) {
            return;
        }
        self.preview_handles.set_vec(Vec::new());
        // 关键帧所属通道也纳入选中：普通点击只选该通道，追加选择保证该通道被选中
        if additive {
            self.set_channel_selected(curve, true);
        } else {
            self.clear_channel_selection();
            self.set_channel_selected(curve, true);
            self.selection_anchor = curve as isize;
        }
        self.current_channel = curve as isize;
        if !additive {
            self.deselect_all();
        }
        self.set_keyframe_selected(curve, key, additive);
    }

    /// 关键帧拖拽：把增量累加到当前关键帧位置。
    /// 增量由 Slint 按“光标相对按下位置的位移 / 缩放”计算（与节点拖拽一致），
    /// 累加（而非以按下点为基准重算）可避免标记与光标之间的反馈振荡。
    /// 若拖过了相邻关键帧导致乱序，则全量重建（曲线按时间排序重绘，仍是函数）。
    /// 移动中用“渲染期钳制”后的有效控制点替换被钳制的手柄（preview 标记）。
    pub fn keyframe_dragged(&mut self, curve: usize, key: usize, delta_time: f32, delta_value: f32) {
        if !self.channel_editable(curve) {
            return;
        }
        if !delta_time.is_finite() || !delta_value.is_finite() {
            return;
        }
        let Some(kf_model) = self.keyframe_models.get(&curve) else {
            return;
        };
        let Some(mut kf) = kf_model.row_data(key) else {
            return;
        };
        kf.time += delta_time;
        kf.value += delta_value;
        kf_model.set_row_data(key, kf);

        if self.is_sorted(curve) {
            self.rebuild_curve(curve);
        } else {
            self.rebuild_all();
        }
        self.rebuild_handles_with_preview(curve, key);
    }

    /// 关键帧松开：恢复普通手柄。
    pub fn keyframe_released(&mut self, _curve: usize, _key: usize) {
        self.rebuild_all();
    }

    /// 计算被拖关键帧两侧相邻段的“渲染期钳制后”有效控制点中，与被拖手柄相关且确实
    /// 被钳制（有效 ≠ 原始）的那些，映射 (模型下标, is_in) → 有效控制点。
    fn clamped_overrides(&self, curve: usize, key: usize) -> HashMap<(usize, bool), (f32, f32)> {
        let mut map = HashMap::new();
        let Some(model) = self.keyframe_models.get(&curve) else {
            return map;
        };
        let color = self
            .channels
            .row_data(curve)
            .map(|c| c.color)
            .unwrap_or_else(|| Color::from_rgb_u8(200, 200, 200));
        let mut kfs: Vec<(usize, SlintKeyframe)> = (0..model.row_count())
            .filter_map(|i| model.row_data(i).map(|k| (i, k)))
            .collect();
        kfs.sort_by(|a, b| a.1.time.partial_cmp(&b.1.time).unwrap_or(std::cmp::Ordering::Equal));
        let Some(pos) = kfs.iter().position(|&(i, _)| i == key) else {
            return map;
        };
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

        // 左侧段 [prev, dragged]：仅当 prev 是贝塞尔时该段才是贝塞尔，P1/P2 才参与钳制
        if pos > 0 && kfs[pos - 1].1.interpolation == SlintInterpolation::Bezier {
            let seg = make_segment(curve, color, &kfs[pos - 1].1, &kfs[pos].1);
            check(&mut map, kfs[pos - 1].0, false, &kfs[pos - 1].1, (seg.cx1, seg.cy1));
            check(&mut map, kfs[pos].0, true, &kfs[pos].1, (seg.cx2, seg.cy2));
        }
        // 右侧段 [dragged, next]：仅当 dragged 是贝塞尔时该段才是贝塞尔
        if pos + 1 < kfs.len() && kfs[pos].1.interpolation == SlintInterpolation::Bezier {
            let seg = make_segment(curve, color, &kfs[pos].1, &kfs[pos + 1].1);
            check(&mut map, kfs[pos].0, false, &kfs[pos].1, (seg.cx1, seg.cy1));
            check(&mut map, kfs[pos + 1].0, true, &kfs[pos + 1].1, (seg.cx2, seg.cy2));
        }
        map
    }

    /// 被拖通道按 preview 覆盖生成手柄：被钳制的手柄用有效控制点 + preview 标记替换；
    /// 若该手柄正是正在被拖的手柄（handle_dragging），则同时输出原始手柄（两者都显示）。
    fn channel_handles_with_preview(
        &self,
        curve: usize,
        overrides: &HashMap<(usize, bool), (f32, f32)>,
    ) -> Vec<SlintHandleMarker> {
        let Some(model) = self.keyframe_models.get(&curve) else {
            return Vec::new();
        };
        let color = self
            .channels
            .row_data(curve)
            .map(|c| c.color)
            .unwrap_or_else(|| Color::from_rgb_u8(200, 200, 200));
        let mut kfs: Vec<(usize, SlintKeyframe)> = (0..model.row_count())
            .filter_map(|i| model.row_data(i).map(|k| (i, k)))
            .collect();
        kfs.sort_by(|a, b| a.1.time.partial_cmp(&b.1.time).unwrap_or(std::cmp::Ordering::Equal));
        let last = kfs.len().saturating_sub(1);

        let mut out = Vec::new();
        for (pos, (midx, kf)) in kfs.iter().enumerate() {
            let key = *midx;
            // 入柄：仅当左侧段是贝塞尔（前一关键帧是贝塞尔）
            if pos > 0 && kfs[pos - 1].1.interpolation == SlintInterpolation::Bezier {
                let raw = (kf.time + kf.handle_in.time_offset, kf.value + kf.handle_in.value_offset);
                match overrides.get(&(key, true)) {
                    Some(&eff) => out.push(mk_marker(curve, key, color, kf, eff.0, eff.1, true, true)),
                    None => out.push(mk_marker(curve, key, color, kf, raw.0, raw.1, true, false)),
                }
            }
            // 出柄：本关键帧是贝塞尔 → 右侧段为贝塞尔
            if pos < last && kf.interpolation == SlintInterpolation::Bezier {
                let raw = (kf.time + kf.handle_out.time_offset, kf.value + kf.handle_out.value_offset);
                match overrides.get(&(key, false)) {
                    Some(&eff) => out.push(mk_marker(curve, key, color, kf, eff.0, eff.1, false, true)),
                    None => out.push(mk_marker(curve, key, color, kf, raw.0, raw.1, false, false)),
                }
            }
        }
        out
    }

    /// 重建 handles 模型：被拖通道的钳制手柄替换为 preview，其余通道保持普通。
    fn rebuild_handles_with_preview(&mut self, curve: usize, key: usize) {
        let overrides = self.clamped_overrides(curve, key);
        let n = self.keyframe_models.len();
        let mut handles: Vec<SlintHandleMarker> = Vec::new();
        let mut hdl_offsets = vec![0usize; n + 1];
        for c in 0..n {
            let h = if c == curve {
                self.channel_handles_with_preview(c, &overrides)
            } else {
                self.curve_elements(c).1
            };
            handles.extend(h);
            hdl_offsets[c + 1] = handles.len();
        }
        self.handle_offsets = hdl_offsets;
        self.handles.set_vec(handles);
    }

    /// 设置单个关键帧的选中状态（additive=true 时取反，否则置为选中）。
    fn set_keyframe_selected(&mut self, curve: usize, key: usize, additive: bool) {
        let Some(kf_model) = self.keyframe_models.get(&curve) else {
            return;
        };
        let Some(mut kf) = kf_model.row_data(key) else {
            return;
        };
        kf.selected = if additive { !kf.selected } else { true };
        kf_model.set_row_data(key, kf);
        self.rebuild_curve(curve);
    }

    /// 清除全部关键帧选中。
    pub fn deselect_all(&mut self) {
        for model in self.keyframe_models.values() {
            for i in 0..model.row_count() {
                let Some(mut kf) = model.row_data(i) else {
                    continue;
                };
                if kf.selected {
                    kf.selected = false;
                    model.set_row_data(i, kf);
                }
            }
        }
    }

    // ── 贝塞尔控制柄 ─────────────────────────────────────────

    /// 控制柄按下：激活所在通道。锁定通道不可交互。
    pub fn handle_pressed(&mut self, curve: usize, _key: usize, _is_in: bool) {
        if !self.channel_editable(curve) {
            return;
        }
        // 拖控制柄也聚焦并选中该通道（曲线保持高亮）
        self.channel_pressed(curve, 0);
    }

    // ── 通道多选 ────────────────────────────────────────────

    fn channel_selected(&self, curve: usize) -> bool {
        self.channels.row_data(curve).map(|c| c.selected).unwrap_or(false)
    }

    fn set_channel_selected(&mut self, curve: usize, selected: bool) {
        let Some(mut ch) = self.channels.row_data(curve) else {
            return;
        };
        if ch.selected != selected {
            ch.selected = selected;
            self.channels.set_row_data(curve, ch);
        }
    }

    fn clear_channel_selection(&mut self) {
        for i in 0..self.channels.row_count() {
            self.set_channel_selected(i, false);
        }
    }

    /// 通道/曲线按下，mode：0=普通（只选中该通道），1=Shift（区间选择 anchor→curve），
    /// 2=Ctrl（切换该通道选中）。点击曲线本体或列表行都走这里。
    pub fn channel_pressed(&mut self, curve: usize, mode: i32) {
        if curve >= self.channels.row_count() {
            return;
        }
        match mode {
            // Shift：从锚点（最近一次普通/Shift 点击）到当前通道全选中，其余保留
            1 => {
                let anchor = if (0..self.channels.row_count() as isize).contains(&self.selection_anchor)
                {
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
            2 => {
                let new_sel = !self.channel_selected(curve);
                self.set_channel_selected(curve, new_sel);
                if new_sel {
                    self.current_channel = curve as isize;
                }
                // Ctrl 不更新区间锚点
            }
            // 普通：清除全部选中，只选当前通道
            _ => {
                self.clear_channel_selection();
                self.set_channel_selected(curve, true);
                self.selection_anchor = curve as isize;
                self.current_channel = curve as isize;
            }
        }
    }

    /// 已选中通道数量。
    pub fn selected_channel_count(&self) -> usize {
        (0..self.channels.row_count())
            .filter(|&i| self.channel_selected(i))
            .count()
    }

    /// 清除激活通道与全部通道选中（画布空白点击）。
    pub fn deselect_channel(&mut self) {
        self.current_channel = -1;
        self.selection_anchor = -1;
        self.clear_channel_selection();
    }

    /// 自动彩虹调色板：按通道下标取色（彩虹色系）。
    fn rainbow_color(index: usize) -> Color {
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

    /// 设置通道曲线颜色来源：0=自动彩虹（显示色 = 调色板按下标取），1=用户设置（显示色 = user-color）。
    ///
    /// 注意：「用户设置」尚未完成 —— 还没有弹出调色盘去修改用户颜色；目前 user-color 只来自
    /// `push_channel` 传入的初始颜色（测试数据），切换到该模式即回落到此颜色。后续接入
    /// 调色盘后，新增「修改 user-color 并刷新段颜色」的入口即可。
    pub fn set_curve_color_mode(&mut self, curve: usize, mode: i32) {
        if curve >= self.channels.row_count() {
            return;
        }
        let mode = if mode == 0 { 0 } else { 1 };
        let Some(mut ch) = self.channels.row_data(curve) else {
            return;
        };
        ch.color_mode = mode;
        ch.color = if mode == 0 {
            Self::rainbow_color(curve)
        } else {
            ch.user_color
        };
        self.channels.set_row_data(curve, ch);
        // 段/手柄颜色随通道色变化（结构不变，可局部更新）
        self.rebuild_curve(curve);
    }

    /// 设置全部选中关键帧的插值类型（0=贝塞尔，1=线性，2=常量）。选中集为空则忽略。
    /// 插值变化会改变手柄数量，需要全量重建。
    pub fn set_selected_keyframes_interpolation(&mut self, interp: i32) {
        let interpolation = match interp {
            0 => SlintInterpolation::Bezier,
            1 => SlintInterpolation::Linear,
            _ => SlintInterpolation::Hold,
        };
        let mut changed = false;
        for model in self.keyframe_models.values() {
            // 按时间排序（模型下标, 时间），用于给新贝塞尔关键帧生成默认控制柄
            let mut sorted: Vec<(usize, f32)> = (0..model.row_count())
                .filter_map(|i| model.row_data(i).map(|k| (i, k.time)))
                .collect();
            sorted.sort_by(|a, b| a.1.partial_cmp(&b.1).unwrap_or(std::cmp::Ordering::Equal));

            // 第一步：线性/常量 → 贝塞尔的关键帧补默认控制柄（原控制柄为 0，否则看不见也拖不动）
            if interpolation == SlintInterpolation::Bezier {
                for i in 0..model.row_count() {
                    let Some(kf) = model.row_data(i) else {
                        continue;
                    };
                    if !kf.selected || kf.interpolation == interpolation {
                        continue;
                    }
                    let Some(pos) = sorted.iter().position(|(mi, _)| *mi == i) else {
                        continue;
                    };
                    let t = sorted[pos].1;
                    let prev_t = if pos > 0 { sorted[pos - 1].1 } else { t };
                    let next_t = if pos + 1 < sorted.len() { sorted[pos + 1].1 } else { t };
                    let mut kf = kf;
                    // 入柄：仅有左侧段是贝塞尔时才会显示；这里预置，右侧邻居切贝塞尔时即可用
                    if pos > 0 && kf.handle_in.time_offset == 0.0 && kf.handle_in.value_offset == 0.0 {
                        kf.handle_in = SlintBezierHandle {
                            time_offset: -(t - prev_t) / 3.0,
                            value_offset: 0.0,
                        };
                    }
                    // 出柄：本关键帧为贝塞尔 → 右侧段为贝塞尔，默认外伸 1/3 间距
                    if pos + 1 < sorted.len()
                        && kf.handle_out.time_offset == 0.0
                        && kf.handle_out.value_offset == 0.0
                    {
                        kf.handle_out = SlintBezierHandle {
                            time_offset: (next_t - t) / 3.0,
                            value_offset: 0.0,
                        };
                    }
                    model.set_row_data(i, kf);
                }
            }

            // 第二步：写回插值类型
            for i in 0..model.row_count() {
                let Some(mut kf) = model.row_data(i) else {
                    continue;
                };
                if kf.selected && kf.interpolation != interpolation {
                    kf.interpolation = interpolation;
                    model.set_row_data(i, kf);
                    changed = true;
                }
            }
        }
        if changed {
            self.rebuild_all();
        }
    }

    /// 全部选中关键帧的数量（跨通道）。
    pub fn selected_keyframe_count(&self) -> usize {
        let mut n = 0;
        for model in self.keyframe_models.values() {
            for i in 0..model.row_count() {
                if model.row_data(i).map(|k| k.selected).unwrap_or(false) {
                    n += 1;
                }
            }
        }
        n
    }

    /// 首个选中关键帧的插值类型下标（0=贝塞尔，1=线性，2=常量；无选中返回 -1）。
    pub fn selected_interpolation_index(&self) -> i32 {
        for model in self.keyframe_models.values() {
            for i in 0..model.row_count() {
                if let Some(kf) = model.row_data(i) {
                    if kf.selected {
                        return match kf.interpolation {
                            SlintInterpolation::Bezier => 0,
                            SlintInterpolation::Linear => 1,
                            SlintInterpolation::Hold => 2,
                        };
                    }
                }
            }
        }
        -1
    }

    /// 控制柄拖拽：把增量累加到被拖控制柄，并按 mode 约束。
    /// mode: 0=Ctrl 自由（对手柄不动），1=默认对齐（被拖柄自由，对手柄旋转共线、
    ///       世界长度不变），2=Shift 原点对称（对手柄 = -被拖柄）。
    pub fn handle_dragged(&mut self, curve: usize, key: usize, is_in: bool, dt: f32, dv: f32, mode: i32) {
        if !self.channel_editable(curve) {
            return;
        }
        if !dt.is_finite() || !dv.is_finite() {
            return;
        }
        let Some(kf_model) = self.keyframe_models.get(&curve) else {
            return;
        };
        let Some(mut kf) = kf_model.row_data(key) else {
            return;
        };

        // 被拖手柄累加增量（自由移动，角度与长度都可变）
        let (nt, nv) = if is_in {
            kf.handle_in.time_offset += dt;
            kf.handle_in.value_offset += dv;
            (kf.handle_in.time_offset, kf.handle_in.value_offset)
        } else {
            kf.handle_out.time_offset += dt;
            kf.handle_out.value_offset += dv;
            (kf.handle_out.time_offset, kf.handle_out.value_offset)
        };

        match mode {
            // 对齐：被拖柄自由；对手柄旋转到与被拖柄共线（角度相反），世界长度保持自身
            1 => {
                let nl = (nt * nt + nv * nv).sqrt();
                if nl > 1e-9 {
                    let len = if is_in {
                        (kf.handle_out.time_offset.powi(2) + kf.handle_out.value_offset.powi(2)).sqrt()
                    } else {
                        (kf.handle_in.time_offset.powi(2) + kf.handle_in.value_offset.powi(2)).sqrt()
                    };
                    let new = SlintBezierHandle { time_offset: -nt / nl * len, value_offset: -nv / nl * len };
                    if is_in {
                        kf.handle_out = new;
                    } else {
                        kf.handle_in = new;
                    }
                }
            }
            // 原点对称：对手柄 = -被拖柄（角度+长度都镜像）
            2 => {
                if is_in {
                    kf.handle_out = SlintBezierHandle { time_offset: -nt, value_offset: -nv };
                } else {
                    kf.handle_in = SlintBezierHandle { time_offset: -nt, value_offset: -nv };
                }
            }
            // 自由 / 其他：对手柄不动
            _ => {}
        }

        // 硬钳制：被拖柄与对手柄都钳到“自己关键帧”一侧；
        // 相邻关键帧边界为软钳制（渲染期交叉钳制处理）
        clamp_handle_own_side(&mut kf, true);
        clamp_handle_own_side(&mut kf, false);
        kf_model.set_row_data(key, kf);
        self.rebuild_curve(curve);
        // 相邻软钳制的预览：被拖柄的有效控制点（若与原始位置不同则显示）
        let preview: Vec<SlintHandleMarker> = self
            .dragged_handle_preview(curve, key, is_in)
            .into_iter()
            .collect();
        self.preview_handles.set_vec(preview);
    }

    /// 计算被拖手柄的“软钳制后”有效控制点预览标记；未被钳制（有效 == 原始）时返回 None。
    fn dragged_handle_preview(&self, curve: usize, key: usize, is_in: bool) -> Option<SlintHandleMarker> {
        let Some(model) = self.keyframe_models.get(&curve) else {
            return None;
        };
        let color = self
            .channels
            .row_data(curve)
            .map(|c| c.color)
            .unwrap_or_else(|| Color::from_rgb_u8(200, 200, 200));
        let mut kfs: Vec<(usize, SlintKeyframe)> = (0..model.row_count())
            .filter_map(|i| model.row_data(i).map(|k| (i, k)))
            .collect();
        kfs.sort_by(|a, b| a.1.time.partial_cmp(&b.1.time).unwrap_or(std::cmp::Ordering::Equal));
        let Some(pos) = kfs.iter().position(|&(i, _)| i == key) else {
            return None;
        };
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
            Some(mk_marker(curve, key, color, &kfs[pos].1, eff.0, eff.1, is_in, true))
        } else {
            None
        }
    }

    /// 控制柄松开：清空预览。
    pub fn handle_released(&mut self, _curve: usize, _key: usize, _is_in: bool) {
        self.preview_handles.set_vec(Vec::new());
    }

    // ── 通道开关 ─────────────────────────────────────────────

    pub fn toggle_channel_visible(&mut self, curve: usize) {
        if let Some(mut ch) = self.channels.row_data(curve) {
            ch.visible = !ch.visible;
            self.channels.set_row_data(curve, ch);
        }
    }

    pub fn toggle_channel_modifier(&mut self, curve: usize) {
        if let Some(mut ch) = self.channels.row_data(curve) {
            ch.modifier_enabled = !ch.modifier_enabled;
            self.channels.set_row_data(curve, ch);
        }
    }

    pub fn toggle_channel_active(&mut self, curve: usize) {
        if let Some(mut ch) = self.channels.row_data(curve) {
            ch.active = !ch.active;
            self.channels.set_row_data(curve, ch);
        }
    }

    pub fn toggle_channel_editable(&mut self, curve: usize) {
        if let Some(mut ch) = self.channels.row_data(curve) {
            ch.editable = !ch.editable;
            self.channels.set_row_data(curve, ch);
        }
    }

    // ── 标尺刻度（派生） ──────────────────────────────────────

    /// 主刻度最小像素间距：主网格线至少要隔这么远（Blender 的 v2d_min_gridsize 类似物）。
    const MIN_MAJOR_PX: f32 = 48.0;
    /// 次刻度最小像素间距：小于该值就不再生成次刻度（网格过密无意义）。
    const MIN_MINOR_PX: f32 = 8.0;
    /// 主刻度数量上限（安全网）：正常按像素间距生成，这里防止极端情况下数量失控。
    const MAX_MAJOR_TICKS: f32 = 120.0;
    /// 主刻度细分份数（Blender view2d_draw_lines 的 get_divisor 通常取 5）。
    const MINOR_DIVISOR: f32 = 5.0;

    /// 按当前缩放选主刻度步长：1/2/5×10^k，保证主网格线在屏幕上 ≥ MIN_MAJOR_PX 像素。
    /// 可见范围由像素决定，因此刻度数量与缩放无关、恒有界（≈ 屏宽/48 个主刻度）。
    fn tick_major_step(zoom: f32, range: f32) -> f32 {
        let z = zoom.max(0.01);
        let raw = (Self::MIN_MAJOR_PX / z).max(range / Self::MAX_MAJOR_TICKS);
        nice_step(raw).max(1e-6)
    }

    /// 生成一组刻度：主刻度（含标签）+ 细分次刻度（无标签，过密时省略）。
    fn tick_levels(lo: f32, hi: f32, major: f32, zoom: f32) -> Vec<SlintRulerTick> {
        let minor = major / Self::MINOR_DIVISOR;
        let show_minor = minor * zoom >= Self::MIN_MINOR_PX;
        let mut ticks = Vec::new();
        let mut v = (lo / major).floor() * major;
        while v <= hi {
            ticks.push(SlintRulerTick {
                value: v,
                label: format_float(v).into(),
                major: true,
            });
            v += major;
        }
        if show_minor {
            let mut m = (lo / minor).floor() * minor;
            while m <= hi {
                // 跳过与主刻度重合的点（浮点容差）
                let d = (m / major).round();
                if (m - d * major).abs() > major * 1e-3 {
                    ticks.push(SlintRulerTick {
                        value: m,
                        label: SharedString::default(),
                        major: false,
                    });
                }
                m += minor;
            }
        }
        ticks.sort_by(|a, b| a.value.partial_cmp(&b.value).unwrap_or(std::cmp::Ordering::Equal));
        ticks
    }

    /// 屏幕可见区间的世界范围 [(-pan)/zoom, (size - pan)/zoom]；无效时回退到数据范围。
    fn visible_world_range(
        &self,
        pan: f32,
        zoom: f32,
        size: f32,
        data_lo: f32,
        data_hi: f32,
    ) -> (f32, f32) {
        let z = zoom.max(0.01);
        let a = -pan / z;
        let b = (size - pan) / z;
        if size > 0.0 && b.is_finite() && (b - a).abs() > 1e-9 {
            if a <= b { (a, b) } else { (b, a) }
        } else {
            (data_lo, data_hi)
        }
    }

    /// 时间刻度：覆盖整个可见时间范围（随 pan/zoom/画布宽度变化），主刻度步长随 zoom-x 选取。
    fn rebuild_time_ticks(&mut self) {
        let zoom = self.time_zoom;
        let (lo, hi) = self.visible_world_range(
            self.pan_x,
            zoom,
            self.view_width,
            0.0,
            16.0, // 兜底：无数据/无视图时的默认时间范围
        );
        let range = (hi - lo).max(0.1);
        let margin = range * 0.05;
        let major = Self::tick_major_step(zoom, range);
        self.time_ticks.set_vec(Self::tick_levels(lo - margin, hi + margin, major, zoom));
    }

    /// 值刻度：覆盖整个可见值范围（随 pan/zoom/画布高度变化），主刻度步长随 zoom-y 选取。
    fn rebuild_value_ticks(&mut self) {
        let zoom = self.value_zoom;
        let (lo, hi) = self.visible_world_range(
            self.pan_y,
            zoom,
            self.view_height,
            0.0,
            100.0, // 兜底：无数据/无视图时的默认值范围
        );
        let range = (hi - lo).max(0.1);
        let margin = range * 0.05;
        let major = Self::tick_major_step(zoom, range);
        self.value_ticks.set_vec(Self::tick_levels(lo - margin, hi + margin, major, zoom));
    }

    /// 视图变化（缩放/平移/画布尺寸）：按可见范围重建刻度，
    /// 使坐标轴随缩放细化、随平移跟随、始终铺满整个画布且数量有界（放大不卡顿）。
    pub fn set_view(
        &mut self,
        zoom_x: f32,
        zoom_y: f32,
        pan_x: f32,
        pan_y: f32,
        width: f32,
        height: f32,
    ) {
        if !zoom_x.is_finite() || zoom_x <= 0.0 {
            return;
        }
        self.time_zoom = zoom_x;
        self.value_zoom = zoom_y.max(0.01);
        self.pan_x = if pan_x.is_finite() { pan_x } else { 0.0 };
        self.pan_y = if pan_y.is_finite() { pan_y } else { 0.0 };
        if width.is_finite() && width > 0.0 {
            self.view_width = width;
        }
        if height.is_finite() && height > 0.0 {
            self.view_height = height;
        }
        self.rebuild_time_ticks();
        self.rebuild_value_ticks();
    }
}

// ── 派生：曲线段 / 控制柄 ────────────────────────────────────────────────

/// 由相邻两关键帧生成一段曲线（线性/保持退化为直线贝塞尔）。
/// 渲染期钳制控制点 x 到单调区间（x0 ≤ P1.x ≤ P2.x ≤ x1），
/// 保证画出来的曲线始终是时间的函数——无论手柄被拖到什么位置都不会出现
/// 回退/S 形/自交。手柄本身不受编辑期钳制（可自由拖拽调整角度）。
fn make_segment(curve: usize, color: Color, k0: &SlintKeyframe, k1: &SlintKeyframe) -> SlintCurveSegment {
    let (kind, mut cx1, mut cy1, mut cx2, mut cy2, x1, y1) = match k0.interpolation {
        SlintInterpolation::Bezier => {
            let c1 = (
                k0.time + k0.handle_out.time_offset,
                k0.value + k0.handle_out.value_offset,
            );
            let c2 = (
                k1.time + k1.handle_in.time_offset,
                k1.value + k1.handle_in.value_offset,
            );
            (SlintSegmentKind::Bezier, c1.0, c1.1, c2.0, c2.1, k1.time, k1.value)
        }
        SlintInterpolation::Linear => straight(
            SlintSegmentKind::Linear,
            k0.time,
            k0.value,
            k1.time,
            k1.value,
        ),
        SlintInterpolation::Hold => straight(SlintSegmentKind::Hold, k0.time, k0.value, k1.time, k0.value),
    };
    // Hold：瞬变的目标值 = 下一关键帧的值（竖直虚线从 y1 连到 step_value）
    let step_value = if k0.interpolation == SlintInterpolation::Hold {
        k1.value
    } else {
        0.0
    };
    // 渲染期软钳制：控制点 x 各自钳到关键帧区间 [t0, t1]。
    // 越过相邻关键帧的控制点会正好落在相邻关键帧上（不取中点，避免产生缝隙）。
    // 相邻边界是软钳制；手柄自身只受“自己关键帧”硬钳制约束。
    if k0.time <= k1.time {
        cx1 = cx1.clamp(k0.time, k1.time);
        cx2 = cx2.clamp(k0.time, k1.time);
    }
    SlintCurveSegment {
        curve_index: curve as i32,
        kind,
        color,
        x0: k0.time,
        y0: k0.value,
        cx1,
        cy1,
        cx2,
        cy2,
        x1,
        y1,
        step_value,
    }
}

/// 构建一个控制柄标记（关键帧位置 + 控制点；preview=true 表示“渲染期钳制后”的有效控制点）。
fn mk_marker(
    curve: usize,
    key: usize,
    color: Color,
    kf: &SlintKeyframe,
    handle_time: f32,
    handle_value: f32,
    is_in: bool,
    preview: bool,
) -> SlintHandleMarker {
    SlintHandleMarker {
        curve_index: curve as i32,
        keyframe_index: key as i32,
        is_in,
        color,
        key_time: kf.time,
        key_value: kf.value,
        handle_time,
        handle_value,
        preview,
    }
}

/// 硬钳制手柄到“自己关键帧”一侧：出柄控制点 x 不能早于自己关键帧（time_offset ≥ 0），
/// 入柄不能晚于自己关键帧（time_offset ≤ 0）。
/// 相邻关键帧边界是软钳制（渲染期交叉钳制处理，允许手柄越过）。
fn clamp_handle_own_side(kf: &mut SlintKeyframe, is_in: bool) {
    if is_in {
        kf.handle_in.time_offset = kf.handle_in.time_offset.min(0.0);
    } else {
        kf.handle_out.time_offset = kf.handle_out.time_offset.max(0.0);
    }
}

/// 直线段：控制点取直线上 1/3、2/3 位置。
fn straight(kind: SlintSegmentKind, x0: f32, y0: f32, x1: f32, y1: f32) -> (SlintSegmentKind, f32, f32, f32, f32, f32, f32) {
    (
        kind,
        x0 + (x1 - x0) / 3.0,
        y0 + (y1 - y0) / 3.0,
        x0 + 2.0 * (x1 - x0) / 3.0,
        y0 + 2.0 * (y1 - y0) / 3.0,
        x1,
        y1,
    )
}

/// 选取 1/2/5×10^k 的“漂亮”步长。
fn nice_step(raw: f32) -> f32 {
    if raw <= 0.0 || !raw.is_finite() {
        return 1.0;
    }
    let mag = 10f32.powf(raw.log10().floor());
    let norm = raw / mag;
    let step = if norm < 1.5 {
        1.0
    } else if norm < 3.5 {
        2.0
    } else if norm < 7.5 {
        5.0
    } else {
        10.0
    };
    step * mag
}

/// 数值格式化：去掉多余尾零。
fn format_float(v: f32) -> String {
    if v.abs() < 1e-4 {
        "0".to_string()
    } else {
        let s = format!("{:.2}", v);
        s.trim_end_matches('0').trim_end_matches('.').to_string()
    }
}

// ── AppStore 入口 ─────────────────────────────────────────────────────────

impl crate::store::AppStore {
    /// 初始化曲线编辑器与 NLA 编辑器：把 Rust 持有的模型绑定到 Slint 的 AnimationState 全局。
    pub fn initialize_animation_editors(&mut self, instance: &crate::bridge::types::MainWindow) {
        use slint::ComponentHandle;

        let state = instance.global::<crate::bridge::types::AnimationState>();
        state.set_channels(ModelRc::from(self.ui_models.curve_editor.channels.clone()));
        state.set_segments(ModelRc::from(self.ui_models.curve_editor.segments.clone()));
        state.set_handles(ModelRc::from(self.ui_models.curve_editor.handles.clone()));
        state.set_time_ticks(ModelRc::from(self.ui_models.curve_editor.time_ticks.clone()));
        state.set_value_ticks(ModelRc::from(self.ui_models.curve_editor.value_ticks.clone()));
        state.set_nla_tracks(ModelRc::from(self.ui_models.nla_editor.tracks.clone()));
        state.set_current_time(self.ui_models.curve_editor.current_time as f32);
        state.set_action_name(self.ui_models.curve_editor.action_name.clone());
        state.set_action_start(self.ui_models.curve_editor.action_start);
        state.set_action_end(self.ui_models.curve_editor.action_end);
        state.set_current_channel(self.ui_models.curve_editor.current_channel as i32);
        state.set_preview_handles(ModelRc::from(self.ui_models.curve_editor.preview_handles.clone()));
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::bridge::types::SlintBezierHandle;
    use slint::Model;

    fn key(time: f32, value: f32, interpolation: SlintInterpolation) -> SlintKeyframe {
        SlintKeyframe {
            time,
            value,
            interpolation,
            handle_in: SlintBezierHandle {
                time_offset: 0.0,
                value_offset: 0.0,
            },
            handle_out: SlintBezierHandle {
                time_offset: 0.0,
                value_offset: 0.0,
            },
            selected: false,
        }
    }

    #[test]
    fn segments_derived_for_each_interpolation() {
        let mut st = CurveEditorState::default();
        let mut k0 = key(0.0, 0.0, SlintInterpolation::Bezier);
        k0.handle_out = SlintBezierHandle {
            time_offset: 1.0,
            value_offset: 5.0,
        };
        st.push_channel(
            "t",
            Color::from_rgb_u8(255, 0, 0),
            vec![
                k0,
                key(2.0, 10.0, SlintInterpolation::Linear),
                key(4.0, 10.0, SlintInterpolation::Hold),
            ],
        );
        st.rebuild_all();

        // 3 个关键帧 → 2 段
        assert_eq!(st.segments.row_count(), 2);

        let seg0 = st.segments.row_data(0).unwrap();
        // 第一段为 bezier（首关键帧插值），控制点 = 关键帧 + 出柄偏移
        assert_eq!(seg0.kind, SlintSegmentKind::Bezier);
        assert_eq!(seg0.cx1, 1.0);
        assert_eq!(seg0.cy1, 5.0);
        assert_eq!(seg0.x0, 0.0);
        assert_eq!(seg0.x1, 2.0);
        assert_eq!(seg0.curve_index, 0);

        let seg1 = st.segments.row_data(1).unwrap();
        // 第二段 linear：控制点位于直线上
        assert_eq!(seg1.kind, SlintSegmentKind::Linear);
        assert_eq!(seg1.cx1, 2.0 + 2.0 / 3.0);
        assert_eq!(seg1.cy1, 10.0);
        assert_eq!(seg1.x1, 4.0);
        assert_eq!(seg1.y1, 10.0);
    }

    #[test]
    fn keyframe_drag_updates_model_and_segments() {
        let mut st = CurveEditorState::default();
        st.push_channel(
            "t",
            Color::from_rgb_u8(255, 0, 0),
            vec![
                key(0.0, 0.0, SlintInterpolation::Linear),
                key(2.0, 10.0, SlintInterpolation::Linear),
                key(4.0, 20.0, SlintInterpolation::Linear),
            ],
        );
        st.rebuild_all();

        st.keyframe_pressed(0, 1, false);
        st.keyframe_dragged(0, 1, 1.0, -5.0);

        let kf = st.keyframe_models[&0].row_data(1).unwrap();
        assert_eq!(kf.time, 3.0);
        assert_eq!(kf.value, 5.0);
        assert!(kf.selected);

        // 段同步更新：段 0 终点 = 新位置
        let seg0 = st.segments.row_data(0).unwrap();
        assert_eq!(seg0.x1, 3.0);
        assert_eq!(seg0.y1, 5.0);

        // 多次拖拽累加（与节点拖拽一致）
        st.keyframe_dragged(0, 1, 0.5, 2.0);
        let kf = st.keyframe_models[&0].row_data(1).unwrap();
        assert_eq!(kf.time, 3.5);
        assert_eq!(kf.value, 7.0);
        st.keyframe_released(0, 1);
    }

    #[test]
    fn action_range_derived_from_keyframe_times() {
        let mut st = CurveEditorState::default();
        st.push_channel(
            "t",
            Color::from_rgb_u8(255, 0, 0),
            vec![
                key(2.0, 0.0, SlintInterpolation::Linear),
                key(5.0, 10.0, SlintInterpolation::Linear),
                key(9.0, 20.0, SlintInterpolation::Linear),
            ],
        );
        assert_eq!((st.action_start, st.action_end), (0.0, 0.0));
        st.rebuild_all();
        assert_eq!((st.action_start, st.action_end), (2.0, 9.0));
    }

    #[test]
    fn handle_drag_updates_bezier_offset() {
        let mut st = CurveEditorState::default();
        st.push_channel(
            "t",
            Color::from_rgb_u8(255, 0, 0),
            vec![
                key(0.0, 0.0, SlintInterpolation::Bezier),
                key(2.0, 10.0, SlintInterpolation::Bezier),
                key(4.0, 20.0, SlintInterpolation::Bezier),
            ],
        );
        st.rebuild_all();
        // 3 个 bezier 关键帧：首只画出柄、末只画入柄、中间两个 → 共 4 条控制柄
        assert_eq!(st.handles.row_count(), 4);

        st.handle_pressed(0, 0, false);
        st.handle_dragged(0, 0, false, 0.5, 3.0, 0);
        st.handle_dragged(0, 0, false, 0.2, 1.0, 0);

        let kf = st.keyframe_models[&0].row_data(0).unwrap();
        assert_eq!(kf.handle_out.time_offset, 0.7);
        assert_eq!(kf.handle_out.value_offset, 4.0);

        st.handle_released(0, 0, false);
    }

    #[test]
    fn handle_drag_modes_constrain_opposite() {
        fn setup() -> CurveEditorState {
            let mut st = CurveEditorState::default();
            st.push_channel(
                "t",
                Color::from_rgb_u8(255, 0, 0),
                vec![
                    key(0.0, 0.0, SlintInterpolation::Bezier),
                    key(2.0, 10.0, SlintInterpolation::Bezier),
                ],
            );
            st.rebuild_all();
            st
        }

        // 模式 0 自由：拖出柄，入柄不动
        let mut st = setup();
        st.handle_pressed(0, 0, false);
        st.handle_dragged(0, 0, false, 2.0, 3.0, 0);
        let kf = st.keyframe_models[&0].row_data(0).unwrap();
        assert_eq!(kf.handle_out.time_offset, 2.0);
        assert_eq!(kf.handle_out.value_offset, 3.0);
        assert_eq!(kf.handle_in.time_offset, 0.0);
        assert_eq!(kf.handle_in.value_offset, 0.0);
        st.handle_released(0, 0, false);

        // 模式 1 对齐：被拖柄自由移动；对手柄旋转共线、长度保持自身
        let mut st = setup();
        st.keyframe_models[&0].set_row_data(
            0,
            SlintKeyframe {
                time: 0.0,
                value: 0.0,
                interpolation: SlintInterpolation::Bezier,
                handle_in: SlintBezierHandle { time_offset: 0.0, value_offset: 4.0 },
                handle_out: SlintBezierHandle { time_offset: 1.0, value_offset: 0.0 },
                selected: false,
            },
        );
        st.handle_pressed(0, 0, false);
        // 拖出柄 (1,0) → 累加 (1,1) 得 (2,1)；入柄旋转到共线方向、长度保持 4
        st.handle_dragged(0, 0, false, 1.0, 1.0, 1);
        let kf = st.keyframe_models[&0].row_data(0).unwrap();
        assert_eq!(kf.handle_out.time_offset, 2.0);
        assert_eq!(kf.handle_out.value_offset, 1.0);
        let nl = (2f32 * 2f32 + 1f32).sqrt();
        assert!((kf.handle_in.time_offset - (-2.0 / nl * 4.0)).abs() < 1e-4);
        assert!((kf.handle_in.value_offset - (-1.0 / nl * 4.0)).abs() < 1e-4);
        st.handle_released(0, 0, false);

        // 模式 2 原点对称：入柄 = -出柄（角度+长度都镜像）
        let mut st = setup();
        st.handle_pressed(0, 0, false);
        st.handle_dragged(0, 0, false, 2.0, 3.0, 2);
        let kf = st.keyframe_models[&0].row_data(0).unwrap();
        assert_eq!(kf.handle_in.time_offset, -2.0);
        assert_eq!(kf.handle_in.value_offset, -3.0);
        st.handle_released(0, 0, false);
    }

    #[test]
    fn endpoint_handles_only_outward_side() {
        let mut st = CurveEditorState::default();
        st.push_channel(
            "t",
            Color::from_rgb_u8(255, 0, 0),
            vec![
                key(0.0, 0.0, SlintInterpolation::Bezier),
                key(2.0, 10.0, SlintInterpolation::Bezier),
                key(4.0, 20.0, SlintInterpolation::Bezier),
            ],
        );
        st.rebuild_all();

        let h0 = st.handles.row_data(0).unwrap();
        let h1 = st.handles.row_data(1).unwrap();
        let h2 = st.handles.row_data(2).unwrap();
        let h3 = st.handles.row_data(3).unwrap();
        // 首关键帧只有出柄
        assert_eq!((h0.keyframe_index, h0.is_in), (0, false));
        // 中间关键帧 入 + 出
        assert_eq!((h1.keyframe_index, h1.is_in), (1, true));
        assert_eq!((h2.keyframe_index, h2.is_in), (1, false));
        // 末关键帧只有入柄
        assert_eq!((h3.keyframe_index, h3.is_in), (2, true));
    }

    #[test]
    fn segment_render_soft_clamps_to_keyframe_interval() {
        let mut st = CurveEditorState::default();
        // k0 出柄 +5（P1.x=5），k1 入柄 -3（P2.x=1）→ 各自钳到 [0,4]，不取中点
        let mut k0 = key(0.0, 0.0, SlintInterpolation::Bezier);
        k0.handle_out = SlintBezierHandle { time_offset: 5.0, value_offset: 0.0 };
        let mut k1 = key(4.0, 10.0, SlintInterpolation::Bezier);
        k1.handle_in = SlintBezierHandle { time_offset: -3.0, value_offset: 0.0 };
        st.push_channel("t", Color::from_rgb_u8(255, 0, 0), vec![k0, k1]);
        st.rebuild_all();

        let seg = st.segments.row_data(0).unwrap();
        // 越过相邻关键帧的钳到相邻关键帧时间（无缝隙）
        assert_eq!(seg.cx1, 4.0);
        assert_eq!(seg.cx2, 1.0);
        assert!(seg.cx1 >= 0.0 && seg.cx2 <= 4.0);
        // 值保持不变
        assert_eq!(seg.cy1, 0.0);
        assert_eq!(seg.cy2, 10.0);
    }

    #[test]
    fn handle_dragged_own_keyframe_hard_clamped() {
        // 手柄拖拽时只有“自己关键帧”是硬钳制；相邻关键帧边界是软钳制（渲染期处理）
        let mut st = CurveEditorState::default();
        st.push_channel(
            "t",
            Color::from_rgb_u8(255, 0, 0),
            vec![
                key(0.0, 0.0, SlintInterpolation::Bezier),
                key(4.0, 10.0, SlintInterpolation::Bezier),
            ],
        );
        st.rebuild_all();

        // 出柄拖 -10（越过自己关键帧之前）→ 硬钳到 0
        st.handle_pressed(0, 0, false);
        st.handle_dragged(0, 0, false, -10.0, 0.0, 0);
        let kf = st.keyframe_models[&0].row_data(0).unwrap();
        assert_eq!(kf.handle_out.time_offset, 0.0);
        st.handle_released(0, 0, false);

        // 入柄拖 +10（越过自己关键帧之后）→ 硬钳到 0
        st.handle_pressed(0, 1, true);
        st.handle_dragged(0, 1, true, 10.0, 0.0, 0);
        let kf = st.keyframe_models[&0].row_data(1).unwrap();
        assert_eq!(kf.handle_in.time_offset, 0.0);
        st.handle_released(0, 1, true);

        // 出柄拖 +10（越过相邻关键帧）→ 软钳制，手柄本身保持 10（曲线由渲染期钳制）
        st.handle_pressed(0, 0, false);
        st.handle_dragged(0, 0, false, 10.0, 0.0, 0);
        let kf = st.keyframe_models[&0].row_data(0).unwrap();
        assert_eq!(kf.handle_out.time_offset, 10.0);
        st.handle_released(0, 0, false);
    }

    #[test]
    fn keyframe_drag_past_neighbor_resorts_curve() {
        let mut st = CurveEditorState::default();
        st.push_channel(
            "t",
            Color::from_rgb_u8(255, 0, 0),
            vec![
                key(0.0, 0.0, SlintInterpolation::Linear),
                key(2.0, 10.0, SlintInterpolation::Linear),
                key(4.0, 20.0, SlintInterpolation::Linear),
            ],
        );
        st.rebuild_all();

        // 把中间关键帧（2,10）拖到时间 5，越过最后关键帧（4,20）
        st.keyframe_pressed(0, 1, false);
        st.keyframe_dragged(0, 1, 3.0, 0.0);

        // 模型顺序不变（索引 1 仍是它），时间变成 5
        let kf = st.keyframe_models[&0].row_data(1).unwrap();
        assert_eq!(kf.time, 5.0);

        // 曲线段按时间排序重绘：(0,0)-(4,20) 然后 (4,20)-(5,10)
        let seg0 = st.segments.row_data(0).unwrap();
        assert_eq!((seg0.x0, seg0.y0, seg0.x1, seg0.y1), (0.0, 0.0, 4.0, 20.0));
        let seg1 = st.segments.row_data(1).unwrap();
        assert_eq!((seg1.x0, seg1.y0, seg1.x1, seg1.y1), (4.0, 20.0, 5.0, 10.0));
        st.keyframe_released(0, 1);
    }

    #[test]
    fn handles_respect_neighbor_interpolation() {
        // A(0) 贝塞尔、B(2) 线性、C(4) 贝塞尔
        let mut st = CurveEditorState::default();
        st.push_channel(
            "t",
            Color::from_rgb_u8(255, 0, 0),
            vec![
                key(0.0, 0.0, SlintInterpolation::Bezier),
                key(2.0, 10.0, SlintInterpolation::Linear),
                key(4.0, 20.0, SlintInterpolation::Bezier),
            ],
        );
        st.rebuild_all();

        // 段 [A,B]：A 贝塞尔 → 用 A.handle_out + B.handle_in；[B,C]：B 线性 → 无贝塞尔柄
        // 所以只有 A.out 与 B.in 两个手柄
        assert_eq!(st.handles.row_count(), 2);
        let h0 = st.handles.row_data(0).unwrap();
        assert_eq!((h0.keyframe_index, h0.is_in), (0, false)); // A.out
        let h1 = st.handles.row_data(1).unwrap();
        assert_eq!((h1.keyframe_index, h1.is_in), (1, true)); // B.in
    }

    #[test]
    fn pressing_controls_activates_channel() {
        let mut st = CurveEditorState::default();
        st.push_channel(
            "a",
            Color::from_rgb_u8(255, 0, 0),
            vec![key(0.0, 0.0, SlintInterpolation::Linear)],
        );
        st.push_channel(
            "b",
            Color::from_rgb_u8(0, 255, 0),
            vec![key(0.0, 0.0, SlintInterpolation::Linear)],
        );
        assert_eq!(st.current_channel, -1);
        st.channel_pressed(1, 0);
        assert_eq!(st.current_channel, 1);
        st.keyframe_pressed(0, 0, false);
        assert_eq!(st.current_channel, 0);
        st.deselect_channel();
        assert_eq!(st.current_channel, -1);
    }

    #[test]
    fn channel_plain_click_selects_only_one() {
        let mut st = CurveEditorState::default();
        for name in ["a", "b", "c", "d"] {
            st.push_channel(
                name,
                Color::from_rgb_u8(255, 0, 0),
                vec![key(0.0, 0.0, SlintInterpolation::Linear)],
            );
        }
        st.channel_pressed(2, 0);
        assert!(st.channels.row_data(2).unwrap().selected);
        for i in [0, 1, 3] {
            assert!(!st.channels.row_data(i).unwrap().selected, "channel {i} 不应被选中");
        }
        assert_eq!(st.selected_channel_count(), 1);
        // 再点另一个：普通点击只选它
        st.channel_pressed(0, 0);
        assert!(st.channels.row_data(0).unwrap().selected);
        assert!(!st.channels.row_data(2).unwrap().selected);
        assert_eq!(st.current_channel, 0);
    }

    #[test]
    fn channel_shift_selects_range_from_anchor() {
        let mut st = CurveEditorState::default();
        for name in ["a", "b", "c", "d", "e"] {
            st.push_channel(
                name,
                Color::from_rgb_u8(255, 0, 0),
                vec![key(0.0, 0.0, SlintInterpolation::Linear)],
            );
        }
        // 锚点 = 最近普通点击的通道（1）
        st.channel_pressed(1, 0);
        // Shift 点 4 → 选中 1..=4
        st.channel_pressed(4, 1);
        for i in 1..=4 {
            assert!(st.channels.row_data(i).unwrap().selected, "channel {i} 应在区间内");
        }
        assert!(!st.channels.row_data(0).unwrap().selected);
        assert_eq!(st.selected_channel_count(), 4);
        assert_eq!(st.current_channel, 4);
        // 再 Shift 点 0（反向区间 0..=4 仍全选，锚点更新为 0）
        st.channel_pressed(0, 1);
        for i in 0..=4 {
            assert!(st.channels.row_data(i).unwrap().selected, "channel {i} 应在区间内");
        }
    }

    #[test]
    fn channel_ctrl_toggles_selection() {
        let mut st = CurveEditorState::default();
        for name in ["a", "b", "c"] {
            st.push_channel(
                name,
                Color::from_rgb_u8(255, 0, 0),
                vec![key(0.0, 0.0, SlintInterpolation::Linear)],
            );
        }
        st.channel_pressed(0, 0);
        // Ctrl 逐个追加
        st.channel_pressed(1, 2);
        st.channel_pressed(2, 2);
        for i in 0..=2 {
            assert!(st.channels.row_data(i).unwrap().selected);
        }
        // Ctrl 再点已选中的 → 取消
        st.channel_pressed(1, 2);
        assert!(!st.channels.row_data(1).unwrap().selected);
        assert!(st.channels.row_data(0).unwrap().selected);
        assert!(st.channels.row_data(2).unwrap().selected);
        // Ctrl 取消后 current_channel 指向最后仍选中的（这里保持最后一次有效选中）
        assert_eq!(st.current_channel, 2);
        assert_eq!(st.selected_channel_count(), 2);
    }

    #[test]
    fn deselect_channel_clears_all_selection() {
        let mut st = CurveEditorState::default();
        for name in ["a", "b", "c"] {
            st.push_channel(
                name,
                Color::from_rgb_u8(255, 0, 0),
                vec![key(0.0, 0.0, SlintInterpolation::Linear)],
            );
        }
        st.channel_pressed(0, 0);
        st.channel_pressed(1, 2);
        st.deselect_channel();
        assert_eq!(st.selected_channel_count(), 0);
        assert_eq!(st.current_channel, -1);
    }

    #[test]
    fn curve_color_mode_switches_auto_rainbow_and_user() {
        let mut st = CurveEditorState::default();
        let user = Color::from_rgb_u8(12, 34, 56);
        st.push_channel(
            "t",
            user,
            vec![
                key(0.0, 0.0, SlintInterpolation::Linear),
                key(4.0, 10.0, SlintInterpolation::Linear),
            ],
        );
        st.rebuild_all();

        // 默认自动彩虹：显示色 = 调色板按下标取，用户色被保存
        let ch = st.channels.row_data(0).unwrap();
        assert_eq!(ch.color_mode, 0);
        assert_eq!(ch.user_color, user);
        assert_ne!(ch.color, user);
        assert_eq!(ch.color, CurveEditorState::rainbow_color(0));

        // 切到用户设置：显示色恢复为用户色
        st.set_curve_color_mode(0, 1);
        let ch = st.channels.row_data(0).unwrap();
        assert_eq!(ch.color_mode, 1);
        assert_eq!(ch.color, user);
        // 段颜色同步更新
        let seg = st.segments.row_data(0).unwrap();
        assert_eq!(seg.color, user);

        // 切回自动彩虹：回到调色板色
        st.set_curve_color_mode(0, 0);
        let ch = st.channels.row_data(0).unwrap();
        assert_eq!(ch.color_mode, 0);
        assert_eq!(ch.color, CurveEditorState::rainbow_color(0));
    }

    #[test]
    fn selected_keyframes_interpolation_change() {
        let mut st = CurveEditorState::default();
        st.push_channel(
            "t",
            Color::from_rgb_u8(255, 0, 0),
            vec![
                key(0.0, 0.0, SlintInterpolation::Linear),
                key(4.0, 10.0, SlintInterpolation::Linear),
            ],
        );
        st.push_channel(
            "u",
            Color::from_rgb_u8(0, 255, 0),
            vec![key(2.0, 5.0, SlintInterpolation::Hold)],
        );
        st.rebuild_all();

        assert_eq!(st.selected_keyframe_count(), 0);
        assert_eq!(st.selected_interpolation_index(), -1);

        // 无选中时修改无效
        st.set_selected_keyframes_interpolation(0);
        assert_eq!(st.selected_keyframe_count(), 0);

        // 选中通道 0 的两个关键帧
        st.keyframe_pressed(0, 0, false);
        st.keyframe_pressed(0, 1, true);
        assert_eq!(st.selected_keyframe_count(), 2);
        assert_eq!(st.selected_interpolation_index(), 1); // linear

        // 改成贝塞尔：全部选中关键帧生效，未选中的不受影响
        st.set_selected_keyframes_interpolation(0);
        let kf0 = st.keyframe_models[&0].row_data(0).unwrap();
        let kf1 = st.keyframe_models[&0].row_data(1).unwrap();
        let kf2 = st.keyframe_models[&1].row_data(0).unwrap();
        assert_eq!(kf0.interpolation, SlintInterpolation::Bezier);
        assert_eq!(kf1.interpolation, SlintInterpolation::Bezier);
        assert_eq!(kf2.interpolation, SlintInterpolation::Hold);
        assert_eq!(st.selected_interpolation_index(), 0); // bezier

        // 改成常量（hold）
        st.set_selected_keyframes_interpolation(2);
        assert_eq!(
            st.keyframe_models[&0].row_data(0).unwrap().interpolation,
            SlintInterpolation::Hold
        );
        assert_eq!(st.selected_interpolation_index(), 2);
    }

    #[test]
    fn linear_to_bezier_creates_out_handle_with_default_offset() {
        let mut st = CurveEditorState::default();
        st.push_channel(
            "t",
            Color::from_rgb_u8(255, 0, 0),
            vec![
                key(0.0, 0.0, SlintInterpolation::Linear),
                key(4.0, 10.0, SlintInterpolation::Linear),
                key(8.0, 20.0, SlintInterpolation::Linear),
            ],
        );
        st.rebuild_all();
        assert_eq!(st.handles.row_count(), 0); // 全线性 → 无控制柄

        // 选中中间关键帧 → 切贝塞尔
        st.keyframe_pressed(0, 1, false);
        st.set_selected_keyframes_interpolation(0);

        // 中间关键帧出柄（右段变贝塞尔）+ 末关键帧入柄（其左段变贝塞尔）；首关键帧（仍线性）无柄
        assert_eq!(st.handles.row_count(), 2);
        let h_out = st.handles.row_data(0).unwrap();
        assert!(!h_out.is_in);
        assert_eq!(h_out.keyframe_index, 1);
        // 默认外伸 1/3 间距（4→8 为 4，外伸 4/3 ≈ 1.333）
        assert!((h_out.handle_time - (4.0 + 4.0 / 3.0)).abs() < 0.001);
        assert!((h_out.handle_value - 10.0).abs() < 0.001);

        let h_in = st.handles.row_data(1).unwrap();
        assert!(h_in.is_in);
        assert_eq!(h_in.keyframe_index, 2);

        // 出柄在模型中的值同步更新
        let kf = st.keyframe_models[&0].row_data(1).unwrap();
        assert_eq!(kf.interpolation, SlintInterpolation::Bezier);
        assert!((kf.handle_out.time_offset - 4.0 / 3.0).abs() < 0.001);

        // 右侧段应为贝塞尔，且控制点不再与端点重合（曲线呈弧形）
        let seg = st.segments.row_data(1).unwrap();
        assert_eq!(seg.kind, SlintSegmentKind::Bezier);
        assert!(seg.cx1 > seg.x0);
        assert!(seg.cx1 < seg.x1);
    }

    #[test]
    fn view_ticks_cover_visible_range_and_stay_bounded() {
        let mut st = CurveEditorState::default();
        // 固定 1000x700 画布，遍历从最小到最大的缩放：刻度应始终铺满可视范围且数量有界
        for zoom in [0.1f32, 0.5, 1.0, 30.0, 100.0, 200.0] {
            let zoom_y = (zoom * 0.067).max(0.01);
            st.set_view(zoom, zoom * 0.067, 0.0, 0.0, 1000.0, 700.0);

            let t = st.time_ticks.row_count();
            let v = st.value_ticks.row_count();
            assert!(t <= 400, "zoom={zoom} 时间刻度数量 {t} 超过上限");
            assert!(v <= 400, "zoom={zoom} 值刻度数量 {v} 超过上限");
            assert!(t >= 8, "zoom={zoom} 时间刻度数量 {t} 过少");
            assert!(v >= 8, "zoom={zoom} 值刻度数量 {v} 过少");

            // 可见世界范围：时间 [0, 1000/zoom]，值 [0, 700/zoom_y]
            let time_max = 1000.0 / zoom;
            let value_max = 700.0 / zoom_y;
            let t_max = st
                .time_ticks
                .row_data(st.time_ticks.row_count() - 1)
                .unwrap()
                .value;
            let v_max = st
                .value_ticks
                .row_data(st.value_ticks.row_count() - 1)
                .unwrap()
                .value;
            assert!(
                t_max >= time_max - 0.5,
                "zoom={zoom} 时间刻度未覆盖可视范围右缘 {t_max} < {time_max}"
            );
            assert!(
                v_max >= value_max * 0.9,
                "zoom={zoom} 值刻度未覆盖可视范围 {v_max} < {value_max}"
            );
        }
    }

    #[test]
    fn view_pan_shifts_ticks_to_follow() {
        let mut st = CurveEditorState::default();
        // 平移到时间 50s 处：刻度应跟随到 [50, ...]，而不是仍停在数据范围
        st.set_view(30.0, 2.0, 0.0, 0.0, 1000.0, 700.0);
        let before_min = st.time_ticks.row_data(0).unwrap().value;
        st.set_view(30.0, 2.0, -1000.0, 0.0, 1000.0, 700.0);
        let after_min = st.time_ticks.row_data(0).unwrap().value;
        assert!(after_min > before_min, "平移后刻度应右移");
        assert!((after_min - 33.0).abs() < 40.0);
    }
}
