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

use std::collections::{HashMap, HashSet};
use std::rc::Rc;

use slint::{Color, Model, ModelExt, ModelRc, SharedString, VecModel};

use crate::bridge::types::{
    ContextMenuItem, SlintBezierHandle, SlintCurveSegment, SlintDopeConnection, SlintFCurveChannel,
    SlintHandleMarker, SlintInterpolation, SlintKeyframe, SlintSegmentKind, SlintSummaryKeyframe,
};

/// 由 Action 名称列表派生下拉菜单项（无前导色点/无分隔线）。
fn build_action_menu_items(names: &[SharedString]) -> Vec<ContextMenuItem> {
    names
        .iter()
        .map(|name| ContextMenuItem {
            text: name.clone(),
            enabled: true,
            dot_color: Color::from_argb_u8(0, 0, 0, 0),
            separator_before: false,
        })
        .collect()
}

/// 时间坐标最小单位为毫秒：把时间对齐到最近的整数毫秒（关键帧位置要求）。
fn snap_ms(t: f32) -> f32 {
    (t * 1000.0).round() / 1000.0
}

/// 时间 → 整数毫秒（用于“同一时间”判断：整数毫秒相等即同一时刻）。
fn ms_of(t: f32) -> i64 {
    (t * 1000.0).round() as i64
}

/// 一个 Action 的通道定义快照（UI 先行阶段的数据源：切换 Action 时据此重建编辑器）。
/// 后续接线 aweft-model 的 AnimationLibrary 后，改由 Action 的 curves 直接生成。
#[derive(Clone, Debug)]
pub struct ActionData {
    /// Action 名
    pub name: SharedString,
    /// (通道名, 用户色, 关键帧列表)
    pub channels: Vec<(SharedString, Color, Vec<SlintKeyframe>)>,
}

/// 曲线编辑器状态
pub struct CurveEditorState {
    /// 通道视图模型（绑定 Slint；`keyframes` 指向 `keyframe_models` 中同一 inner 模型）
    pub channels: Rc<VecModel<SlintFCurveChannel>>,
    /// 曲线编辑器通道列表的过滤压缩视图（FilterModel，只含匹配搜索的通道；元素带真实 curve-index）
    pub graph_filter: ModelRc<SlintFCurveChannel>,
    /// 摄影表通道列表/画布的过滤压缩视图（同上）
    pub dope_filter: ModelRc<SlintFCurveChannel>,
    /// 曲线编辑器画布：段/手柄的过滤视图（FilterModel，只含匹配搜索通道的段/手柄）
    pub graph_segments: ModelRc<SlintCurveSegment>,
    pub graph_handles: ModelRc<SlintHandleMarker>,
    /// 派生曲线段（Path 逐段渲染）
    pub segments: Rc<VecModel<SlintCurveSegment>>,
    /// 派生贝塞尔控制柄
    pub handles: Rc<VecModel<SlintHandleMarker>>,
    /// 摄影表连接线（线性/常量插值段的直线连接，扁平模型）
    pub dope_connections: Rc<VecModel<SlintDopeConnection>>,
    /// 摄影表汇总条带的关键帧（按整数毫秒时间聚合所有通道同一时间的关键帧）
    pub summary_keyframes: Rc<VecModel<SlintSummaryKeyframe>>,
    /// 当前时间（播放头）
    pub current_time: f64,
    /// 当前 Action 名称（header 栏中区显示；UI 先行阶段为占位，后续接 model 的 Action 名）
    pub action_name: SharedString,
    /// Action 列表（Header 中区下拉菜单的数据源；UI 先行阶段为占位，后续接 model 的 AnimationLibrary.actions）
    pub action_names: Vec<SharedString>,
    /// 各 Action 的通道数据（下标与 action_names / current_action 对齐；切换 Action 时据此重建编辑器）
    pub actions: Vec<ActionData>,
    /// 当前选中的 Action 下标（-1 = 无）
    pub current_action: i32,
    /// Action 下拉菜单项模型（由 action_names 派生，色点透明）
    pub action_items: Rc<VecModel<ContextMenuItem>>,
    /// Action 时长范围（画布明暗分段用，由关键帧时间推导）
    pub action_start: f32,
    pub action_end: f32,
    /// 当前激活通道（点击曲线/关键帧/控制柄/通道列表行设置；-1 = 无激活；侧边栏据此显示哪条曲线）
    pub current_channel: isize,
    /// Shift 区间选择的锚点（最近一次普通/Shift 点击的通道；Ctrl 切换不更新）
    selection_anchor: isize,
    /// 关键帧按下时的"待应用通道选择"。
    /// 关键帧标记嵌套在 channels for 循环内，按下期间若更新外层 channels 模型会让 Slint
    /// 重刷 grab 路径上的行元素、丢失拖拽抓取（见 tests/dope_drag.rs 的复现）。
    /// 因此按下时只记录期望选择，松开（grab 已结束）后再写回 channels 模型。
    pending_channel_selection: Vec<usize>,
    /// 本次按下时，被按关键帧位置是否已存在选中（用于单击循环选中几乎重合的点）。
    pressed_was_selected: bool,
    /// 曲线编辑器 / 摄影表各自的搜索词（独立过滤）
    graph_search: String,
    dope_search: String,
    /// 曲线编辑器当前匹配搜索的通道集合（段/手柄过滤用）
    graph_matched: HashSet<usize>,
    /// 手柄拖拽时相邻关键帧软钳制的预览：被拖柄的有效控制点（与原始位置不同）
    pub preview_handles: Rc<VecModel<SlintHandleMarker>>,
    /// 选中的贝塞尔控制柄：(通道, 关键帧下标, is_in)。
    /// 与关键帧选中独立；框选可同时选中关键帧与手柄；多选拖拽时同侧手柄按相同角度增量旋转。
    pub selected_handles: HashSet<(usize, usize, bool)>,
    /// 当前视图缩放（px/单位）：命中检测容差随缩放变化（放大更精确）
    pub time_zoom: f32,
    pub value_zoom: f32,

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
            graph_filter: ModelRc::default(),
            dope_filter: ModelRc::default(),
            graph_segments: ModelRc::default(),
            graph_handles: ModelRc::default(),
            segments: Rc::new(VecModel::default()),
            handles: Rc::new(VecModel::default()),
            dope_connections: Rc::new(VecModel::default()),
            summary_keyframes: Rc::new(VecModel::default()),
            current_time: 0.0,
            action_name: "主 Action".into(),
            action_names: vec!["主 Action".into()],
            actions: vec![ActionData {
                name: "主 Action".into(),
                channels: Vec::new(),
            }],
            current_action: 0,
            action_items: {
                let m = Rc::new(VecModel::default());
                let names: Vec<SharedString> = vec!["主 Action".into()];
                m.set_vec(build_action_menu_items(&names));
                m
            },
            action_start: 0.0,
            action_end: 0.0,
            current_channel: -1,
            selection_anchor: -1,
            pending_channel_selection: Vec::new(),
            pressed_was_selected: false,
            graph_search: String::new(),
            dope_search: String::new(),
            graph_matched: HashSet::new(),
            preview_handles: Rc::new(VecModel::default()),
            selected_handles: HashSet::new(),
            // 与 Slint GraphEditor 的 nav-zoom-x/y 初始值一致（见 graph_editor.slint）
            time_zoom: 30.0,
            value_zoom: 2.0,
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
        // 时间坐标以毫秒为最小单位：关键帧必须落在整数毫秒上（UI 先行阶段在此归一化，
        // 后续接入 model 后由数据层保证）。
        let keyframes: Vec<SlintKeyframe> = keyframes
            .into_iter()
            .map(|k| {
                let mut k = k;
                k.time = snap_ms(k.time);
                k
            })
            .collect();
        let inner: Rc<VecModel<SlintKeyframe>> = Rc::new(VecModel::from(keyframes));
        self.keyframe_models.insert(idx, Rc::clone(&inner));
        self.channels.push(SlintFCurveChannel {
            curve_index: idx as i32,
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
        // 新增通道后刷新两个过滤视图
        self.rebuild_filters();
    }

    /// 重建曲线编辑器 / 摄影表各自的通道过滤视图，以及曲线编辑器画布的段/手柄过滤视图
    /// （都通过 ModelExt::filter 语法糖构造 FilterModel，自动跟随底层模型变化）。
    fn rebuild_filters(&mut self) {
        let g = self.graph_search.clone();
        self.graph_filter = ModelRc::from(Rc::new(self.channels.clone().filter(move |c: &SlintFCurveChannel| {
            let needle = g.trim().to_lowercase();
            needle.is_empty() || c.name.to_lowercase().contains(&needle)
        })));
        let d = self.dope_search.clone();
        self.dope_filter = ModelRc::from(Rc::new(self.channels.clone().filter(move |c: &SlintFCurveChannel| {
            let needle = d.trim().to_lowercase();
            needle.is_empty() || c.name.to_lowercase().contains(&needle)
        })));

        // 曲线编辑器画布：只显示匹配搜索通道的段/手柄（关键帧走 graph-channels）
        self.graph_matched = (0..self.graph_filter.row_count())
            .filter_map(|i| self.graph_filter.row_data(i).map(|c| c.curve_index as usize))
            .collect();
        let seg_matched = self.graph_matched.clone();
        self.graph_segments = ModelRc::from(Rc::new(self.segments.clone().filter(
            move |s: &SlintCurveSegment| seg_matched.contains(&(s.curve_index as usize)),
        )));
        let hdl_matched = self.graph_matched.clone();
        self.graph_handles = ModelRc::from(Rc::new(self.handles.clone().filter(
            move |h: &SlintHandleMarker| hdl_matched.contains(&(h.curve_index as usize)),
        )));
    }

    /// 曲线编辑器搜索：仅影响曲线编辑器自己的过滤视图（与摄影表互不干扰）。
    pub fn set_graph_search(&mut self, text: &str) {
        self.graph_search = text.to_string();
        self.rebuild_filters();
    }

    /// 摄影表搜索：仅影响摄影表自己的过滤视图。
    pub fn set_dope_search(&mut self, text: &str) {
        self.dope_search = text.to_string();
        self.rebuild_filters();
        self.rebuild_dope_connections();
        self.rebuild_summary();
    }

    /// 摄影表点击压缩视图中的一行：把显示行映射回真实通道下标再选中。
    pub fn dope_channel_pressed(&mut self, view_row: usize, mode: i32) {
        let real = self
            .dope_filter
            .row_data(view_row)
            .map(|c| c.curve_index as usize)
            .unwrap_or(view_row);
        self.channel_pressed(real, mode);
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
        self.rebuild_dope_connections();
        self.rebuild_summary();
        self.rebuild_action_range();
    }

    /// 重建摄影表汇总条带：把全部通道的关键帧按整数毫秒时间聚合，
    /// 同一毫秒的关键帧合并为一个汇总标记（count = 该时刻关键帧总数，
    /// selected = 逐通道选中掩码，按通道在过滤视图中的顺序）。
    /// 只统计摄影表当前可见的通道（搜索过滤 + 眼睛），与下方条带保持一致。
    /// 时间已按整数毫秒对齐，因此“同一时间”即 ms 相等。
    fn rebuild_summary(&mut self) {
        use std::collections::BTreeMap;
        let mut groups: BTreeMap<i64, Vec<bool>> = BTreeMap::new();
        for row in 0..self.dope_filter.row_count() {
            let Some(ch) = self.dope_filter.row_data(row) else { continue };
            if !ch.visible {
                continue;
            }
            let curve = ch.curve_index as usize;
            let Some(model) = self.keyframe_models.get(&curve) else { continue };
            for i in 0..model.row_count() {
                if let Some(kf) = model.row_data(i) {
                    groups.entry(ms_of(kf.time)).or_default().push(kf.selected);
                }
            }
        }
        let items: Vec<SlintSummaryKeyframe> = groups
            .into_iter()
            .map(|(ms, sel)| SlintSummaryKeyframe {
                time: ms as f32 / 1000.0,
                count: sel.len() as i32,
                all_selected: sel.iter().all(|s| *s),
                selected: ModelRc::from(Rc::new(slint::VecModel::from(sel))),
            })
            .collect();
        self.summary_keyframes.set_vec(items);
    }

    /// 重建摄影表连接线：遍历摄影表过滤视图（压缩，仅匹配搜索的通道），
    /// 每个通道内按时间排序后，相邻关键帧之间为线性/常量插值的加一条连接。
    /// row-index 为压缩视图行号（画布行位置用）；visible 反映通道眼睛。
    fn rebuild_dope_connections(&mut self) {
        let mut conns: Vec<SlintDopeConnection> = Vec::new();
        for row in 0..self.dope_filter.row_count() {
            let Some(ch) = self.dope_filter.row_data(row) else { continue };
            let c = ch.curve_index as usize;
            let Some(model) = self.keyframe_models.get(&c) else { continue };
            let mut kfs: Vec<SlintKeyframe> = (0..model.row_count())
                .filter_map(|i| model.row_data(i))
                .collect();
            kfs.sort_by(|a, b| a.time.partial_cmp(&b.time).unwrap_or(std::cmp::Ordering::Equal));
            for i in 0..kfs.len().saturating_sub(1) {
                let interp = kfs[i].interpolation;
                if interp == SlintInterpolation::Linear || interp == SlintInterpolation::Hold {
                    conns.push(SlintDopeConnection {
                        curve_index: c as i32,
                        row_index: row as i32,
                        x0: kfs[i].time,
                        x1: kfs[i + 1].time,
                        color: ch.color,
                        is_hold: interp == SlintInterpolation::Hold,
                        visible: ch.visible,
                    });
                }
            }
        }
        self.dope_connections.set_vec(conns);
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

    /// 由 `action_names` 重建下拉菜单项模型（Action 列表变化时调用）。
    pub fn rebuild_action_menu(&mut self) {
        self.action_items
            .set_vec(build_action_menu_items(&self.action_names));
    }

    /// 选择 Action：把该 Action 的通道数据装载进编辑器并重建派生数据。
    /// UI 先行阶段数据来自 actions 快照；后续接线 model 的 AnimationLibrary 后改为实时加载。
    pub fn select_action(&mut self, index: i32) {
        if index < 0 {
            self.current_action = -1;
            self.action_name = SharedString::default();
            return;
        }
        self.load_action(index as usize);
    }

    /// 装载第 `index` 个 Action：清空旧通道/关键帧，按该 Action 的通道定义重建全部派生数据。
    pub fn load_action(&mut self, index: usize) {
        let Some(data) = self.actions.get(index).cloned() else { return };
        self.channels.set_vec(Vec::new());
        self.keyframe_models.clear();
        for (name, color, kfs) in &data.channels {
            self.push_channel(name.clone(), *color, kfs.clone());
        }
        self.action_name = data.name.clone();
        self.current_action = index as i32;
        self.rebuild_all();
    }

    /// 下插到 NLA 后重置全局 Action：清空当前 Action 的通道/关键帧与名称，
    /// 开始一个全新的空 Action（无名称，UI 显示"<无动作>"；时长范围归零）。
    pub fn reset_action(&mut self) {
        self.channels.set_vec(Vec::new());
        self.keyframe_models.clear();
        self.preview_handles.set_vec(Vec::new());
        self.action_name = SharedString::default();
        self.current_action = -1;
        self.rebuild_all();
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
                    selected: self.selected_handles.contains(&(curve, *model_idx, true)),
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
                    selected: self.selected_handles.contains(&(curve, *model_idx, false)),
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
        // 摄影表连接线随关键帧时间/插值变化
        self.rebuild_dope_connections();
    }

    // ── 交互回调 ──────────────────────────────────────────────

    /// 通道是否可编辑（锁）。
    fn channel_editable(&self, curve: usize) -> bool {
        self.channels.row_data(curve).map(|c| c.editable).unwrap_or(false)
    }

    /// 关键帧按下：激活所在通道 + 选中（additive=Shift/Ctrl 追加）。锁定通道不可交互。
    ///
    /// 选中语义（对齐 Blender，避免拖拽反馈问题）：
    ///  - 普通点击**未选中**的关键帧：清空选择，单选它；
    ///  - 普通点击**已选中**的关键帧：保持现有多选（拖拽时所有选中一起移动）；
    ///  - Shift/Ctrl：把该关键帧**加入**选择（不切换取消——被拖的关键帧必须保持选中，
    ///    否则 marker 不跟随光标，拖拽增量按“marker 随动”计算会反馈放大、越拖越快）。
    ///
    /// 注意：通道（`channels` 外层模型）的选中状态在这里只记录到 `pending_channel_selection`，
    /// 等 `keyframe_released`（grab 结束后）才写回。原因是关键帧标记渲染在
    /// `for ch in channels: for kf in ch.keyframes` 的嵌套循环里，按下期间对外层
    /// channels 模型 `set_row_data` 会破坏 Slint 的拖拽抓取（见 tests/dope_drag.rs）。
    pub fn keyframe_pressed(&mut self, curve: usize, key: usize, additive: bool) {
        if !self.channel_editable(curve) {
            return;
        }
        self.preview_handles.set_vec(Vec::new());
        let already_selected = self
            .keyframe_models
            .get(&curve)
            .and_then(|m| m.row_data(key))
            .map(|kf| kf.selected)
            .unwrap_or(false);
        // 该位置是否已有选中点（含其它通道几乎重合的）：
        // 有 → 本次按下视为“点已选中的点”，松开（单击）时循环到下一个重合点
        self.pressed_was_selected = already_selected || self.has_selected_coincident(curve, key);

        if additive {
            // 加入该关键帧（已选中则不变），并确保其通道在期望选中集合里
            if !already_selected {
                self.set_keyframe_selected(curve, key, false);
            }
            let mut sel = self.effective_selected_channels();
            if !sel.contains(&curve) {
                sel.push(curve);
            }
            self.pending_channel_selection = sel;
        } else if self.pressed_was_selected {
            // 该位置已有选中点（含其它通道几乎重合的）：保持现状，
            // 拖拽时所有选中一起移动；松开（单击）时循环到下一个重合点
            self.pending_channel_selection = self.effective_selected_channels();
        } else {
            // 普通点击未选中的关键帧：清空并单选
            self.pending_channel_selection = vec![curve];
            self.selection_anchor = curve as isize;
            self.deselect_all();
            self.set_keyframe_selected(curve, key, false);
        }
        self.current_channel = curve as isize;
    }

    /// 被按关键帧附近（几乎重合的像素范围）是否已有选中的关键帧（含其它通道）。
    fn has_selected_coincident(&self, curve: usize, key: usize) -> bool {
        let Some(model) = self.keyframe_models.get(&curve) else {
            return false;
        };
        let Some(target) = model.row_data(key) else {
            return false;
        };
        let eps_t = 5.0 / self.time_zoom.max(0.01);
        let eps_v = 5.0 / self.value_zoom.max(0.01);
        for (&c, m) in self.keyframe_models.iter() {
            if !self.channels.row_data(c).map(|ch| ch.visible).unwrap_or(false) {
                continue;
            }
            for j in 0..m.row_count() {
                let Some(kf) = m.row_data(j) else { continue };
                if kf.selected
                    && (kf.time - target.time).abs() <= eps_t
                    && (kf.value - target.value).abs() <= eps_v
                {
                    return true;
                }
            }
        }
        false
    }

    /// 单击已选中的关键帧：循环选中几乎重合的若干点（Blender 行为）。
    /// 收集被按位置附近的全部关键帧，取“当前选中”的下一个；只有一个时保持自身。
    fn cycle_coincident(&mut self, curve: usize, key: usize) {
        let Some(model) = self.keyframe_models.get(&curve) else {
            return;
        };
        let Some(target) = model.row_data(key) else {
            return;
        };
        let eps_t = 5.0 / self.time_zoom.max(0.01);
        let eps_v = 5.0 / self.value_zoom.max(0.01);
        let mut list: Vec<(usize, usize)> = Vec::new();
        for c in 0..self.channels.row_count() {
            if !self.channels.row_data(c).map(|ch| ch.visible).unwrap_or(false) {
                continue;
            }
            let Some(m) = self.keyframe_models.get(&c) else { continue };
            for j in 0..m.row_count() {
                let Some(kf) = m.row_data(j) else { continue };
                if (kf.time - target.time).abs() <= eps_t && (kf.value - target.value).abs() <= eps_v {
                    list.push((c, j));
                }
            }
        }
        if list.is_empty() {
            return;
        }
        list.sort_unstable();
        // 当前选中者（在重合点中）的下一个；无选中则选第一个
        let cur = list
            .iter()
            .position(|&(c, j)| self.keyframe_models.get(&c).and_then(|m| m.row_data(j)).map(|k| k.selected).unwrap_or(false));
        let next_idx = match cur {
            Some(i) => (i + 1) % list.len(),
            None => 0,
        };
        let (nc, nj) = list[next_idx];
        self.pending_channel_selection = vec![nc];
        self.selection_anchor = nc as isize;
        self.deselect_all();
        self.selected_handles.clear();
        self.current_channel = nc as isize;
        self.set_keyframe_selected(nc, nj, false);
    }

    /// 关键帧拖拽。mode：
    ///   - 0：把增量累加到**所有选中**关键帧（含被拖的这个，按下时已被选中）。
    ///     增量由 Slint 按“光标相对按下位置的位移 / 缩放”计算（与节点拖拽一致），
    ///     累加（而非以按下点为基准重算）可避免标记与光标之间的反馈振荡。
    ///   - 1（Shift）：从贝塞尔关键帧“拖出手柄”（关键帧本体不动，手柄跟随光标），
    ///     避免手柄过短与关键帧重合而无法抓取。
    pub fn keyframe_dragged(&mut self, curve: usize, key: usize, delta_time: f32, delta_value: f32, mode: i32) {
        if !self.channel_editable(curve) {
            return;
        }
        if !delta_time.is_finite() || !delta_value.is_finite() {
            return;
        }
        if mode == 1 {
            self.drag_out_handle(curve, key, delta_time, delta_value);
            return;
        }
        // 收集被移动的关键帧下标：所有选中且通道可编辑的关键帧（跨通道一起动）
        let mut moved: Vec<(usize, Vec<usize>)> = Vec::new();
        for (&c, model) in self.keyframe_models.iter() {
            if !self.channel_editable(c) {
                continue;
            }
            let mut idxs = Vec::new();
            for j in 0..model.row_count() {
                let Some(kf) = model.row_data(j) else { continue };
                if kf.selected {
                    let mut kf2 = kf;
                    // 时间坐标最小单位毫秒：移动后的关键帧必须落在整数毫秒上
                    kf2.time = snap_ms(kf2.time + delta_time);
                    kf2.value += delta_value;
                    model.set_row_data(j, kf2);
                    idxs.push(j);
                }
            }
            if !idxs.is_empty() {
                moved.push((c, idxs));
            }
        }
        if moved.is_empty() {
            return;
        }

        // 任一被移动的通道乱序 → 全量重建（重新排序曲线/段/手柄/刻度）；否则局部重建
        let needs_all = moved.iter().any(|&(c, _)| !self.is_sorted(c));
        if needs_all {
            self.rebuild_all();
        } else {
            for &(c, _) in &moved {
                self.rebuild_curve(c);
            }
        }
        // 手柄预览（渲染期钳制反馈）只对拖拽起始通道的被拖关键帧做，避免多选时复杂化
        if moved.iter().any(|&(c, ref idxs)| c == curve && idxs.contains(&key)) {
            self.rebuild_handles_with_preview(curve, key);
        }
        // 通道关键帧移动后，汇总条带标记的位置/数量随之变化（拖拽元素是通道关键帧，
        // 重建汇总模型不打断其 grab）
        self.rebuild_summary();
    }

    /// Shift+拖拽关键帧“拖出手柄”：关键帧本体不动，出柄/入柄按**原点对称**调节
    /// （入柄 = -出柄），与 Shift 拖手柄的原点对称模式一致，而不是只调半边。
    /// 关键帧元素在拖出手柄模式下不移动，KeyframeMarker 上报的 dt/dv 即为
    /// 光标相对关键帧的累计偏移（世界坐标），直接作为手柄偏移（不再累加）。
    fn drag_out_handle(&mut self, curve: usize, key: usize, dt: f32, dv: f32) {
        let Some(kf_model) = self.keyframe_models.get(&curve) else {
            return;
        };
        let Some(mut kf) = kf_model.row_data(key) else {
            return;
        };
        if kf.interpolation != SlintInterpolation::Bezier {
            return; // 非贝塞尔关键帧不拖出手柄
        }
        kf.handle_out = SlintBezierHandle {
            time_offset: dt,
            value_offset: dv,
        };
        kf.handle_in = SlintBezierHandle {
            time_offset: -dt,
            value_offset: -dv,
        };
        kf_model.set_row_data(key, kf);
        self.rebuild_curve(curve);
        self.rebuild_handles_with_preview(curve, key);
    }

    /// 关键帧松开。moved=true 表示发生过拖拽（保持多选）；false 为单击 → 折叠为单选。
    /// 先写回按下时记录的通道选中（grab 已结束，此时对外层 channels 模型更新安全），
    /// 再恢复普通手柄。
    pub fn keyframe_released(&mut self, curve: usize, key: usize, moved: bool) {
        if !moved {
            if self.pressed_was_selected {
                // 单击已选中的关键帧（或该位置有已选中的重合点）：循环到下一个几乎重合的点
                self.cycle_coincident(curve, key);
            } else {
                // 首次单击：折叠为单选被按的关键帧
                self.pending_channel_selection = vec![curve];
                self.selection_anchor = curve as isize;
                self.deselect_all();
                self.selected_handles.clear();
                self.set_keyframe_selected(curve, key, false);
            }
        }
        self.apply_pending_channel_selection();
        self.rebuild_all();
    }

    // ── 摄影表汇总条带（聚合所有通道同一毫秒的关键帧） ─────────────────

    /// 汇总关键帧按下：选中该时刻的全部关键帧（跨通道）。additive=Shift/Ctrl 追加选择。
    /// 与普通关键帧不同：汇总帧代表"该时刻的全部"，因此普通点击**总是**清空并
    /// 单选该时刻全部（不会因为其中某个已选中而"保持现状"，否则拖拽时只剩已选中的
    /// 那几个会移动，与用户预期不符）；Shift/Ctrl 才追加到现有选择。
    /// 通道选中用 pending（grab 结束后写回）。
    pub fn summary_keyframe_pressed(&mut self, index: usize, additive: bool) {
        let Some(sk) = self.summary_keyframes.row_data(index) else { return };
        let target_ms = ms_of(sk.time);
        // 收集该时刻跨通道的全部关键帧（可见且可编辑的通道；与拖拽一致）
        let mut group: Vec<(usize, usize)> = Vec::new();
        for (&c, model) in self.keyframe_models.iter() {
            if !self.channel_editable(c) {
                continue;
            }
            let visible = self.channels.row_data(c).map(|ch| ch.visible).unwrap_or(true);
            if !visible {
                continue;
            }
            for j in 0..model.row_count() {
                if let Some(kf) = model.row_data(j) {
                    if ms_of(kf.time) == target_ms {
                        group.push((c, j));
                    }
                }
            }
        }
        if group.is_empty() {
            return;
        }
        let already_selected = group
            .iter()
            .any(|&(c, j)| {
                self.keyframe_models
                    .get(&c)
                    .and_then(|m| m.row_data(j))
                    .map(|k| k.selected)
                    .unwrap_or(false)
            });
        self.pressed_was_selected = already_selected;
        if additive {
            // 加入：全部选上（不取消），并确保其通道在期望选中集合里
            for &(c, j) in &group {
                self.set_keyframe_selected(c, j, false);
            }
            let mut sel = self.effective_selected_channels();
            for &(c, _) in &group {
                if !sel.contains(&c) {
                    sel.push(c);
                }
            }
            // 立即写回通道选中（追加，不清空其它）；汇总标记独立于 channels 嵌套循环，
            // 按下期间更新安全——拖动一开始就亮起所有关联轨道，无需等松开二次选中。
            for &c in &sel {
                self.set_channel_selected(c, true);
            }
            self.pending_channel_selection.clear();
        } else {
            // 普通点击：无论该时刻是否已有选中，都清空并单选该时刻全部
            let mut chans: Vec<usize> = group.iter().map(|&(c, _)| c).collect();
            chans.sort_unstable();
            chans.dedup();
            self.selection_anchor = chans[0] as isize;
            self.deselect_all();
            for &(c, j) in &group {
                self.set_keyframe_selected(c, j, false);
            }
            // 立即写回通道选中（替换选择）
            self.clear_channel_selection();
            for &c in &chans {
                self.set_channel_selected(c, true);
            }
            self.pending_channel_selection.clear();
        }
        self.current_channel = group[0].0 as isize;
        // 按下立即高亮（视觉反馈，不用等松开重建）：汇总标记独立于 channels 嵌套循环，
        // 按下期间更新本行模型安全（不会破坏 Slint 拖拽抓取）。
        if let Some(mut sk) = self.summary_keyframes.row_data(index) {
            let n = sk.selected.row_count();
            sk.selected = ModelRc::from(Rc::new(slint::VecModel::from(vec![true; n])));
            sk.all_selected = true;
            self.summary_keyframes.set_row_data(index, sk);
        }
    }

    /// 汇总关键帧拖拽：把增量累加到全部选中关键帧的时间上（跨通道同时移动，
    /// 同一时刻的关键帧保持同步），时间对齐到整数毫秒。
    /// `index` 为被拖的汇总标记下标：就地更新其 time（只改数据不改结构，
    /// 保住 Slint 拖拽抓取）；不整体重建汇总模型（set_vec 会重建元素、打断 grab）。
    pub fn summary_keyframe_dragged(&mut self, index: usize, delta_time: f32) {
        if !delta_time.is_finite() {
            return;
        }
        let mut moved: Vec<usize> = Vec::new();
        for (&c, model) in self.keyframe_models.iter() {
            if !self.channel_editable(c) {
                continue;
            }
            let mut changed = false;
            for j in 0..model.row_count() {
                let Some(kf) = model.row_data(j) else { continue };
                if kf.selected {
                    let mut kf2 = kf;
                    kf2.time = snap_ms(kf2.time + delta_time);
                    model.set_row_data(j, kf2);
                    changed = true;
                }
            }
            if changed {
                moved.push(c);
            }
        }
        if moved.is_empty() {
            return;
        }
        // 局部重建（曲线段/手柄/摄影表连接线随关键帧移动刷新）；不触发汇总模型重建
        for &c in &moved {
            self.rebuild_curve(c);
        }
        // 就地更新被拖汇总标记的 time，让标记跟随光标
        if let Some(sk) = self.summary_keyframes.row_data(index) {
            let mut sk2 = sk;
            sk2.time = snap_ms(sk2.time + delta_time);
            self.summary_keyframes.set_row_data(index, sk2);
        }
    }

    /// 汇总关键帧松开。通道选中已在按下时立即写回，无需二次选中；
    /// 松开只需重建视图（曲线段/手柄/汇总标记随关键帧状态刷新）。
    pub fn summary_keyframe_released(&mut self, _moved: bool) {
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

    /// 当前 Action 是否有关键帧数据（全局 Action 的"向下插入"按钮据此显示）。
    pub fn has_keyframes(&self) -> bool {
        self.keyframe_models.values().any(|m| m.row_count() > 0)
    }

    /// 清除全部关键帧选中与控制柄选中（手柄模型就地重建以刷新 selected 视觉）。
    /// 注意：只清 keyframe 的 selected 字段（内层模型 set_row_data）与 selected_handles，
    /// 不改 channels 外层模型，因此关键帧拖拽期间调用安全。
    /// 同时清空摄影表汇总标记的选中掩码（独立层，拖拽期间更新安全）。
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
        for i in 0..self.summary_keyframes.row_count() {
            let Some(mut sk) = self.summary_keyframes.row_data(i) else {
                continue;
            };
            if sk.selected.iter().any(|s| s) {
                let n = sk.selected.row_count();
                sk.selected = ModelRc::from(Rc::new(slint::VecModel::from(vec![false; n])));
                sk.all_selected = false;
                self.summary_keyframes.set_row_data(i, sk);
            }
        }
        if !self.selected_handles.is_empty() {
            self.selected_handles.clear();
            self.rebuild_all_handles();
        }
    }

    /// 全量重建手柄模型（就地 set_vec 到 Slint 的 handles 模型）。
    /// 手柄是扁平模型（非嵌套于 channels 循环），拖拽期间更新安全。
    fn rebuild_all_handles(&mut self) {
        let n = self.keyframe_models.len();
        let mut handles: Vec<SlintHandleMarker> = Vec::new();
        let mut hdl_offsets = vec![0usize; n + 1];
        for c in 0..n {
            handles.extend(self.curve_elements(c).1);
            hdl_offsets[c + 1] = handles.len();
        }
        self.handle_offsets = hdl_offsets;
        self.handles.set_vec(handles);
    }

    // ── 贝塞尔控制柄 ─────────────────────────────────────────

    /// 控制柄按下：激活所在通道。锁定通道不可交互。
    /// 控制柄按下：激活所在通道 + 选中该手柄（additive=Shift/Ctrl 追加）。
    /// 语义对齐关键帧：普通点未选中的手柄清空重选；点已选中的手柄保持多选；
    /// Shift/Ctrl 加入选择（不切换取消，保证被拖手柄保持选中，拖拽增量才能正确计算）。
    /// 锁定通道不可交互。
    pub fn handle_pressed(&mut self, curve: usize, key: usize, is_in: bool, additive: bool) {
        if !self.channel_editable(curve) {
            return;
        }
        let key_in_sel = self.selected_handles.contains(&(curve, key, is_in));
        if additive {
            self.selected_handles.insert((curve, key, is_in));
        } else if !key_in_sel {
            self.deselect_all(); // 清关键帧与其它手柄选中
            self.selected_handles.insert((curve, key, is_in));
        }
        // 拖控制柄也聚焦并选中该通道（曲线保持高亮）
        self.channel_pressed(curve, 0);
        // 就地重建本通道手柄，刷新 selected 视觉（手柄为扁平模型，拖拽期间更新安全）
        self.rebuild_curve(curve);
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
        self.pending_channel_selection.clear();
        self.clear_channel_selection();
    }

    /// 把按下关键帧时记录的期望通道选中写回 channels 模型。
    /// 只能在关键帧 grab 结束（松开）后调用，否则外层模型更新会破坏 Slint 拖拽抓取。
    fn apply_pending_channel_selection(&mut self) {
        let pending = std::mem::take(&mut self.pending_channel_selection);
        self.clear_channel_selection();
        for &i in &pending {
            self.set_channel_selected(i, true);
        }
    }

    /// 当前"生效"的选中通道集合 = 已写回 channels 模型的选中 ∪ 尚未应用的 pending 选中。
    /// 连续按下（两次按下之间未松开）时，通道模型可能还没写回，需要把 pending 也算上，
    /// 否则 Shift 追加会丢掉上一个按下的通道。
    fn effective_selected_channels(&self) -> Vec<usize> {
        (0..self.channels.row_count())
            .filter(|&i| self.channel_selected(i) || self.pending_channel_selection.contains(&i))
            .collect()
    }

    /// 摄影表空白处框选：按世界时间范围 [x_lo, x_hi] 与行（通道）区间 [row_lo, row_hi]
    /// 选中范围内、且通道可见的关键帧（菱形中心落在框内）。
    /// mode 0 = 替换现有选择；非 0（Shift/Ctrl）= 追加到现有选择。
    /// 只在框选结束（grab 已结束）后调用，可直接更新 channels 外层模型。
    pub fn dope_marquee_select(&mut self, x_lo: f32, x_hi: f32, row_lo: i32, row_hi: i32, mode: i32) {
        let n = self.channels.row_count() as i32;
        if n <= 0 {
            return;
        }
        let r_lo = row_lo.max(0).min(n - 1);
        let r_hi = row_hi.max(0).min(n - 1);
        let (r_lo, r_hi) = (r_lo.min(r_hi), r_lo.max(r_hi));
        let (t_lo, t_hi) = if x_lo <= x_hi { (x_lo, x_hi) } else { (x_hi, x_lo) };
        if t_hi < t_lo {
            return;
        }

        let additive = mode != 0;
        if !additive {
            self.deselect_all(); // 替换：先清掉所有关键帧选中
            self.pending_channel_selection.clear();
        }

        // 第一遍：命中并选中范围内的关键帧（选中态直接写回内层模型）
        let mut hit: Vec<usize> = Vec::new();
        for curve in r_lo as usize..=r_hi as usize {
            let visible = self.channels.row_data(curve).map(|c| c.visible).unwrap_or(false);
            if !visible {
                continue;
            }
            let Some(model) = self.keyframe_models.get(&curve) else { continue };
            let mut changed = false;
            for j in 0..model.row_count() {
                let Some(kf) = model.row_data(j) else { continue };
                if kf.time >= t_lo && kf.time <= t_hi && !kf.selected {
                    let mut kf2 = kf;
                    kf2.selected = true;
                    model.set_row_data(j, kf2);
                    changed = true;
                }
            }
            if changed {
                hit.push(curve);
            }
        }

        // 第二遍：通道选中（替换 = 只留命中的；追加 = 确保命中通道被选中、保留其余）
        if additive {
            for curve in &hit {
                self.set_channel_selected(*curve, true);
            }
        } else {
            self.clear_channel_selection();
            for curve in &hit {
                self.set_channel_selected(*curve, true);
            }
        }
        // 命中通道的段/手柄随关键帧选中态重算
        for curve in &hit {
            self.rebuild_curve(*curve);
        }
        if let Some(&first) = hit.first() {
            self.current_channel = first as isize;
        }
        self.rebuild_summary();
    }

    /// 曲线编辑器空白处框选：按世界时间范围 [x_lo, x_hi] × 值范围 [y_lo, y_hi]
    /// 选中框内的**关键帧**（菱形中心）与**贝塞尔控制柄**（控制点），跳过不可见通道。
    /// mode 0 = 替换现有选择；非 0（Shift/Ctrl）= 追加。
    /// 只在框选结束（grab 已结束）后调用，可直接更新 channels 外层模型。
    pub fn graph_marquee_select(&mut self, x_lo: f32, x_hi: f32, y_lo: f32, y_hi: f32, mode: i32) {
        let (t_lo, t_hi) = if x_lo <= x_hi { (x_lo, x_hi) } else { (x_hi, x_lo) };
        let (v_lo, v_hi) = if y_lo <= y_hi { (y_lo, y_hi) } else { (y_hi, y_lo) };
        let additive = mode != 0;

        if !additive {
            self.deselect_all(); // 清关键帧 + 手柄选中
            self.pending_channel_selection.clear();
        }

        let mut hit_channels: Vec<usize> = Vec::new();
        for curve in 0..self.channels.row_count() {
            let visible = self.channels.row_data(curve).map(|c| c.visible).unwrap_or(false);
            if !visible {
                continue;
            }
            let Some(model) = self.keyframe_models.get(&curve) else { continue };
            let mut kf_changed = false;
            for j in 0..model.row_count() {
                let Some(kf) = model.row_data(j) else { continue };
                // 手柄：控制点（关键帧 + 偏移）落入框内即选中
                for si in [true, false] {
                    let (ho_t, ho_v) = if si {
                        (kf.handle_in.time_offset, kf.handle_in.value_offset)
                    } else {
                        (kf.handle_out.time_offset, kf.handle_out.value_offset)
                    };
                    let hx = kf.time + ho_t;
                    let hy = kf.value + ho_v;
                    if hx >= t_lo && hx <= t_hi && hy >= v_lo && hy <= v_hi {
                        self.selected_handles.insert((curve, j, si));
                    }
                }
                // 关键帧菱形中心落入框内
                let kf_in_box = kf.time >= t_lo && kf.time <= t_hi && kf.value >= v_lo && kf.value <= v_hi;
                if kf_in_box && !kf.selected {
                    let mut kf2 = kf;
                    kf2.selected = true;
                    model.set_row_data(j, kf2);
                    kf_changed = true;
                }
            }
            if kf_changed {
                hit_channels.push(curve);
            }
        }

        // 框住曲线本身也选中所在通道：判断该通道是否有任一段曲线与框选矩形相交
        // （用三次贝塞尔采样点粗判：任一采样点落入框内即算相交）。
        let mut curve_hit: Vec<usize> = Vec::new();
        for row in 0..self.graph_segments.row_count() {
            let Some(seg) = self.graph_segments.row_data(row) else { continue };
            let curve = seg.curve_index as usize;
            if curve_hit.contains(&curve) {
                continue;
            }
            let visible = self.channels.row_data(curve).map(|c| c.visible).unwrap_or(false);
            if !visible {
                continue;
            }
            if segment_intersects_rect(&seg, t_lo, t_hi, v_lo, v_hi) {
                curve_hit.push(curve);
            }
        }
        for c in curve_hit {
            if !hit_channels.contains(&c) {
                hit_channels.push(c);
            }
        }

        // 通道选中：替换 = 只留命中通道；追加 = 确保命中通道被选中、保留其余
        if additive {
            for &c in &hit_channels {
                self.set_channel_selected(c, true);
            }
        } else {
            self.clear_channel_selection();
            for &c in &hit_channels {
                self.set_channel_selected(c, true);
            }
        }
        // 手柄 selected 视觉全量重建（跨通道）；命中通道的段随关键帧选中态重算
        self.rebuild_all_handles();
        for &c in &hit_channels {
            self.rebuild_curve(c);
        }
        if let Some(&c) = hit_channels.first() {
            self.current_channel = c as isize;
        }
        self.rebuild_summary();
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

        // 多选同侧手柄：各自按相同屏幕偏移移动（世界增量一致 → 屏幕位移一致，更符合预期）。
        // 非主手柄同样做“自己关键帧一侧”的硬钳制；软钳制预览对所有被移动的手柄都做。
        let mut synced: Vec<(usize, usize, bool)> = Vec::new();
        if (dt != 0.0 || dv != 0.0) && !self.selected_handles.is_empty() {
            let sel: Vec<(usize, usize, bool)> = self.selected_handles.iter().copied().collect();
            for (c, k, si) in sel {
                if c == curve && k == key && si == is_in {
                    continue; // 被拖手柄已更新
                }
                if si != is_in {
                    continue; // 只同步同一侧
                }
                if !self.channel_editable(c) {
                    continue;
                }
                let Some(model) = self.keyframe_models.get(&c) else { continue };
                let Some(mut kf2) = model.row_data(k) else { continue };
                if si {
                    kf2.handle_in.time_offset += dt;
                    kf2.handle_in.value_offset += dv;
                    clamp_handle_own_side(&mut kf2, true);
                } else {
                    kf2.handle_out.time_offset += dt;
                    kf2.handle_out.value_offset += dv;
                    clamp_handle_own_side(&mut kf2, false);
                }
                // 与主手柄一致的模式：对每个被同步手柄的对手柄同样应用（对齐/原点对称/自由）
                apply_handle_mode(&mut kf2, si, mode);
                clamp_handle_own_side(&mut kf2, true);
                clamp_handle_own_side(&mut kf2, false);
                model.set_row_data(k, kf2);
                synced.push((c, k, si));
            }
        }

        // 主手柄：按模式约束其对手柄
        apply_handle_mode(&mut kf, is_in, mode);

        // 硬钳制：被拖柄与对手柄都钳到“自己关键帧”一侧；
        // 相邻关键帧边界为软钳制（渲染期交叉钳制处理）
        clamp_handle_own_side(&mut kf, true);
        clamp_handle_own_side(&mut kf, false);
        kf_model.set_row_data(key, kf);

        // 重建被拖通道 + 被同步通道的手柄/曲线
        self.rebuild_curve(curve);
        let mut rebuilt: Vec<usize> = vec![curve];
        for &(c, _, _) in &synced {
            if !rebuilt.contains(&c) {
                rebuilt.push(c);
                self.rebuild_curve(c);
            }
        }
        // 软钳制预览：主手柄 + 所有被同步手柄的有效控制点（与原始不同才显示）
        let mut preview: Vec<SlintHandleMarker> = Vec::new();
        for &(c, k, si) in synced.iter().chain(std::iter::once(&(curve, key, is_in))) {
            if let Some(p) = self.dragged_handle_preview(c, k, si) {
                preview.push(p);
            }
        }
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
            Some(mk_marker(curve, key, color, &kfs[pos].1, eff.0, eff.1, is_in, true, false))
        } else {
            None
        }
    }

    /// 控制柄松开。moved=true 表示发生过拖拽（保持多选）；false 为单击 → 折叠为单选该手柄。
    pub fn handle_released(&mut self, curve: usize, key: usize, is_in: bool, moved: bool) {
        if !moved {
            // 单击已选中的手柄：折叠为单选
            self.deselect_all(); // 清关键帧与其它手柄选中
            self.selected_handles.clear();
            self.selected_handles.insert((curve, key, is_in));
            self.rebuild_all_handles();
        }
        self.preview_handles.set_vec(Vec::new());
    }

    // ── 通道开关 ─────────────────────────────────────────────

    pub fn toggle_channel_visible(&mut self, curve: usize) {
        if let Some(mut ch) = self.channels.row_data(curve) {
            ch.visible = !ch.visible;
            self.channels.set_row_data(curve, ch);
        }
        self.rebuild_dope_connections();
        self.rebuild_summary();
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

    /// 视图缩放变化：更新命中检测容差依据（刻度已完全移到 Slint 侧即时计算，
    /// 见 ui/areas/graph_editor/ruler.slint 顶部注释；pan/尺寸不再需要 Rust 侧状态）。
    pub fn set_view(
        &mut self,
        zoom_x: f32,
        zoom_y: f32,
        _pan_x: f32,
        _pan_y: f32,
        _width: f32,
        _height: f32,
    ) {
        if !zoom_x.is_finite() || zoom_x <= 0.0 {
            return;
        }
        self.time_zoom = zoom_x;
        self.value_zoom = zoom_y.max(0.01);
    }
}

// ── 派生：曲线段 / 控制柄 ────────────────────────────────────────────────

/// 三次贝塞尔上参数 t 处的点。
fn cubic_point(
    x0: f32,
    y0: f32,
    cx1: f32,
    cy1: f32,
    cx2: f32,
    cy2: f32,
    x1: f32,
    y1: f32,
    t: f32,
) -> (f32, f32) {
    let mt = 1.0 - t;
    // B(t) = (1-t)^3 P0 + 3(1-t)^2 t P1 + 3(1-t) t^2 P2 + t^3 P3
    let x = mt * mt * mt * x0 + 3.0 * mt * mt * t * cx1 + 3.0 * mt * t * t * cx2 + t * t * t * x1;
    let y = mt * mt * mt * y0 + 3.0 * mt * mt * t * cy1 + 3.0 * mt * t * t * cy2 + t * t * t * y1;
    (x, y)
}

/// 曲线段与框选矩形是否相交——对齐 Blender 的 `rectf_curve_intersection`（graph_select.cc）：
/// 在矩形的 x 范围内均匀采样（每段曲线 x 单调，控制点已被钳制到 [x0,x1]），
/// 对每个采样 x 求曲线在该点的 y 值，判断它相对矩形是上方/内部/下方；
/// 任一采样点在内部、或相邻两采样点跨越矩形上下边界（从一侧穿到另一侧）即相交。
/// 与 Blender 一样在 x 轴上采样（而非 t 参数），放大/弯曲剧烈时也不会漏检。
fn segment_intersects_rect(
    seg: &SlintCurveSegment,
    t_lo: f32,
    t_hi: f32,
    v_lo: f32,
    v_hi: f32,
) -> bool {
    // 只在该段曲线的 x 范围与框 x 范围的重叠区间内采样
    let x_min = seg.x0.max(t_lo);
    let x_max = seg.x1.min(t_hi);
    if x_max <= x_min {
        return false;
    }
    // Blender 用 30 个采样点
    const SAMPLES: usize = 30;
    // 求曲线在给定 x 处的 y（段内 x 单调 → 二分 t）
    let y_at = |x: f32| -> f32 {
        let mut lo = 0.0f32;
        let mut hi = 1.0f32;
        for _ in 0..40 {
            let mid = (lo + hi) * 0.5;
            let (px, _) = cubic_point(
                seg.x0, seg.y0, seg.cx1, seg.cy1, seg.cx2, seg.cy2, seg.x1, seg.y1, mid,
            );
            if px < x {
                lo = mid;
            } else {
                hi = mid;
            }
        }
        let (_, py) = cubic_point(
            seg.x0, seg.y0, seg.cx1, seg.cy1, seg.cx2, seg.cy2, seg.x1, seg.y1, (lo + hi) * 0.5,
        );
        py
    };
    // zone：1=上方，0=内部，-1=下方（Blender 的 ABOVE/INSIDE/BELOW）
    let zone = |y: f32| -> i8 {
        if y < v_lo {
            -1
        } else if y <= v_hi {
            0
        } else {
            1
        }
    };
    let mut prev = zone(y_at(x_min));
    if prev == 0 {
        return true;
    }
    for i in 1..=SAMPLES {
        let x = x_min + (x_max - x_min) * (i as f32 / SAMPLES as f32);
        let z = zone(y_at(x));
        if z == 0 || z != prev {
            return true;
        }
        prev = z;
    }
    false
}

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
    selected: bool,
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
        selected,
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

/// 对关键帧应用手柄模式（作用于被调柄的**对手柄**）：
///  - 1 = 对齐：对手柄旋转到与被调柄共线（方向相反），世界长度保持对手柄自身；
///  - 2 = 原点对称：对手柄 = -被调柄（角度+长度都镜像）；
///  - 0 / 其他 = 自由：对手柄不动。
/// `adjusted_side` 为被调整手柄的一侧（true=入柄，false=出柄），其偏移已写入 kf。
fn apply_handle_mode(kf: &mut SlintKeyframe, adjusted_side: bool, mode: i32) {
    let (t, v) = if adjusted_side {
        (kf.handle_in.time_offset, kf.handle_in.value_offset)
    } else {
        (kf.handle_out.time_offset, kf.handle_out.value_offset)
    };
    match mode {
        // 对齐：对手柄旋转到与被调柄共线（角度相反），世界长度保持自身
        1 => {
            let nl = (t * t + v * v).sqrt();
            if nl > 1e-9 {
                let len = if adjusted_side {
                    (kf.handle_out.time_offset.powi(2) + kf.handle_out.value_offset.powi(2)).sqrt()
                } else {
                    (kf.handle_in.time_offset.powi(2) + kf.handle_in.value_offset.powi(2)).sqrt()
                };
                let new = SlintBezierHandle { time_offset: -t / nl * len, value_offset: -v / nl * len };
                if adjusted_side {
                    kf.handle_out = new;
                } else {
                    kf.handle_in = new;
                }
            }
        }
        // 原点对称：对手柄 = -被调柄（角度+长度都镜像）
        2 => {
            let new = SlintBezierHandle { time_offset: -t, value_offset: -v };
            if adjusted_side {
                kf.handle_out = new;
            } else {
                kf.handle_in = new;
            }
        }
        // 自由 / 其他：对手柄不动
        _ => {}
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

// ── AppStore 入口 ─────────────────────────────────────────────────────────

impl crate::store::AppStore {
    /// 初始化曲线编辑器与 NLA 编辑器：把 Rust 持有的模型绑定到 Slint 的 AnimationState 全局。
    pub fn initialize_animation_editors(&mut self, instance: &crate::bridge::types::MainWindow) {
        use slint::ComponentHandle;

        let state = instance.global::<crate::bridge::types::AnimationState>();
        state.set_channels(ModelRc::from(self.ui_models.curve_editor.channels.clone()));
        state.set_graph_channels(self.ui_models.curve_editor.graph_filter.clone());
        state.set_dope_channels(self.ui_models.curve_editor.dope_filter.clone());
        state.set_graph_segments(self.ui_models.curve_editor.graph_segments.clone());
        state.set_graph_handles(self.ui_models.curve_editor.graph_handles.clone());
        state.set_segments(ModelRc::from(self.ui_models.curve_editor.segments.clone()));
        state.set_handles(ModelRc::from(self.ui_models.curve_editor.handles.clone()));
        state.set_dope_connections(ModelRc::from(self.ui_models.curve_editor.dope_connections.clone()));
        state.set_dope_summary_keyframes(ModelRc::from(self.ui_models.curve_editor.summary_keyframes.clone()));
        state.set_nla_tracks(ModelRc::from(self.ui_models.nla_editor.tracks_filter.clone()));
        state.set_current_time(self.ui_models.curve_editor.current_time as f32);
        state.set_action_name(self.ui_models.curve_editor.action_name.clone());
        state.set_action_items(ModelRc::from(self.ui_models.curve_editor.action_items.clone()));
        state.set_current_action(self.ui_models.curve_editor.current_action);
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
        st.keyframe_dragged(0, 1, 1.0, -5.0, 0);

        let kf = st.keyframe_models[&0].row_data(1).unwrap();
        assert_eq!(kf.time, 3.0);
        assert_eq!(kf.value, 5.0);
        assert!(kf.selected);

        // 段同步更新：段 0 终点 = 新位置
        let seg0 = st.segments.row_data(0).unwrap();
        assert_eq!(seg0.x1, 3.0);
        assert_eq!(seg0.y1, 5.0);

        // 多次拖拽累加（与节点拖拽一致）
        st.keyframe_dragged(0, 1, 0.5, 2.0, 0);
        let kf = st.keyframe_models[&0].row_data(1).unwrap();
        assert_eq!(kf.time, 3.5);
        assert_eq!(kf.value, 7.0);
        st.keyframe_released(0, 1, true);
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
    fn action_selection_switches_curves() {
        let mut st = CurveEditorState::default();
        // 默认：一个空"主 Action"
        assert_eq!(st.current_action, 0);
        assert_eq!(st.action_name.as_str(), "主 Action");
        st.rebuild_action_menu();
        assert_eq!(st.action_items.row_count(), 1);

        // 两个 Action，通道与关键帧不同
        st.actions = vec![
            ActionData {
                name: "主 Action".into(),
                channels: vec![(
                    "A".into(),
                    Color::from_rgb_u8(255, 0, 0),
                    vec![
                        key(0.0, 0.0, SlintInterpolation::Linear),
                        key(4.0, 4.0, SlintInterpolation::Linear),
                    ],
                )],
            },
            ActionData {
                name: "奔跑".into(),
                channels: vec![(
                    "B".into(),
                    Color::from_rgb_u8(0, 255, 0),
                    vec![
                        key(1.0, 10.0, SlintInterpolation::Linear),
                        key(2.0, 20.0, SlintInterpolation::Linear),
                    ],
                )],
            },
        ];
        st.action_names = st.actions.iter().map(|a| a.name.clone()).collect();
        st.rebuild_action_menu();
        assert_eq!(st.action_items.row_count(), 2);

        st.select_action(0);
        assert_eq!(st.current_action, 0);
        assert_eq!(st.action_name.as_str(), "主 Action");
        assert_eq!(st.channels.row_count(), 1);
        assert_eq!(st.channels.row_data(0).unwrap().name.as_str(), "A");
        assert_eq!((st.action_start, st.action_end), (0.0, 4.0));

        // 切换 Action：通道/关键帧/时长范围全部换掉
        st.select_action(1);
        assert_eq!(st.current_action, 1);
        assert_eq!(st.action_name.as_str(), "奔跑");
        assert_eq!(st.channels.row_count(), 1);
        assert_eq!(st.channels.row_data(0).unwrap().name.as_str(), "B");
        assert_eq!((st.action_start, st.action_end), (1.0, 2.0));

        // 负数清空
        st.select_action(-1);
        assert_eq!(st.current_action, -1);
        assert_eq!(st.action_name.as_str(), "");
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

        st.handle_pressed(0, 0, false, false);
        st.handle_dragged(0, 0, false, 0.5, 3.0, 0);
        st.handle_dragged(0, 0, false, 0.2, 1.0, 0);

        let kf = st.keyframe_models[&0].row_data(0).unwrap();
        assert_eq!(kf.handle_out.time_offset, 0.7);
        assert_eq!(kf.handle_out.value_offset, 4.0);

        st.handle_released(0, 0, false, true);
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
        st.handle_pressed(0, 0, false, false);
        st.handle_dragged(0, 0, false, 2.0, 3.0, 0);
        let kf = st.keyframe_models[&0].row_data(0).unwrap();
        assert_eq!(kf.handle_out.time_offset, 2.0);
        assert_eq!(kf.handle_out.value_offset, 3.0);
        assert_eq!(kf.handle_in.time_offset, 0.0);
        assert_eq!(kf.handle_in.value_offset, 0.0);
        st.handle_released(0, 0, false, true);

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
        st.handle_pressed(0, 0, false, false);
        // 拖出柄 (1,0) → 累加 (1,1) 得 (2,1)；入柄旋转到共线方向、长度保持 4
        st.handle_dragged(0, 0, false, 1.0, 1.0, 1);
        let kf = st.keyframe_models[&0].row_data(0).unwrap();
        assert_eq!(kf.handle_out.time_offset, 2.0);
        assert_eq!(kf.handle_out.value_offset, 1.0);
        let nl = (2f32 * 2f32 + 1f32).sqrt();
        assert!((kf.handle_in.time_offset - (-2.0 / nl * 4.0)).abs() < 1e-4);
        assert!((kf.handle_in.value_offset - (-1.0 / nl * 4.0)).abs() < 1e-4);
        st.handle_released(0, 0, false, true);

        // 模式 2 原点对称：入柄 = -出柄（角度+长度都镜像）
        let mut st = setup();
        st.handle_pressed(0, 0, false, false);
        st.handle_dragged(0, 0, false, 2.0, 3.0, 2);
        let kf = st.keyframe_models[&0].row_data(0).unwrap();
        assert_eq!(kf.handle_in.time_offset, -2.0);
        assert_eq!(kf.handle_in.value_offset, -3.0);
        st.handle_released(0, 0, false, true);
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
        st.handle_pressed(0, 0, false, false);
        st.handle_dragged(0, 0, false, -10.0, 0.0, 0);
        let kf = st.keyframe_models[&0].row_data(0).unwrap();
        assert_eq!(kf.handle_out.time_offset, 0.0);
        st.handle_released(0, 0, false, true);

        // 入柄拖 +10（越过自己关键帧之后）→ 硬钳到 0
        st.handle_pressed(0, 1, true, false);
        st.handle_dragged(0, 1, true, 10.0, 0.0, 0);
        let kf = st.keyframe_models[&0].row_data(1).unwrap();
        assert_eq!(kf.handle_in.time_offset, 0.0);
        st.handle_released(0, 1, true, true);

        // 出柄拖 +10（越过相邻关键帧）→ 软钳制，手柄本身保持 10（曲线由渲染期钳制）
        st.handle_pressed(0, 0, false, false);
        st.handle_dragged(0, 0, false, 10.0, 0.0, 0);
        let kf = st.keyframe_models[&0].row_data(0).unwrap();
        assert_eq!(kf.handle_out.time_offset, 10.0);
        st.handle_released(0, 0, false, true);
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
        st.keyframe_dragged(0, 1, 3.0, 0.0, 0);

        // 模型顺序不变（索引 1 仍是它），时间变成 5
        let kf = st.keyframe_models[&0].row_data(1).unwrap();
        assert_eq!(kf.time, 5.0);

        // 曲线段按时间排序重绘：(0,0)-(4,20) 然后 (4,20)-(5,10)
        let seg0 = st.segments.row_data(0).unwrap();
        assert_eq!((seg0.x0, seg0.y0, seg0.x1, seg0.y1), (0.0, 0.0, 4.0, 20.0));
        let seg1 = st.segments.row_data(1).unwrap();
        assert_eq!((seg1.x0, seg1.y0, seg1.x1, seg1.y1), (4.0, 20.0, 5.0, 10.0));
        st.keyframe_released(0, 1, true);
    }

    #[test]
    fn dope_drag_moves_only_time_and_is_shared_source() {
        // 摄影表拖拽路径 = keyframe_dragged(curve, key, dt, dv=0)：只移动时间。
        // 摄影表与曲线编辑器必须同源：channels[i].keyframes 指向与权威 keyframe_models 同一 inner 模型，
        // 拖拽后曲线编辑器的 segments 也同步重算。
        let mut st = CurveEditorState::default();
        st.push_channel(
            "t",
            Color::from_rgb_u8(255, 0, 0),
            vec![
                key(0.0, 5.0, SlintInterpolation::Linear),
                key(2.0, 10.0, SlintInterpolation::Linear),
                key(4.0, 20.0, SlintInterpolation::Linear),
            ],
        );
        st.rebuild_all();

        st.keyframe_pressed(0, 1, false);
        st.keyframe_dragged(0, 1, 2.5, 0.0, 0);

        // 权威模型：时间移动 2.5、值不变（少一个维度）
        let kf = st.keyframe_models[&0].row_data(1).unwrap();
        assert_eq!(kf.time, 4.5);
        assert_eq!(kf.value, 10.0);

        // 同源：渲染给 Slint 的通道投影指向同一 inner 模型（摄影表/曲线编辑器读到同一关键帧）
        let ch = st.channels.row_data(0).unwrap();
        let proj = ch.keyframes.row_data(1).unwrap();
        assert_eq!(proj.time, 4.5);
        assert_eq!(proj.value, 10.0);

        // 曲线编辑器视图同步：松开后段按新时间排序重算
        st.keyframe_released(0, 1, true);
        let seg0 = st.segments.row_data(0).unwrap();
        let seg1 = st.segments.row_data(1).unwrap();
        assert_eq!((seg0.x0, seg0.y0, seg0.x1, seg0.y1), (0.0, 5.0, 4.0, 20.0));
        assert_eq!((seg1.x0, seg1.y0, seg1.x1, seg1.y1), (4.0, 20.0, 4.5, 10.0));
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
    fn dope_marquee_select_picks_keyframes_in_box() {
        let mut st = CurveEditorState::default();
        st.push_channel(
            "a",
            Color::from_rgb_u8(255, 0, 0),
            vec![
                key(0.0, 5.0, SlintInterpolation::Linear),
                key(2.0, 10.0, SlintInterpolation::Linear),
                key(4.0, 15.0, SlintInterpolation::Linear),
            ],
        );
        st.push_channel(
            "b",
            Color::from_rgb_u8(0, 255, 0),
            vec![
                key(1.0, 5.0, SlintInterpolation::Linear),
                key(3.0, 10.0, SlintInterpolation::Linear),
            ],
        );
        st.push_channel(
            "c",
            Color::from_rgb_u8(0, 0, 255),
            vec![key(0.0, 5.0, SlintInterpolation::Linear)],
        );
        st.rebuild_all();

        // 框选时间 [0, 2.5] × 行 [0,1]：
        // 行 0 (a)：time 0、2 命中，4 不命中；行 1 (b)：time 1 命中、3 不命中；行 2 (c) 不在框内
        st.dope_marquee_select(0.0, 2.5, 0, 1, 0);

        let a0 = st.keyframe_models[&0].row_data(0).unwrap().selected;
        let a1 = st.keyframe_models[&0].row_data(1).unwrap().selected;
        let a2 = st.keyframe_models[&0].row_data(2).unwrap().selected;
        let b0 = st.keyframe_models[&1].row_data(0).unwrap().selected;
        let b1 = st.keyframe_models[&1].row_data(1).unwrap().selected;
        let c0 = st.keyframe_models[&2].row_data(0).unwrap().selected;
        assert_eq!((a0, a1, a2), (true, true, false));
        assert_eq!((b0, b1), (true, false));
        assert!(!c0, "框外的行不应被选中");
        // 通道选中：只命中行 0、1
        assert_eq!(st.selected_channel_count(), 2);
        assert!(st.channel_selected(0) && st.channel_selected(1) && !st.channel_selected(2));
        assert_eq!(st.current_channel, 0);
    }

    #[test]
    fn dope_marquee_select_additive_keeps_existing() {
        let mut st = CurveEditorState::default();
        st.push_channel(
            "a",
            Color::from_rgb_u8(255, 0, 0),
            vec![key(0.0, 5.0, SlintInterpolation::Linear), key(2.0, 10.0, SlintInterpolation::Linear)],
        );
        st.push_channel(
            "b",
            Color::from_rgb_u8(0, 255, 0),
            vec![key(1.0, 5.0, SlintInterpolation::Linear)],
        );
        st.rebuild_all();

        // 先替换选 a0；再用 Shift(mode=1) 追加选 b0
        st.dope_marquee_select(0.0, 0.5, 0, 0, 0);
        st.dope_marquee_select(1.0, 1.5, 1, 1, 1);

        assert!(st.keyframe_models[&0].row_data(0).unwrap().selected);
        assert!(st.keyframe_models[&1].row_data(0).unwrap().selected);
        assert_eq!(st.selected_channel_count(), 2);
    }

    #[test]
    fn dope_marquee_select_ignores_invisible_channels() {
        let mut st = CurveEditorState::default();
        st.push_channel(
            "a",
            Color::from_rgb_u8(255, 0, 0),
            vec![key(0.0, 5.0, SlintInterpolation::Linear)],
        );
        st.push_channel(
            "b",
            Color::from_rgb_u8(0, 255, 0),
            vec![key(0.0, 5.0, SlintInterpolation::Linear)],
        );
        st.rebuild_all();
        // 关掉 b 的眼睛
        let mut b = st.channels.row_data(1).unwrap();
        b.visible = false;
        st.channels.set_row_data(1, b);

        st.dope_marquee_select(-1.0, 10.0, 0, 1, 0);
        assert!(st.keyframe_models[&0].row_data(0).unwrap().selected);
        assert!(!st.keyframe_models[&1].row_data(0).unwrap().selected, "不可见通道的关键帧不应被框选");
        assert_eq!(st.selected_channel_count(), 1);
    }

    #[test]
    fn keyframe_drag_moves_all_selected_keyframes() {
        let mut st = CurveEditorState::default();
        st.push_channel(
            "a",
            Color::from_rgb_u8(255, 0, 0),
            vec![
                key(0.0, 0.0, SlintInterpolation::Linear),
                key(2.0, 10.0, SlintInterpolation::Linear),
            ],
        );
        st.push_channel(
            "b",
            Color::from_rgb_u8(0, 255, 0),
            vec![
                key(1.0, 5.0, SlintInterpolation::Linear),
                key(3.0, 15.0, SlintInterpolation::Linear),
            ],
        );
        st.rebuild_all();

        // 跨通道选中 a0、a1、b0，拖 a0：三个都移动，未选中的 b1 不动
        st.keyframe_pressed(0, 0, false);
        st.keyframe_pressed(0, 1, true);
        st.keyframe_pressed(1, 0, true);
        st.keyframe_dragged(0, 0, 0.5, 0.0, 0);

        assert_eq!(st.keyframe_models[&0].row_data(0).unwrap().time, 0.5);
        assert_eq!(st.keyframe_models[&0].row_data(1).unwrap().time, 2.5);
        assert_eq!(st.keyframe_models[&1].row_data(0).unwrap().time, 1.5);
        assert_eq!(st.keyframe_models[&1].row_data(1).unwrap().time, 3.0, "未选中的关键帧不应移动");
        // 值不变（dope 拖拽 dv=0 路径）
        assert_eq!(st.keyframe_models[&1].row_data(0).unwrap().value, 5.0);
    }

    #[test]
    fn keyframe_drag_skips_locked_channels() {
        let mut st = CurveEditorState::default();
        st.push_channel(
            "a",
            Color::from_rgb_u8(255, 0, 0),
            vec![key(0.0, 0.0, SlintInterpolation::Linear)],
        );
        st.push_channel(
            "b",
            Color::from_rgb_u8(0, 255, 0),
            vec![key(1.0, 5.0, SlintInterpolation::Linear)],
        );
        st.rebuild_all();

        // b 上锁：它的选中关键帧不随 a 的拖拽移动
        st.keyframe_pressed(0, 0, false);
        let mut b = st.channels.row_data(1).unwrap();
        b.editable = false;
        st.channels.set_row_data(1, b);
        // 直接给 b0 打上选中（跨通道模拟）
        let mut kf = st.keyframe_models[&1].row_data(0).unwrap();
        kf.selected = true;
        st.keyframe_models[&1].set_row_data(0, kf);

        st.keyframe_dragged(0, 0, 1.0, 0.0, 0);
        assert_eq!(st.keyframe_models[&0].row_data(0).unwrap().time, 1.0);
        assert_eq!(st.keyframe_models[&1].row_data(0).unwrap().time, 1.0, "锁定通道的关键帧不应移动");
    }

    #[test]
    fn keyframe_press_on_selected_keeps_multi_selection() {
        let mut st = CurveEditorState::default();
        st.push_channel(
            "a",
            Color::from_rgb_u8(255, 0, 0),
            vec![
                key(0.0, 0.0, SlintInterpolation::Linear),
                key(2.0, 10.0, SlintInterpolation::Linear),
            ],
        );
        st.rebuild_all();

        // 多选 a0、a1，然后普通点击已选中的 a0 再拖：多选保持，两个一起移动
        st.keyframe_pressed(0, 0, false);
        st.keyframe_pressed(0, 1, true);
        st.keyframe_released(0, 1, true);
        assert!(st.keyframe_models[&0].row_data(0).unwrap().selected);
        assert!(st.keyframe_models[&0].row_data(1).unwrap().selected);

        st.keyframe_pressed(0, 0, false); // 点已选中的 a0，不应清空选择
        assert!(st.keyframe_models[&0].row_data(1).unwrap().selected, "点已选中的关键帧不应清空多选");
        st.keyframe_dragged(0, 0, 0.5, 0.0, 0);
        assert_eq!(st.keyframe_models[&0].row_data(0).unwrap().time, 0.5);
        assert_eq!(st.keyframe_models[&0].row_data(1).unwrap().time, 2.5, "多选中被拖的关键帧外的其它选中关键帧也应移动");
    }

    #[test]
    fn shift_press_adds_never_deselects_grabbed() {
        let mut st = CurveEditorState::default();
        st.push_channel(
            "a",
            Color::from_rgb_u8(255, 0, 0),
            vec![
                key(0.0, 0.0, SlintInterpolation::Linear),
                key(2.0, 10.0, SlintInterpolation::Linear),
            ],
        );
        st.rebuild_all();

        // Shift 连续追加 a0、a1；再 Shift 点 a0（已选中）→ 仍保持选中（不会切掉）
        st.keyframe_pressed(0, 0, true);
        st.keyframe_pressed(0, 1, true);
        st.keyframe_pressed(0, 0, true);
        assert!(st.keyframe_models[&0].row_data(0).unwrap().selected);
        assert!(st.keyframe_models[&0].row_data(1).unwrap().selected);

        // Shift 点过的被拖关键帧保持选中 → 拖拽不会反馈放大（其余选中关键帧按正常速度移动）
        st.keyframe_dragged(0, 0, 0.5, 0.0, 0);
        assert_eq!(st.keyframe_models[&0].row_data(0).unwrap().time, 0.5);
        assert_eq!(st.keyframe_models[&0].row_data(1).unwrap().time, 2.5);
    }

    #[test]
    fn graph_marquee_select_picks_keyframes_and_handles() {
        let mut st = CurveEditorState::default();
        let mut k0 = key(0.0, 0.0, SlintInterpolation::Bezier);
        k0.handle_out = SlintBezierHandle { time_offset: 1.0, value_offset: 5.0 };
        st.push_channel(
            "a",
            Color::from_rgb_u8(255, 0, 0),
            vec![
                k0,
                key(2.0, 10.0, SlintInterpolation::Linear),
                key(4.0, 20.0, SlintInterpolation::Bezier),
            ],
        );
        st.rebuild_all();

        // 框选时间 [0,1.5] × 值 [0,10]：A 关键帧中心 (0,0) 命中，A 出柄控制点 (1,5) 命中；
        // B(2,10) 与 C(4,20) 不在框内
        st.graph_marquee_select(0.0, 1.5, 0.0, 10.0, 0);
        assert!(st.keyframe_models[&0].row_data(0).unwrap().selected);
        assert!(!st.keyframe_models[&0].row_data(1).unwrap().selected);
        assert!(!st.keyframe_models[&0].row_data(2).unwrap().selected);
        assert!(st.selected_handles.contains(&(0, 0, false)), "A 的出柄应被选中");
        assert!(!st.selected_handles.contains(&(0, 2, false)), "C 的出柄不应被选中");

        // selected 反映到 Slint 手柄模型
        let h = (0..st.handles.row_count())
            .filter_map(|i| st.handles.row_data(i))
            .find(|h| h.keyframe_index == 0 && !h.is_in)
            .expect("A 出柄应在模型中");
        assert!(h.selected);
    }

    #[test]
    fn graph_marquee_select_additive_keeps_existing() {
        let mut st = CurveEditorState::default();
        st.push_channel(
            "a",
            Color::from_rgb_u8(255, 0, 0),
            vec![
                key(0.0, 0.0, SlintInterpolation::Linear),
                key(2.0, 10.0, SlintInterpolation::Linear),
            ],
        );
        st.rebuild_all();

        st.graph_marquee_select(0.0, 0.5, -1.0, 1.0, 0); // 只选 a0
        st.graph_marquee_select(2.0, 2.5, 9.0, 11.0, 1); // Shift 追加 a1
        assert!(st.keyframe_models[&0].row_data(0).unwrap().selected);
        assert!(st.keyframe_models[&0].row_data(1).unwrap().selected);
        assert_eq!(st.selected_channel_count(), 1);
    }

    #[test]
    fn multi_handle_drag_same_screen_offset() {
        let mut st = CurveEditorState::default();
        // 两个贝塞尔关键帧，各自有出柄
        let mut k0 = key(0.0, 0.0, SlintInterpolation::Bezier);
        k0.handle_out = SlintBezierHandle { time_offset: 1.0, value_offset: 0.0 }; // (1,0)
        let mut k1 = key(2.0, 0.0, SlintInterpolation::Bezier);
        k1.handle_out = SlintBezierHandle { time_offset: 1.0, value_offset: 1.0 }; // (1,1)
        st.push_channel("a", Color::from_rgb_u8(255, 0, 0), vec![k0, k1]);
        st.rebuild_all();

        st.handle_pressed(0, 0, false, false);
        st.handle_pressed(0, 1, false, true);
        assert!(st.selected_handles.contains(&(0, 0, false)) && st.selected_handles.contains(&(0, 1, false)));

        // 拖 A 出柄 +1s,+1 值：A (1,0)→(2,1)，B 相同屏幕偏移 (1,1)→(2,2)
        st.handle_dragged(0, 0, false, 1.0, 1.0, 0);
        let kf0 = st.keyframe_models[&0].row_data(0).unwrap();
        assert!((kf0.handle_out.time_offset - 2.0).abs() < 1e-3);
        assert!((kf0.handle_out.value_offset - 1.0).abs() < 1e-3);
        let kf1 = st.keyframe_models[&0].row_data(1).unwrap();
        assert!(
            (kf1.handle_out.time_offset - 2.0).abs() < 1e-3
                && (kf1.handle_out.value_offset - 2.0).abs() < 1e-3,
            "B 出柄应保持相同屏幕偏移，got ({}, {}) expected (2, 2)",
            kf1.handle_out.time_offset,
            kf1.handle_out.value_offset
        );
    }

    #[test]
    fn multi_handle_drag_clamps_and_previews_synced_handles() {
        let mut st = CurveEditorState::default();
        let mut k0 = key(0.0, 0.0, SlintInterpolation::Bezier);
        k0.handle_out = SlintBezierHandle { time_offset: 1.0, value_offset: 0.0 };
        let mut k1 = key(2.0, 0.0, SlintInterpolation::Bezier);
        k1.handle_out = SlintBezierHandle { time_offset: -1.0, value_offset: 1.0 }; // 出柄时间偏移为负（会被硬钳到 ≥0）
        st.push_channel("a", Color::from_rgb_u8(255, 0, 0), vec![k0, k1]);
        st.rebuild_all();

        st.handle_pressed(0, 0, false, false);
        st.handle_pressed(0, 1, false, true);
        // 拖 A 向右 +2s：A (1,0)→(3,0)。B 同偏移 (-1,1)→(1,1)，再被硬钳制到 (≥0)：已 ≥0
        // 若把 B 拖向负方向（dt<0）则应被钳到 0
        st.handle_dragged(0, 0, false, 2.0, 0.0, 0);
        st.handle_dragged(0, 0, false, -4.0, 0.0, 0); // 累计 A：(3,0)→(-1,0)→硬钳到 (0,0)
        let kf0 = st.keyframe_models[&0].row_data(0).unwrap();
        assert!(kf0.handle_out.time_offset >= 0.0, "主手柄出柄应硬钳到 ≥0，got {}", kf0.handle_out.time_offset);
        let kf1 = st.keyframe_models[&0].row_data(1).unwrap();
        // B 跟随 A：(-1,1)→(1,1)→(-3,1)→硬钳到 (0,1)
        assert!(
            kf1.handle_out.time_offset >= 0.0,
            "非主手柄出柄也应硬钳到 ≥0，got {}",
            kf1.handle_out.time_offset
        );
        assert!((kf1.handle_out.value_offset - 1.0).abs() < 1e-3);
    }

    #[test]
    fn click_on_selected_keyframe_collapses_to_single() {
        let mut st = CurveEditorState::default();
        st.push_channel(
            "a",
            Color::from_rgb_u8(255, 0, 0),
            vec![
                key(0.0, 0.0, SlintInterpolation::Linear),
                key(2.0, 10.0, SlintInterpolation::Linear),
            ],
        );
        st.rebuild_all();

        // 多选 a0、a1（拖拽结束 moved=true 保持多选）
        st.keyframe_pressed(0, 0, false);
        st.keyframe_pressed(0, 1, true);
        st.keyframe_released(0, 1, true);
        assert!(st.keyframe_models[&0].row_data(0).unwrap().selected);
        assert!(st.keyframe_models[&0].row_data(1).unwrap().selected);

        // 单击 a0（未拖动 moved=false）→ 折叠为单选
        st.keyframe_pressed(0, 0, false);
        st.keyframe_released(0, 0, false);
        assert!(st.keyframe_models[&0].row_data(0).unwrap().selected);
        assert!(!st.keyframe_models[&0].row_data(1).unwrap().selected, "单击已选中的关键帧应折叠为单选");
    }

    #[test]
    fn click_on_selected_handle_collapses_to_single() {
        let mut st = CurveEditorState::default();
        let mut k0 = key(0.0, 0.0, SlintInterpolation::Bezier);
        k0.handle_out = SlintBezierHandle { time_offset: 1.0, value_offset: 0.0 };
        let mut k1 = key(2.0, 10.0, SlintInterpolation::Bezier);
        k1.handle_out = SlintBezierHandle { time_offset: 1.0, value_offset: 1.0 };
        st.push_channel("a", Color::from_rgb_u8(255, 0, 0), vec![k0, k1]);
        st.rebuild_all();

        st.handle_pressed(0, 0, false, false);
        st.handle_pressed(0, 1, false, true);
        assert_eq!(st.selected_handles.len(), 2);

        // 单击 a0 出柄（未拖动）→ 折叠为单选该手柄
        st.handle_pressed(0, 0, false, false);
        st.handle_released(0, 0, false, false);
        assert_eq!(st.selected_handles.len(), 1);
        assert!(st.selected_handles.contains(&(0, 0, false)));
        assert!(!st.selected_handles.contains(&(0, 1, false)));
    }

    #[test]
    fn shift_drag_pulls_out_handle_on_bezier_keyframe() {
        let mut st = CurveEditorState::default();
        st.push_channel(
            "a",
            Color::from_rgb_u8(255, 0, 0),
            vec![
                key(0.0, 0.0, SlintInterpolation::Bezier),
                key(2.0, 10.0, SlintInterpolation::Linear),
            ],
        );
        st.rebuild_all();

        // Shift+拖动 a0：关键帧元素不动，上报的 dt/dv 为光标相对关键帧的累计偏移，
        // 直接作为手柄偏移（不再累加），且出柄/入柄原点对称
        st.keyframe_pressed(0, 0, false);
        st.keyframe_dragged(0, 0, 1.0, 2.0, 1);
        st.keyframe_dragged(0, 0, 2.0, 3.0, 1); // 累计光标位置，直接覆盖
        let kf = st.keyframe_models[&0].row_data(0).unwrap();
        assert_eq!(kf.handle_out.time_offset, 2.0);
        assert_eq!(kf.handle_out.value_offset, 3.0);
        assert_eq!(kf.handle_in.time_offset, -2.0, "入柄应与出柄原点对称");
        assert_eq!(kf.handle_in.value_offset, -3.0);
        assert_eq!(kf.time, 0.0, "关键帧本体不应移动");
        assert_eq!(kf.value, 0.0);

        // 非贝塞尔关键帧不拖出手柄
        st.keyframe_pressed(0, 1, false);
        st.keyframe_dragged(0, 1, 1.0, 1.0, 1);
        let kf1 = st.keyframe_models[&0].row_data(1).unwrap();
        assert_eq!(kf1.handle_out.time_offset, 0.0);
        assert_eq!(kf1.handle_in.time_offset, 0.0);
    }

    #[test]
    fn multi_handle_drag_applies_mode_to_synced_handles() {
        let mut st = CurveEditorState::default();
        let mut k0 = key(0.0, 0.0, SlintInterpolation::Bezier);
        k0.handle_out = SlintBezierHandle { time_offset: 1.0, value_offset: 0.0 };
        k0.handle_in = SlintBezierHandle { time_offset: -1.0, value_offset: 0.0 };
        let mut k1 = key(2.0, 0.0, SlintInterpolation::Bezier);
        k1.handle_out = SlintBezierHandle { time_offset: 1.0, value_offset: 0.0 };
        k1.handle_in = SlintBezierHandle { time_offset: -1.0, value_offset: 0.0 };
        st.push_channel("a", Color::from_rgb_u8(255, 0, 0), vec![k0, k1]);
        st.rebuild_all();

        // 多选两个出柄，用 mode 2（原点对称）拖 A 出柄 +1,+1
        st.handle_pressed(0, 0, false, false);
        st.handle_pressed(0, 1, false, true);
        st.handle_dragged(0, 0, false, 1.0, 1.0, 2);

        // 主手柄 A 出柄 (1,0)→(2,1)，其入柄原点对称 = -(2,1)
        let kf0 = st.keyframe_models[&0].row_data(0).unwrap();
        assert!((kf0.handle_out.time_offset - 2.0).abs() < 1e-3);
        assert!((kf0.handle_in.time_offset + 2.0).abs() < 1e-3, "主手柄入柄应原点对称");
        assert!((kf0.handle_in.value_offset + 1.0).abs() < 1e-3);

        // 被同步的 B 出柄同样 +1,+1 → (2,1)，其入柄同样原点对称 = -(2,1)（模式一致）
        let kf1 = st.keyframe_models[&0].row_data(1).unwrap();
        assert!((kf1.handle_out.time_offset - 2.0).abs() < 1e-3);
        assert!((kf1.handle_in.time_offset + 2.0).abs() < 1e-3, "被同步手柄的对手柄也应原点对称（模式一致）");
        assert!((kf1.handle_in.value_offset + 1.0).abs() < 1e-3);
    }

    #[test]
    fn click_cycles_through_coincident_keyframes() {
        let mut st = CurveEditorState::default();
        // 三个通道在相同位置 (1, 5) 有几乎重合的关键帧
        st.push_channel("a", Color::from_rgb_u8(255, 0, 0), vec![key(1.0, 5.0, SlintInterpolation::Linear)]);
        st.push_channel("b", Color::from_rgb_u8(0, 255, 0), vec![key(1.0, 5.0, SlintInterpolation::Linear)]);
        st.push_channel("c", Color::from_rgb_u8(0, 0, 255), vec![key(1.0, 5.0, SlintInterpolation::Linear)]);
        st.rebuild_all();

        // 第一次点击 a0（未选中）→ 单选 a0
        st.keyframe_pressed(0, 0, false);
        st.keyframe_released(0, 0, false);
        assert!(st.keyframe_models[&0].row_data(0).unwrap().selected);
        assert!(!st.keyframe_models[&1].row_data(0).unwrap().selected);

        // 第二次点击该位置（已有选中的重合点）→ 循环到 b0
        st.keyframe_pressed(0, 0, false);
        assert!(st.pressed_was_selected, "该位置已有选中的重合点");
        st.keyframe_released(0, 0, false);
        assert!(!st.keyframe_models[&0].row_data(0).unwrap().selected);
        assert!(st.keyframe_models[&1].row_data(0).unwrap().selected);

        // 第三次 → c0
        st.keyframe_pressed(0, 0, false);
        st.keyframe_released(0, 0, false);
        assert!(st.keyframe_models[&2].row_data(0).unwrap().selected);

        // 第四次 → 回到 a0
        st.keyframe_pressed(0, 0, false);
        st.keyframe_released(0, 0, false);
        assert!(st.keyframe_models[&0].row_data(0).unwrap().selected);
    }

    #[test]
    fn channel_search_filters_per_editor_independently() {
        let mut st = CurveEditorState::default();
        st.push_channel("Position X", Color::from_rgb_u8(255, 0, 0), vec![key(0.0, 0.0, SlintInterpolation::Linear)]);
        st.push_channel("Rotation Y", Color::from_rgb_u8(0, 255, 0), vec![key(0.0, 0.0, SlintInterpolation::Linear)]);
        st.rebuild_all();
        assert_eq!(st.graph_filter.row_count(), 2);
        assert_eq!(st.dope_filter.row_count(), 2);

        // 曲线编辑器搜索只影响自己的过滤视图
        st.set_graph_search("pos"); // 忽略大小写子串匹配
        assert_eq!(st.graph_filter.row_count(), 1, "曲线编辑器过滤只影响自己");
        assert_eq!(st.dope_filter.row_count(), 2, "摄影表不受曲线编辑器搜索影响");
        assert_eq!(st.graph_filter.row_data(0).unwrap().curve_index, 0, "过滤视图元素保留真实索引");
        assert_eq!(st.graph_filter.row_data(0).unwrap().name.as_str(), "Position X");

        // 摄影表搜索独立
        st.set_dope_search("rot");
        assert_eq!(st.graph_filter.row_count(), 1);
        assert_eq!(st.dope_filter.row_count(), 1);
        assert_eq!(st.dope_filter.row_data(0).unwrap().curve_index, 1);

        // 清空恢复全部
        st.set_graph_search("");
        st.set_dope_search("");
        assert_eq!(st.graph_filter.row_count(), 2);
        assert_eq!(st.dope_filter.row_count(), 2);
    }

    #[test]
    fn graph_segments_and_handles_filtered_by_search() {
        let mut st = CurveEditorState::default();
        let mut k0 = key(0.0, 0.0, SlintInterpolation::Bezier);
        k0.handle_out = SlintBezierHandle { time_offset: 1.0, value_offset: 1.0 };
        st.push_channel(
            "Position X",
            Color::from_rgb_u8(255, 0, 0),
            vec![k0, key(2.0, 5.0, SlintInterpolation::Linear)],
        );
        st.push_channel(
            "Rotation Y",
            Color::from_rgb_u8(0, 255, 0),
            vec![key(0.0, 0.0, SlintInterpolation::Linear), key(2.0, 5.0, SlintInterpolation::Linear)],
        );
        st.rebuild_all();
        // 两个通道都有段；只有 Position X 有贝塞尔出柄
        assert_eq!(st.segments.row_count(), 2);
        assert_eq!(st.graph_segments.row_count(), 2);

        st.set_graph_search("pos");
        // 段过滤到只含 Position X；手柄 = Position X 的贝塞尔段两端各一个（出柄+入柄）
        let segs: Vec<i32> = (0..st.graph_segments.row_count())
            .filter_map(|i| st.graph_segments.row_data(i).map(|s| s.curve_index))
            .collect();
        let hdls: Vec<i32> = (0..st.graph_handles.row_count())
            .filter_map(|i| st.graph_handles.row_data(i).map(|h| h.curve_index))
            .collect();
        assert_eq!(segs, vec![0]);
        assert_eq!(hdls, vec![0, 0], "手柄都应来自 Position X");
        assert_eq!(st.graph_handles.row_count(), 2);

        st.set_graph_search("");
        assert_eq!(st.graph_segments.row_count(), 2);
        assert_eq!(st.graph_handles.row_count(), 2);
    }

    #[test]
    fn dope_connections_cover_linear_and_hold_segments() {
        let mut st = CurveEditorState::default();
        st.push_channel(
            "a",
            Color::from_rgb_u8(255, 0, 0),
            vec![
                key(0.0, 0.0, SlintInterpolation::Linear),
                key(2.0, 10.0, SlintInterpolation::Hold),
                key(4.0, 10.0, SlintInterpolation::Bezier),
            ],
        );
        st.rebuild_all();
        // 段由左侧关键帧插值决定：[0,2] 线性、[2,4] 常量；最后一个贝塞尔无右侧段
        assert_eq!(st.dope_connections.row_count(), 2);
        let c0 = st.dope_connections.row_data(0).unwrap();
        assert_eq!((c0.x0, c0.x1), (0.0, 2.0));
        assert!(!c0.is_hold, "线性段不标记 hold");
        assert!(c0.visible);
        let c1 = st.dope_connections.row_data(1).unwrap();
        assert_eq!((c1.x0, c1.x1), (2.0, 4.0));
        assert!(c1.is_hold, "常量段应标记 hold");
    }

    #[test]
    fn dope_connections_skip_bezier_and_follow_visibility() {
        let mut st = CurveEditorState::default();
        st.push_channel(
            "a",
            Color::from_rgb_u8(255, 0, 0),
            vec![
                key(0.0, 0.0, SlintInterpolation::Bezier),
                key(2.0, 10.0, SlintInterpolation::Linear),
                key(4.0, 10.0, SlintInterpolation::Linear),
            ],
        );
        st.rebuild_all();
        // [0,2] 贝塞尔（左侧关键帧 Bezier）→ 无连接；[2,4] 线性 → 连接
        assert_eq!(st.dope_connections.row_count(), 1);
        let c0 = st.dope_connections.row_data(0).unwrap();
        assert_eq!((c0.x0, c0.x1), (2.0, 4.0));

        // 关掉眼睛 → 连接 visible=false
        let mut ch = st.channels.row_data(0).unwrap();
        ch.visible = false;
        st.channels.set_row_data(0, ch);
        st.toggle_channel_visible(0); // 重新触发重建（先恢复再关掉 → 最终 false）
        st.toggle_channel_visible(0);
        assert!(!st.dope_connections.row_data(0).unwrap().visible, "眼睛关闭后连接应隐藏");
    }

    #[test]
    fn keyframe_times_snap_to_integer_ms() {
        let mut st = CurveEditorState::default();
        st.push_channel(
            "a",
            Color::from_rgb_u8(255, 0, 0),
            vec![
                key(0.1234, 1.0, SlintInterpolation::Linear),
                key(2.9996, 2.0, SlintInterpolation::Linear),
            ],
        );
        st.rebuild_all();
        let m = st.keyframe_models.get(&0).unwrap();
        let k0 = m.row_data(0).unwrap();
        let k1 = m.row_data(1).unwrap();
        // 0.1234 → 123ms → 0.123；2.9996 → 3000ms → 3.0
        assert!((k0.time - 0.123).abs() < 1e-6, "应吸附到 123ms，得 {}", k0.time);
        assert!((k1.time - 3.0).abs() < 1e-6, "应吸附到 3000ms，得 {}", k1.time);
    }

    #[test]
    fn summary_aggregates_coincident_keyframes_by_ms() {
        let mut st = CurveEditorState::default();
        st.push_channel(
            "a",
            Color::from_rgb_u8(255, 0, 0),
            vec![
                key(0.0, 1.0, SlintInterpolation::Linear),
                key(2.0, 2.0, SlintInterpolation::Linear),
            ],
        );
        st.push_channel(
            "b",
            Color::from_rgb_u8(0, 255, 0),
            vec![
                key(0.0, 3.0, SlintInterpolation::Linear), // 与 a 同时刻
                key(2.5, 4.0, SlintInterpolation::Linear),
            ],
        );
        st.rebuild_all();
        // 时间 0ms（两条）、2000ms（一条）、2500ms（一条）
        let mut times: Vec<f32> = (0..st.summary_keyframes.row_count())
            .map(|i| st.summary_keyframes.row_data(i).unwrap().time)
            .collect();
        times.sort_by(|a, b| a.partial_cmp(b).unwrap());
        assert_eq!(times.len(), 3);
        assert_eq!(st.summary_keyframes.row_data(0).unwrap().count, 2, "同一毫秒应合并计数");
        assert!((times[0] - 0.0).abs() < 1e-6);
        assert!((times[1] - 2.0).abs() < 1e-6);
        assert!((times[2] - 2.5).abs() < 1e-6);
    }

    #[test]
    fn reset_action_clears_current_action() {
        let mut st = CurveEditorState::default();
        st.push_channel(
            "a",
            Color::from_rgb_u8(255, 0, 0),
            vec![
                key(0.0, 1.0, SlintInterpolation::Linear),
                key(2.0, 3.0, SlintInterpolation::Linear),
            ],
        );
        st.rebuild_all();
        assert!(st.action_start < st.action_end, "应有非零时长范围");
        assert_eq!(st.summary_keyframes.row_count(), 2);

        // 下插到 NLA 后重置全局 Action：通道/关键帧清空、名称清空、时长归零
        st.reset_action();
        assert_eq!(st.channels.row_count(), 0);
        assert_eq!(st.summary_keyframes.row_count(), 0);
        assert_eq!((st.action_start, st.action_end), (0.0, 0.0));
        assert!(st.action_name.is_empty(), "重置后名称清空，UI 显示<无动作>");
        assert_eq!(st.current_action, -1);
    }

    #[test]
    fn summary_selected_reflects_any_underlying() {
        let mut st = CurveEditorState::default();
        st.push_channel(
            "a",
            Color::from_rgb_u8(255, 0, 0),
            vec![key(0.0, 1.0, SlintInterpolation::Linear)],
        );
        st.push_channel(
            "b",
            Color::from_rgb_u8(0, 255, 0),
            vec![key(0.0, 3.0, SlintInterpolation::Linear)],
        );
        st.rebuild_all();
        // 初始无选中（逐通道掩码全 false）
        assert!(!st.summary_keyframes.row_data(0).unwrap().selected.iter().any(|s| s));

        // 普通点击汇总关键帧：按下立即高亮（掩码全 true），选中该时刻全部（两通道）
        st.summary_keyframe_pressed(0, false);
        assert!(
            st.summary_keyframes.row_data(0).unwrap().selected.iter().all(|s| s),
            "按下应立即高亮汇总标记，不用等松开"
        );
        assert!(st.channels.row_data(0).unwrap().selected, "按下应立即亮起关联轨道");
        assert!(st.channels.row_data(1).unwrap().selected, "按下应立即亮起全部关联轨道");
        st.summary_keyframe_released(false);
        assert!(st.keyframe_models.get(&0).unwrap().row_data(0).unwrap().selected);
        assert!(st.keyframe_models.get(&1).unwrap().row_data(0).unwrap().selected);
        assert!(
            st.summary_keyframes.row_data(0).unwrap().selected.iter().all(|s| s),
            "松开重建后仍全部高亮"
        );

        // 拖拽：两通道同时刻的关键帧一起移动
        st.summary_keyframe_dragged(0, 1.0);
        let k0 = st.keyframe_models.get(&0).unwrap().row_data(0).unwrap();
        let k1 = st.keyframe_models.get(&1).unwrap().row_data(0).unwrap();
        assert!((k0.time - 1.0).abs() < 1e-6);
        assert!((k1.time - 1.0).abs() < 1e-6);
    }

    #[test]
    fn summary_drag_snaps_to_integer_ms() {
        let mut st = CurveEditorState::default();
        st.push_channel(
            "a",
            Color::from_rgb_u8(255, 0, 0),
            vec![key(0.0, 1.0, SlintInterpolation::Linear)],
        );
        st.rebuild_all();
        st.summary_keyframe_pressed(0, false);
        // 拖 1.2345 秒 → 应吸附到 1235ms
        st.summary_keyframe_dragged(0, 1.2345);
        let k0 = st.keyframe_models.get(&0).unwrap().row_data(0).unwrap();
        assert!((k0.time - 1.235).abs() < 1e-6, "拖拽应吸附整数毫秒，得 {}", k0.time);
    }

    #[test]
    fn summary_drag_moves_whole_group_even_if_partially_selected() {
        // 回归：先单独选中某个通道的关键帧，再去拖它上面的汇总帧——
        // 应移动该时刻的全部关键帧（而不是只移动已选中的那个）。
        let mut st = CurveEditorState::default();
        st.push_channel(
            "a",
            Color::from_rgb_u8(255, 0, 0),
            vec![
                key(0.0, 1.0, SlintInterpolation::Linear),
                key(4.0, 2.0, SlintInterpolation::Linear),
            ],
        );
        st.push_channel(
            "b",
            Color::from_rgb_u8(0, 255, 0),
            vec![key(0.0, 3.0, SlintInterpolation::Linear)],
        );
        st.rebuild_all();
        // 先只选中通道 a 的 t=0 关键帧（模拟用户先点它）
        st.keyframe_pressed(0, 0, false);
        st.keyframe_released(0, 0, false);
        assert!(st.keyframe_models.get(&0).unwrap().row_data(0).unwrap().selected);
        assert!(!st.keyframe_models.get(&1).unwrap().row_data(0).unwrap().selected);

        // 按下汇总帧（t=0）并拖 1 秒：应跨通道移动 a、b 全部同时刻关键帧
        st.summary_keyframe_pressed(0, false);
        st.summary_keyframe_dragged(0, 1.0);
        st.summary_keyframe_released(false);
        let a0 = st.keyframe_models.get(&0).unwrap().row_data(0).unwrap();
        let b0 = st.keyframe_models.get(&1).unwrap().row_data(0).unwrap();
        assert!((a0.time - 1.0).abs() < 1e-6, "a 通道 t=0 关键帧应随汇总帧移到 1s，得 {}", a0.time);
        assert!((b0.time - 1.0).abs() < 1e-6, "b 通道同时刻关键帧也应一起移动，得 {}", b0.time);
    }

    #[test]
    fn marquee_selects_channel_when_box_covers_curve() {
        // 对齐 Blender：框选矩形覆盖到曲线本身（即使没有关键帧在框内）也应选中该通道。
        let mut st = CurveEditorState::default();
        st.push_channel(
            "c",
            Color::from_rgb_u8(255, 0, 0),
            vec![
                key(0.0, 0.0, SlintInterpolation::Linear),
                key(4.0, 0.0, SlintInterpolation::Linear),
            ],
        );
        st.rebuild_all();

        // 框住曲线中段（x∈[1,3], y∈[-1,1]），关键帧在框外 → 仅曲线命中，通道被选中
        st.graph_marquee_select(1.0, 3.0, -1.0, 1.0, 0);
        assert!(
            st.channels.row_data(0).unwrap().selected,
            "框住曲线本身应选中该通道"
        );
    }

    #[test]
    fn marquee_selects_channel_when_curve_bends_into_box() {
        // 贝塞尔曲线弯曲进框内（框内无关键帧）也应选中通道。
        let mut st = CurveEditorState::default();
        // 曲线 (0,0)→(4,0) 凸到 y≈1.73；框取 y∈[1,2] 只罩住弯起部分
        let mut k0 = key(0.0, 0.0, SlintInterpolation::Bezier);
        k0.handle_out = SlintBezierHandle { time_offset: 2.0, value_offset: 6.0 };
        let mut k1 = key(4.0, 0.0, SlintInterpolation::Bezier);
        k1.handle_in = SlintBezierHandle { time_offset: -2.0, value_offset: -6.0 };
        st.push_channel("c", Color::from_rgb_u8(255, 0, 0), vec![k0, k1]);
        st.rebuild_all();

        // 关键帧在 (0,0)/(4,0)，都远离框；曲线在 x∈[1.0,1.3] 处 y≈1.73 落入框内
        st.graph_marquee_select(1.0, 1.5, 1.5, 2.0, 0);
        assert!(
            st.channels.row_data(0).unwrap().selected,
            "框住曲线弯起的部分应选中该通道"
        );
    }

    #[test]
    fn marquee_ignores_box_above_below_curve() {
        // 框选矩形完全在曲线上方/下方（没有罩住曲线）→ 不选中该通道
        let mut st = CurveEditorState::default();
        st.push_channel(
            "c",
            Color::from_rgb_u8(255, 0, 0),
            vec![
                key(0.0, 0.0, SlintInterpolation::Linear),
                key(4.0, 0.0, SlintInterpolation::Linear),
            ],
        );
        st.rebuild_all();

        // 曲线上方大框（y∈[5,10]，曲线 y=0 完全在下方）→ 不选中
        st.graph_marquee_select(0.0, 4.0, 5.0, 10.0, 0);
        assert!(
            !st.channels.row_data(0).unwrap().selected,
            "框在曲线上方不应选中通道"
        );
    }
}
