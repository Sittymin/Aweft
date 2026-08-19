//! 曲线编辑器（Graph Editor）Rust 侧：持有 FCurve 通道视图模型与全部编辑操作。
//!
//! # main 架构接线（依赖图结构）
//!
//! 交互统一走 main 标准管线：`AnimationCallbacks` → `AppStore::dispatch(AnimationAction)`
//! → `dispatch_animation`（reducer，只改数据层并返回精确 tag）→ depsgraph 单轮求值
//! → 投影 `op_animation_*`（把数据层同步到派生层 Slint 模型 + 标量）。
//!
//! # 权威数据在 model（已接线）
//!
//! 曲线编辑器编辑的 Action 权威存储在 `Project.library.animation_library.actions`
//! （`aweft-model::animation::Action`）。本 store 只保存 UI 数据并生成 Slint 的
//! 通道/关键帧/段/手柄视图模型（投影），不再自持权威数据。
//!
//! 映射关系（通道按下标 1:1）：
//!  - 视图通道 `curve` ↔ model `Action.curves[curve]`（`ActionCurve { property, curve: FCurve }`）
//!  - 视图关键帧下标 ↔ model `FCurve.keyframes` 下标（装载时按时间排序，编辑时保序）
//!  - `SlintKeyframe` ↔ `Keyframe`（`BezierHandle`/`Interpolation`），f32 ↔ f64
//!
//! 数据流：
//!  - **装载**：`dispatch_animation(ActionSelected)` 读 model Action → `load_action`（全量重建视图）
//!  - **拖拽（高频）**：`KeyframeDragged`/`HandleDragged` 只改视图（局部 `set_row_data`，
//!    保住 Slint 拖拽抓取）；对齐 main 的 preview/commit 模式（同 LineEditor 拖拽）
//!  - **提交**：`KeyframeReleased`/`HandleReleased`/插值变更后 `commit_to_model` 把视图
//!    关键帧写回 model Action（按时间排序，丢 UI 选中态）
//!  - UI 态（选中/搜索/视图缩放/通道可见性/颜色）留在本 store，不入 model
//!
//! # 派生数据（投影）职责划分
//!
//! 本模块的状态分两层：
//!  - **数据层**（reducer 可写）：`channels_data`（通道属性）、`editing_keyframes`
//!    （关键帧数据）、选中集与 UI 态（selected_handles/搜索/缩放/锚点/pending 选择……）
//!  - **派生层**（只由投影写）：`channels`/`keyframe_models`/`extrapolations`（Slint 绑定模型）、
//!    `segments`/`handles`/`dope_connections`/`summary_keyframes`/
//!    `selected_display`/`preview_handles`/`action_start`/`action_end` 与 Slint 标量
//!
//! 交互方法（reducer）只改数据层并返回 `Vec<Tag>`；`dispatch_animation` 打 tag 后由
//! depsgraph 投影（`op_animation_*`）把数据层同步到派生层——对齐 LineEditor 的
//! `Action → tag → 投影` 标准管线。

use std::collections::{HashMap, HashSet};
use std::rc::Rc;

use aweft_model::{animation::Action as ModelAction, ids::ActionId};
use slint::{ModelRc, SharedString, VecModel};

use crate::bridge::types::{
    ContextMenuItem, SlintCurveSegment, SlintDopeConnection, SlintExtrapolation,
    SlintFCurveChannel, SlintHandleMarker, SlintKeyframe, SlintSummaryKeyframe,
};
use crate::depsgraph::HandleLoc;

mod channel;
mod connections;
mod curve_math;
mod filters;
mod handle;
mod interpolation;
mod keyframe;
mod load;
mod marquee;
mod preview;
mod projection;
mod rebuild;
mod summary;
#[cfg(test)]
mod tests;

/// 时间坐标最小单位为毫秒：把时间对齐到最近的整数毫秒（关键帧位置要求）。
fn snap_ms(t: f32) -> f32 {
    (t * 1000.0).round() / 1000.0
}

/// 时间 → 整数毫秒（用于"同一时间"判断：整数毫秒相等即同一时刻）。
pub(super) fn ms_of(t: f32) -> i64 {
    (t * 1000.0).round() as i64
}

/// 关键帧位置标识：(通道, 关键帧下标)。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct KeyframeLoc {
    pub curve: usize,
    pub key: usize,
}

/// 关键帧数据更新请求（拖拽时 reducer 收集、统一写回）。
#[derive(Debug, Clone)]
pub struct KeyframeUpdate {
    pub curve: usize,
    pub key: usize,
    pub keyframe: SlintKeyframe,
}

/// 关键帧时间更新请求（汇总拖拽时 reducer 收集、统一写回）。
#[derive(Debug, Clone, Copy)]
pub struct KeyframeTimeUpdate {
    pub curve: usize,
    pub key: usize,
    pub time: f32,
}

/// 一个 Action 的 UI 侧清单项（装载时从 model 实时读取曲线数据）。
#[derive(Clone, Debug)]
pub struct ActionData {
    /// Action 名（Header 下拉菜单显示）
    pub name: SharedString,
    /// model 中的 ActionId
    pub id: ActionId,
}

/// 曲线编辑器状态
pub struct GraphEditorState {
    /// 通道属性数据（编辑权威；`keyframes` 字段为占位，投影写入真实 inner 模型）。
    pub(crate) channels_data: Vec<SlintFCurveChannel>,
    /// 关键帧数据（编辑权威）：通道下标 → 关键帧列表（与 `keyframe_models` 同序）。
    pub(crate) editing_keyframes: HashMap<usize, Vec<SlintKeyframe>>,

    /// 通道视图模型（投影输出；绑定 Slint；`keyframes` 指向 `keyframe_models` 中同一 inner 模型）
    pub channels: Rc<VecModel<SlintFCurveChannel>>,
    /// 曲线编辑器通道列表的过滤压缩视图（投影输出；FilterModel，只含匹配搜索的通道；元素带真实 curve-index）
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
    /// 每通道按时间排序后的首/末关键帧（投影输出；外推线数据源——
    /// 关键帧模型拖拽期间保序，外推线必须读排序后首末，见 graph_editor.slint）
    pub extrapolations: Rc<VecModel<SlintExtrapolation>>,
    /// 当前时间（播放头）
    pub current_time: f64,
    /// 当前 Action 名称（header 栏中区显示）
    pub action_name: SharedString,
    /// Action 清单（Header 下拉菜单数据源；顺序即显示顺序，装载时从 model 实时读取曲线数据）
    pub actions: Vec<ActionData>,
    /// 当前选中的 Action 下标（-1 = 无）
    pub current_action: i32,
    /// Action 下拉菜单项模型（由 actions 清单名派生，色点透明）
    pub action_items: Rc<VecModel<ContextMenuItem>>,
    /// Action 时长范围（画布明暗分段用，由关键帧时间推导）
    pub action_start: f32,
    pub action_end: f32,
    /// 当前激活通道（点击曲线/关键帧/控制柄/通道列表行设置；-1 = 无激活；侧边栏据此显示哪条曲线）
    pub current_channel: isize,
    /// Shift 区间选择的锚点（最近一次普通/Shift 点击的通道；Ctrl 切换不更新）
    selection_anchor: isize,
    /// 关键帧 Shift 区间选择锚点（同一通道内按关键帧下标扩展）。
    keyframe_selection_anchor: Option<KeyframeLoc>,
    /// 控制柄 Shift 区间选择锚点（同一通道、同一侧扩展）。
    handle_selection_anchor: Option<HandleLoc>,
    /// 通道选中**显示**掩码（`[bool]`，下标 = 通道下标）：按下/选择即更新，
    /// 供曲线高亮读取。与 `channels` 模型的 `selected` 字段解耦——后者嵌套在
    /// 关键帧 repeater 内，拖拽中更新会重建元素破坏 grab（见 tests/channel_select_drag.rs），
    /// 故延迟到 released 写回；此掩码是独立模型，更新不触碰交互元素（"显示先动"）。
    pub selected_display: Rc<VecModel<bool>>,
    /// 关键帧按下时的"待应用通道选择"。
    /// 关键帧标记嵌套在 channels for 循环内，按下期间若更新外层 channels 模型会让 Slint
    /// 重刷 grab 路径上的行元素、丢失拖拽抓取（见 tests/dope_drag.rs 的复现）。
    /// 因此按下时只记录期望选择，松开（grab 已结束）后再写回 channels 模型。
    pub(crate) pending_channel_selection: Vec<usize>,
    /// 本次按下时，被按关键帧位置是否已存在选中（用于单击循环选中几乎重合的点）。
    pressed_was_selected: bool,
    /// 本次按下时的修饰键（用于释放时判断是否跳过折叠）
    pressed_modifier: crate::bridge::types::SlintSelectionModifier,
    /// 曲线编辑器 / 摄影表各自的搜索词（独立过滤）
    graph_search: String,
    dope_search: String,
    /// 曲线编辑器当前匹配搜索的通道集合（段/手柄过滤用）
    graph_matched: HashSet<usize>,
    /// 摄影表当前匹配搜索的通道集合（连接线增量更新用）
    dope_matched: HashSet<usize>,
    /// 手柄拖拽时相邻关键帧软钳制的预览：被拖柄的有效控制点（与原始位置不同）
    pub preview_handles: Rc<VecModel<SlintHandleMarker>>,
    /// 选中的贝塞尔控制柄（`HandleLoc`）。
    /// 与关键帧选中独立；框选可同时选中关键帧与手柄；多选拖拽时同侧手柄按相同角度增量旋转。
    pub selected_handles: HashSet<HandleLoc>,
    /// 当前视图缩放（px/单位）：命中检测容差随缩放变化（放大更精确）
    pub time_zoom: f32,
    pub value_zoom: f32,

    /// 关键帧的 Slint 嵌套模型（投影输出）：通道下标 → 稳定的 inner VecModel，
    /// 由投影从 `editing_keyframes` 同步（拖拽期间 set_row_data 保住 grab）。
    pub(crate) keyframe_models: HashMap<usize, Rc<VecModel<SlintKeyframe>>>,
    /// 曲线段在 segments 模型中的偏移：offset[k] = 第 k 个通道的段起始下标（长度 = 通道数+1）
    pub(crate) segment_offsets: Vec<usize>,
    /// 控制柄在 handles 模型中的偏移（同上）
    pub(crate) handle_offsets: Vec<usize>,
}

impl Default for GraphEditorState {
    fn default() -> Self {
        Self {
            channels_data: Vec::new(),
            editing_keyframes: HashMap::new(),
            channels: Rc::new(VecModel::default()),
            graph_filter: ModelRc::default(),
            dope_filter: ModelRc::default(),
            graph_segments: ModelRc::default(),
            graph_handles: ModelRc::default(),
            segments: Rc::new(VecModel::default()),
            handles: Rc::new(VecModel::default()),
            dope_connections: Rc::new(VecModel::default()),
            summary_keyframes: Rc::new(VecModel::default()),
            extrapolations: Rc::new(VecModel::default()),
            current_time: 0.0,
            action_name: SharedString::default(),
            actions: Vec::new(),
            current_action: -1,
            action_items: Rc::new(VecModel::default()),
            action_start: 0.0,
            action_end: 0.0,
            current_channel: -1,
            selection_anchor: -1,
            keyframe_selection_anchor: None,
            handle_selection_anchor: None,
            selected_display: Rc::new(VecModel::default()),
            pending_channel_selection: Vec::new(),
            pressed_was_selected: false,
            pressed_modifier: crate::bridge::types::SlintSelectionModifier::None,
            graph_search: String::new(),
            dope_search: String::new(),
            graph_matched: HashSet::new(),
            dope_matched: HashSet::new(),
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

// ── AppStore 入口 ─────────────────────────────────────────────────────────
impl crate::store::AppStore {
    /// 初始化曲线编辑器：把 Rust 持有的模型绑定到 Slint 的 AnimationState 全局。
    /// 一次性绑定（VecModel 共享同一 inner，后续变更自动通知 Slint）。
    pub fn initialize_animation_editors(&mut self, instance: &crate::bridge::types::MainWindow) {
        use slint::ComponentHandle;

        let state = instance.global::<crate::bridge::types::AnimationState>();
        state.set_channels(ModelRc::from(self.ui_models.graph_editor.channels.clone()));
        state.set_graph_channels(self.ui_models.graph_editor.graph_filter.clone());
        state.set_dope_channels(self.ui_models.graph_editor.dope_filter.clone());
        state.set_graph_segments(self.ui_models.graph_editor.graph_segments.clone());
        state.set_graph_handles(self.ui_models.graph_editor.graph_handles.clone());
        state.set_segments(ModelRc::from(self.ui_models.graph_editor.segments.clone()));
        state.set_handles(ModelRc::from(self.ui_models.graph_editor.handles.clone()));
        state.set_dope_connections(ModelRc::from(
            self.ui_models.graph_editor.dope_connections.clone(),
        ));
        state.set_dope_summary_keyframes(ModelRc::from(
            self.ui_models.graph_editor.summary_keyframes.clone(),
        ));
        state.set_extrapolations(ModelRc::from(
            self.ui_models.graph_editor.extrapolations.clone(),
        ));
        state.set_current_time(self.ui_models.graph_editor.current_time as f32);
        state.set_action_name(self.ui_models.graph_editor.action_name.clone());
        state.set_action_items(ModelRc::from(
            self.ui_models.graph_editor.action_items.clone(),
        ));
        state.set_current_action(self.ui_models.graph_editor.current_action);
        state.set_action_start(self.ui_models.graph_editor.action_start);
        state.set_action_end(self.ui_models.graph_editor.action_end);
        state.set_current_channel(self.ui_models.graph_editor.current_channel as i32);
        state.set_selected_channels(ModelRc::from(
            self.ui_models.graph_editor.selected_display.clone(),
        ));
        state.set_preview_handles(ModelRc::from(
            self.ui_models.graph_editor.preview_handles.clone(),
        ));
        state.set_nla_tracks(self.ui_models.nla_editor.tracks_filter.clone().into());
    }

    /// 新建一个**空的**编辑 Action 并装载（正确初始化路径）。
    ///
    /// 未来由用户在视图 Area / 属性 Area 中 K 帧构建 Action 数据；当前作为
    /// 启动时的默认编辑目标（无通道、无关键帧）。
    pub fn new_editing_action(&mut self, name: &str) {
        let action = ModelAction::new(name);
        let id = action.id;
        self.state
            .project
            .library
            .animation_library
            .insert_action(action);
        let ce = &mut self.ui_models.graph_editor;
        ce.actions.push(ActionData {
            name: name.into(),
            id,
        });
        ce.current_action = (ce.actions.len() - 1) as i32;
        let action = self
            .state
            .project
            .library
            .animation_library
            .get_action(id)
            .expect("刚插入的 Action 应存在");
        let tags = ce.load_action(ce.current_action as usize, action);
        // 不经 dispatch 的直接调用路径：手动跑投影（结构变化 → 派生层重建 + 过滤视图重绑定）
        self.ui_models.graph_editor.apply_tags(&tags);
        self.rebind_animation_filters();
        // NLA 全局 Action 轨同步亦走 depsgraph（初始化亦经投影，局部更新）
        let nla_tags = self.sync_nla_global_action();
        for tag in nla_tags {
            self.depsgraph.tag(tag);
        }
        self.evaluate_depsgraph();
    }

    /// HACK: 填充曲线编辑器测试数据（临时；当前被 `window_manager` 初始化调用）。
    ///
    /// 视图 Area 与属性 Area 尚未实现，编辑器内无法 K 帧构建 Action，因此目前
    /// 直接以测试数据填充（多个 Action 用于演示下拉切换/编辑交互），便于测试。
    /// 正确路径（未来）：初始化只调 `new_editing_action` 新建空 Action，由用户在
    /// 编辑器中 K 帧构建——届时应删除本函数与 window_manager 里的调用。
    pub fn create_test_animation_data(&mut self) {
        use aweft_model::animation::{
            Action as ModelAction, BezierHandle, Keyframe as Mk, PropertyPath, action::ActionCurve,
            fcurve::FCurve,
        };

        let bez = |time: f64, value: f64, in_to: f64, in_vo: f64, out_to: f64, out_vo: f64| {
            Mk::bezier(
                time,
                value,
                BezierHandle::new(in_to, in_vo),
                BezierHandle::new(out_to, out_vo),
            )
        };
        let lin = |time: f64, value: f64| Mk::linear(time, value);
        let hold = |time: f64, value: f64| Mk::hold(time, value);

        let mk_action = |name: &str, curves: Vec<(PropertyPath, Vec<Mk>)>| {
            let mut action = ModelAction::new(name);
            action.curves = curves
                .into_iter()
                .map(|(property, kfs)| {
                    let fcurve = FCurve {
                        keyframes: kfs,
                        ..Default::default()
                    };
                    ActionCurve::new(property, fcurve)
                })
                .collect();
            action
        };

        // 主 Action：四条通道（贝塞尔/线性/常量/线性混合）
        let main = mk_action(
            "主 Action",
            vec![
                (
                    PropertyPath::FontSize,
                    vec![
                        bez(0.0, 30.0, 0.0, 0.0, 2.0, 18.0),
                        bez(3.0, 66.0, -1.0, -14.0, 1.2, 12.0),
                        bez(6.0, 52.0, -0.8, -10.0, 1.0, 16.0),
                        bez(9.0, 80.0, -0.9, -14.0, 1.4, 12.0),
                        bez(12.0, 44.0, -1.2, -16.0, 0.8, 8.0),
                        bez(15.0, 60.0, -0.8, -8.0, 0.0, 0.0),
                    ],
                ),
                (
                    PropertyPath::PositionX,
                    vec![
                        lin(0.0, -0.5),
                        lin(2.5, 0.6),
                        lin(5.0, -0.2),
                        lin(7.5, 0.8),
                        lin(10.0, -0.6),
                        lin(12.5, 0.3),
                        lin(15.0, 0.0),
                    ],
                ),
                (
                    PropertyPath::FontRotationZ,
                    vec![
                        hold(0.0, 0.0),
                        hold(4.0, 30.0),
                        hold(8.0, 60.0),
                        hold(12.0, 90.0),
                        hold(15.0, 120.0),
                    ],
                ),
                (
                    PropertyPath::BorderX,
                    vec![
                        lin(0.0, 2.0),
                        lin(3.0, 6.0),
                        lin(6.0, 3.0),
                        lin(9.0, 8.0),
                        lin(12.0, 4.0),
                        lin(15.0, 5.0),
                    ],
                ),
            ],
        );

        // 奔跑：节奏快、周期短、位移左右往返
        let run = mk_action(
            "奔跑",
            vec![
                (
                    PropertyPath::PositionX,
                    vec![
                        lin(0.0, 0.0),
                        lin(0.5, 1.2),
                        lin(1.0, 0.0),
                        lin(1.5, -1.2),
                        lin(2.0, 0.0),
                    ],
                ),
                (
                    PropertyPath::PositionY,
                    vec![
                        lin(0.0, 0.0),
                        lin(0.5, 1.8),
                        lin(1.0, 0.0),
                        lin(1.5, 1.8),
                        lin(2.0, 0.0),
                    ],
                ),
            ],
        );

        // 跳跃：抛物线上升 + 落地 + 旋转
        let jump = mk_action(
            "跳跃",
            vec![
                (
                    PropertyPath::PositionY,
                    vec![
                        lin(0.0, 0.0),
                        bez(1.0, 3.0, -0.2, 0.0, 0.2, 0.0),
                        lin(2.0, 0.0),
                    ],
                ),
                (
                    PropertyPath::FontRotationZ,
                    vec![hold(0.0, 0.0), hold(1.0, 90.0), hold(2.0, 0.0)],
                ),
            ],
        );

        let lib = &mut self.state.project.library.animation_library;
        let main_id = lib.insert_action(main);
        let run_id = lib.insert_action(run);
        let jump_id = lib.insert_action(jump);

        let ce = &mut self.ui_models.graph_editor;
        ce.actions = vec![
            ActionData {
                name: "主 Action".into(),
                id: main_id,
            },
            ActionData {
                name: "奔跑".into(),
                id: run_id,
            },
            ActionData {
                name: "跳跃".into(),
                id: jump_id,
            },
        ];
        ce.current_action = 0;
        ce.current_time = 3.0;

        // 装载第 0 个 Action（读 model 权威数据重建编辑数据）+ 手动跑投影
        let first_id = ce.current_action_id().expect("测试数据至少有一个 Action");
        let action = self
            .state
            .project
            .library
            .animation_library
            .get_action(first_id)
            .expect("ActionId 应存在");
        let tags = ce.load_action(0, action);
        self.ui_models.graph_editor.apply_tags(&tags);
        self.rebind_animation_filters();
        self.create_test_nla_data();
    }
}
