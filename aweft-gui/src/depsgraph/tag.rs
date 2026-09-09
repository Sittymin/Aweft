use std::rc::Rc;

use crate::screen::area::{AreaType, ScrAreaKey};
use crate::screen::edge::ScrEdgeKey;
use aweft_model::ids::{ItemId, NodeId, SocketId};
use aweft_model::sequence::ItemLocation;
use chrono::TimeDelta;

/// 数据域：可被标记变更的权威数据。
/// 值对应 AppState 直接持有的数据
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum DataId {
    /// 项目字幕结构（行/分组/时间码/文本/层级）。
    Sequence,
    /// 选中状态（Sequence 选中集等）。
    Selection,
    /// 屏幕布局（areas/edges/verts 几何与类型）。
    ScreenLayout,
    /// 拖拽状态（分割/合并/边移动预览）。
    DragState,
    /// 动画数据（AnimData/Action，供曲线/Dope/NLA 投影）。
    Animation,
    /// 节点图数据（NodeGraph/ApplicationGraph 的节点/连线/端口值）。
    NodeGraph,
    /// 视频会话状态（会话结构、选择、主视频、位置、静音、时间和播放状态）。
    Video,
}

/// 节点图类型（tag 内区分样式图/应用图，定义在 model 侧）。
pub use aweft_model::node_graph::NodeGraphKind;

/// 手柄位置标识：(通道, 关键帧下标, 入/出柄)。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct HandleLoc {
    pub curve: usize,
    pub key: usize,
    pub is_in: bool,
}

/// 汇总条带拖拽上下文：被拖标记下标 + 累计时间增量（投影就地更新其 time）。
#[derive(Debug, Clone, Copy)]
pub struct SummaryDrag {
    pub index: usize,
    pub delta_time: f32,
}

/// NLA 片断定位：稳定 ID + 当前权威轨道/片断下标。
///
/// 下标用于派生模型的局部更新，ID 用于结构变化后避免更新到错误片段。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct NlaStripRef {
    pub id: u64,
    pub track: usize,
    pub index: usize,
}

/// NLA 片断移动：稳定 ID + 源轨/下标 → 目标轨。
/// 多选整体拖拽也用它表达同轨重排，投影会先批量移除再批量插入。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct NlaStripMove {
    pub id: u64,
    pub src_track: usize,
    pub src_index: usize,
    pub dst_track: usize,
}

/// NLA 片断同轨重排：水平穿越后按 start 排序（投影 remove 旧下标 + insert 新下标）。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct NlaStripReorder {
    pub track: usize,
    pub old_index: usize,
    pub new_index: usize,
}

/// NLA 片断插入：向轨道插入一个新片断（"向下插入"到已有轨道，row_added）。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct NlaStripInsert {
    pub track: usize,
    pub index: usize,
}

/// 节点图一次变更的精确描述（投影据此精确定位局部更新）。
#[derive(Debug, Clone)]
pub enum NodeGraphChange {
    /// 新增节点（含请求位置，供投影构建快照 + row_added）。
    NodesInserted {
        node_ids: Vec<NodeId>,
        positions: std::collections::HashMap<NodeId, (f32, f32)>,
    },
    /// 删除节点（投影移除行 + row_removed + 连线重映射）。
    NodesRemoved { node_ids: Vec<NodeId> },
    /// 连线增删/重连（投影更新受影响节点的 connected 标记 + 连线模型）。
    LinksChanged {
        /// connected 标记发生变化的节点（连线的两端点），投影只 row_changed 这些行。
        affected_nodes: Vec<NodeId>,
    },
    /// 端口值变化（投影更新该节点行：输入文本/角色选中）。
    SocketValueChanged {
        node_id: NodeId,
        socket_id: SocketId,
    },
    /// 引用节点换引用链（原位改值，投影重建该节点一行：menu_selected/端口）。
    ReferenceChanged { node_id: NodeId },
    /// 新建链后切到它（投影：派生链菜单 + 设当前链 + 全量重建快照）。`target` 为目标链下标。
    ChainSwitched { target: usize },
}

impl NodeGraphChange {
    /// 包装为 `Tag::NodeGraph`（带图类型）。
    pub fn into_tag(self, kind: NodeGraphKind) -> Tag {
        Tag::NodeGraph { kind, change: self }
    }
}

/// 选中操作类型：投影按此路由到对应的选择方法。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum SelectionKind {
    /// 单选（清空其他选中）。
    Single,
    /// 范围选择（从上次锚点到该行）。
    Range,
    /// 切换（有则取消，无则选中）。
    Toggle,
}

/// 一次数据变更的描述（带 payload，供投影做精准局部更新）。
///
/// 元素级：值变化 tag 携带 `item_id` + `element` 标识 + `ancestors`（祖先分组链），
/// 依赖图据此精确路由到"更新该行"与"更新所有祖先分组聚合"。
#[derive(Debug, Clone)]
pub enum Tag {
    // ── Sequence ──────────────────────────────────────────────
    /// 行/分组结构变化（增删、层级调整）——需要重建行列表索引。
    StructureChanged,
    /// 分组折叠状态切换（局部：只影响过滤结果）。
    SequenceCollapsedToggled {
        group_id: ItemId,
        /// 祖先分组链（自身不折叠影响，仅用于可能的聚合）。
        ancestors: Vec<ItemId>,
    },
    /// 事件时间码元素变化（start/end）。`ancestors` 供聚合传播到分组头。
    EventTimecodeChanged {
        event_id: ItemId,
        is_start_time: bool,
        time: TimeDelta,
        /// 祖先分组链（叶子到根）。
        ancestors: Vec<ItemId>,
    },
    /// 事件时间码拖拽预览（不修改 sequence，仅 UI 显示预览值）。
    TimecodePreviewChanged {
        event_id: ItemId,
        is_start_time: bool,
        time: TimeDelta,
        /// 祖先分组链（叶子到根）。
        ancestors: Vec<ItemId>,
    },
    /// 批量事件时间码元素变化。
    EventsTimecodeChanged {
        updates: Vec<(ItemId, bool, TimeDelta)>,
        /// 所有涉及行的祖先分组链（去重后）。
        ancestors: Vec<ItemId>,
    },
    /// 事件文本元素变化。
    EventTextChanged {
        event_id: ItemId,
        ancestors: Vec<ItemId>,
    },
    /// 插入一项。
    ItemsInserted {
        item_id: ItemId,
        new_item_id: ItemId,
    },
    /// 移除多项（已从 project.sequence 移除）。
    ItemsRemoved {
        item_ids: Vec<ItemId>,
    },
    /// 粘贴子树（已分配新 ItemId）。
    ItemsPasted {
        new_root_ids: Vec<ItemId>,
        base_location: ItemLocation,
    },
    /// 解散分组。
    GroupsUngrouped {
        group_id: ItemId,
        new_ids: Vec<ItemId>,
    },

    // ── Selection ─────────────────────────────────────────────────────
    /// 选中状态元素变化。`affected` 含被选中的行 + 受影响的祖先分组头。
    SelectionChanged {
        item_id: ItemId,
        /// 选择类型：单选 / 范围 / 切换——投影据此调用对应方法，
        /// 否则无法区分（SelectToggle 会被误降级为 select_single）。
        kind: SelectionKind,
        /// 该行 + 所有祖先分组头（需要刷新 selected 状态的行）。
        affected: Vec<ItemId>,
    },
    /// 框选（时间 × 行，求交语义，`store/utils/marquee::clamp_rows`）。
    /// 由 `SequenceAction::MarqueeSelect` 产生，经 `Selection` 数据域走依赖图。
    SelectionMarquee {
        x_lo: f32,
        x_hi: f32,
        row_lo: i32,
        row_hi: i32,
        modifier: crate::bridge::types::SlintSelectionModifier,
    },
    /// 清空选中（空白点击）。
    SelectionDeselectedAll,

    // ── ScreenLayout ──────────────────────────────────────────────────
    ScreenInitialized,
    ScreenResized {
        old_size: (u32, u32),
        new_size: (u32, u32),
    },
    ScaleChanged {
        old_scale: f64,
        new_scale: f64,
    },
    AreaTypeChanged {
        area_key: ScrAreaKey,
        old_type: AreaType,
        new_type: AreaType,
    },
    AreasSplit {
        parent_key: ScrAreaKey,
        new_area_key: ScrAreaKey,
    },
    AreasJoined {
        /// 被移除的 area（主合并 + 递归空位填充）。
        removed_keys: Vec<ScrAreaKey>,
        /// 新创建的 area（对齐裁剪 split 产生）。
        added_keys: Vec<ScrAreaKey>,
        merged_into: ScrAreaKey,
    },

    // ── DragState ─────────────────────────────────────────────────────
    SplitPreviewUpdated,
    JoinPreviewUpdated,
    PreviewCleared,
    EdgeUpdated {
        edge_keys: Rc<Vec<ScrEdgeKey>>,
        area_keys: Rc<Vec<ScrAreaKey>>,
    },
    DragEnded {
        areas_changed: bool,
        /// 边移动结束是否发生了顶点合并（拓扑变化）——拓扑变了才需全量重建 edges。
        topology_changed: bool,
    },

    // ── Animation ─────────────────────────────────────────────────────
    /// 动画编辑器：装载/重置/新建 Action / 搜索变化（过滤视图重建 + 全量派生重建）。
    GraphStructureChanged,
    /// 动画编辑器：关键帧数据值变化（拖拽/插值/手柄调整）。
    /// 投影重建受影响通道的曲线段/手柄/连接线，并重算汇总条带中受影响的 ms 条目。
    GraphKeyframesChanged {
        /// 数据变化的通道。
        curves: Vec<usize>,
        /// 汇总条带需要重算的 ms（拖拽移动时含旧位置 + 新位置）。
        summary_ms: Vec<i64>,
        /// 关键帧拖拽的被拖通道（投影对其做相邻手柄软钳制预览替换）。
        dragged_curve: Option<usize>,
        /// 从汇总条带拖拽时：被拖标记 + 累计时间增量（投影就地更新其 time，
        /// 不 insert/remove——拖拽期间重建汇总模型会破坏 Slint grab）。
        dragged_summary: Option<SummaryDrag>,
        /// 手柄拖拽时被移动的手柄（被拖 + 同步 + 对向柄）：投影据此重建
        /// preview_handles（软钳制有效控制点预览，拖拽上下文依赖）。
        handle_preview: Option<Vec<HandleLoc>>,
    },
    /// 动画编辑器：关键帧/通道选中变化。
    /// 投影同步 selected_display、汇总条目选中掩码、手柄 selected 视觉。
    GraphSelectionChanged {
        /// 汇总条带需要重算选中掩码的 ms。
        summary_ms: Vec<i64>,
    },
    /// 动画编辑器：通道属性变化（可见/激活/可编辑/颜色模式）。
    /// 投影更新连接线（visible/颜色）与汇总条目（可见性影响聚合）。
    GraphChannelChanged {
        curve: usize,
    },
    /// 动画编辑器：拖拽结束后的数据整理（恢复编辑不变量）。
    /// 投影清理拖拽残留（handles preview 替换换回普通）、重排乱序关键帧。
    /// 与选中变化分离：其他组件（如 NLA）可单独依赖本整理。
    GraphDragEnded,
    /// 动画编辑器：标量变化（播放头/当前通道/选中数/插值聚合）。
    /// 投影只同步 Slint 全局标量。
    GraphScalarsChanged,
    /// NLA 编辑器：需要重建过滤派生视图的变化（装载/搜索）。
    /// 投影全量重建派生层（tracks/strip_models/过滤视图重新绑定）。
    NlaStructureChanged,
    /// NLA 编辑器：片断选中变化（单选/多选/清空）。
    /// 投影只更新选中态（`set_row_data` 选中列），不重排结构。
    NlaSelectionChanged {
        /// 本次选中的片断（选中态翻转/置位）。
        updated: Vec<NlaStripRef>,
        /// 是否先清空全部选中（单选模式）。
        deselect_all: bool,
    },
    /// NLA 编辑器：片断位置/拖拽值变化（start/end/dragging）。
    /// 投影就地 `set_row_data`（保住 grab）。
    NlaStripPositionChanged {
        /// 位置发生变化的片断。
        updated: Vec<NlaStripRef>,
    },
    /// NLA 编辑器：片断结构变化（跨轨移动/同轨重排/插入）。
    /// 投影精确 remove+insert（row_added/removed，存量复用）。
    NlaStripStructureChanged {
        /// 移动片断（跨轨或多选整体同轨重排；remove 源轨 + 按 start 排序插入目标轨）。
        moved: Vec<NlaStripMove>,
        /// 同轨重排（水平穿越后按 start 排序：remove 旧下标 + insert 新下标）。
        reordered: Vec<NlaStripReorder>,
        /// 新增片断（向已有轨道插入一个片断，row_added）。
        inserted: Vec<NlaStripInsert>,
    },
    /// NLA 编辑器：轨道行级属性变化（启用/锁定开关）。
    /// 投影只更新该轨道行 + 过滤视图对应行（`set_row_data`，不重建）。
    NlaTrackChanged {
        /// 权威轨道下标。
        track: usize,
    },
    /// NLA 编辑器：新建轨道（"向下插入"放不下时新建）。
    /// 投影局部插入轨道行（row_added）+ 新片断模型 + 过滤视图对应行（按匹配），不重建存量。
    NlaTrackAdded {
        /// 权威轨道下标（追加到末尾，orig_index 不变）。
        track: usize,
    },

    // ── NodeGraph ─────────────────────────────────────────────────────
    /// 节点图结构/值变化（携带精确操作信息，投影据此局部更新）。
    NodeGraph {
        kind: NodeGraphKind,
        change: NodeGraphChange,
    },

    // ── Video ─────────────────────────────────────────────────────────
    /// 视频会话结构变化；`inserted` 区分追加与原地替换。
    VideoStructureChanged { index: usize, inserted: bool },
    /// 视频视口选择变化。
    VideoSelectionChanged,
    /// 主视频变化（同时影响参考框、位置和时间条标量）。
    VideoMainChanged,
    /// 视频视口位置变化。
    VideoPositionChanged { indices: Vec<usize> },
    /// 单个视频静音状态变化。
    VideoMuteChanged { index: usize },
    /// 视频会话时间变化（seek 或非实时状态同步）。
    /// sync_timeline=false 表示用户正在主动 Seek：仅刷新视频实际时间，不以钳制后的
    /// 会话时间覆盖动画时间轴；播放时钟仍通过实时出口以实际位置接管时间轴。
    VideoTimeChanged {
        indices: Vec<usize>,
        sync_timeline: bool,
    },
    /// 全局视频 transport 播放状态变化。
    VideoPlaybackChanged,
    /// 某个视频当前播放音轨发生切换。
    VideoAudioTrackChanged { video_index: usize },
}

/// 操作语义分类：同一语义的 Tag 归为一类，供操作按 `handles` 精确路由。
///
/// 一个操作声明它处理哪些 kind（`Operation::handles`），只有 kind 命中的
/// tag 才会被路由到该操作——避免把所有 tags 塞给所有操作。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum TagKind {
    /// 行/分组结构增删、层级调整（需重建或精确 insert/remove）。
    SequenceStructure,
    /// 行时间码值变化（含批量/预览/取消）。
    SequenceTimecode,
    /// 行文本元素变化。
    SequenceText,
    /// 分组折叠切换。
    SequenceCollapse,
    /// 选中状态变化。
    Selection,
    /// 屏幕布局结构变化（初始/缩放/分割/合并）。
    ScreenStructure,
    /// 区域类型切换。
    AreaType,
    /// 拖拽覆盖层（split/join 预览、清除）。
    DragOverlay,
    /// 拖拽边移动（局部几何更新）。
    DragEdgeMove,
    /// 拖拽结束（全量重建）。
    DragEnded,
    /// 动画编辑器：结构（装载/重置/搜索）。
    GraphStructure,
    /// 动画编辑器：关键帧数据值变化。
    GraphKeyframes,
    /// 动画编辑器：选中变化。
    GraphSelection,
    /// 动画编辑器：拖拽结束后的数据整理。
    GraphDragEnded,
    /// 动画编辑器：通道属性变化。
    GraphChannel,
    /// 动画编辑器：标量变化。
    GraphScalars,
    /// NLA 编辑器：结构（装载/搜索过滤视图重建）。
    NlaStructure,
    /// NLA 编辑器：片断选中变化。
    NlaSelection,
    /// NLA 编辑器：片断位置值变化（拖拽）。
    NlaStripPosition,
    /// NLA 编辑器：片断结构变化（移动/重排/插入）。
    NlaStripStructure,
    /// NLA 编辑器：轨道属性行更新（启用/锁定，set_row_data）。
    NlaTrackProperties,
    /// NLA 编辑器：轨道结构新增（row_added）。
    NlaTrackStructure,
    /// 节点图：结构（节点增删）。
    NodeGraphStructure,
    /// 节点图：连线（增删/重连）。
    NodeGraphLinks,
    /// 节点图：端口值/引用（单行更新）。
    NodeGraphSocket,
    /// 节点图：新建链（全量重建）。
    NodeGraphChain,
    /// 视频：会话结构变化。
    VideoStructure,
    /// 视频：选择变化。
    VideoSelection,
    /// 视频：主视频变化。
    VideoMain,
    /// 视频：位置变化。
    VideoPosition,
    /// 视频：静音变化。
    VideoMute,
    /// 视频：时间变化。
    VideoTime,
    /// 视频：播放状态变化。
    VideoPlayback,
    /// 视频：当前播放音轨变化。
    VideoAudioTrack,
}

/// 操作唯一标识（编译期检查的枚举，可无限扩展加变体）。
///
/// 用于跨组件 `depends_on` 引用（Blender 的 OperationCode）。变体**全局唯一**，
/// 前缀隐含组件维度（`Line*` 属 Sequence，`Screen*` 属 Screen）——投影组件
/// 是核心库随版本更新的内容，无运行时动态注册需求，故不需组件名字符串。
/// 拼写错误在编译期暴露，而非运行时 panic。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum OperationCode {
    /// Sequence 行结构操作。
    SequenceStructure,
    /// Sequence 时间码操作。
    SequenceTimecode,
    /// Sequence 折叠操作。
    SequenceCollapse,
    /// Sequence 选中操作。
    SequenceSelection,
    /// Screen 覆盖层操作。
    ScreenOverlays,
    /// Screen 布局操作。
    ScreenLayout,
    /// 节点图：结构操作（节点增删）。
    NodeGraphStructure,
    /// 节点图：连线操作（增删/重连）。
    NodeGraphLinks,
    /// 节点图：端口值/引用操作（单行更新）。
    NodeGraphSocket,
    /// 节点图：新建链操作（全量重建）。
    NodeGraphChain,
    /// 动画编辑器：结构变化（装载/重置/搜索）。
    GraphStructure,
    /// 动画编辑器：关键帧值变化（段/手柄/连接线/汇总增量重建）。
    GraphKeyframes,
    /// 动画编辑器：选中变化（显示掩码/汇总掩码/手柄选中）。
    GraphSelection,
    /// 动画编辑器：拖拽结束整理（预览恢复/重排/补柄）。
    GraphDragEnded,
    /// 动画编辑器：通道属性变化（连接线/汇总）。
    GraphChannel,
    /// 动画编辑器：标量同步到 Slint 全局。
    GraphScalars,
    /// NLA 编辑器：结构变化（装载/搜索）。
    NlaStructure,
    /// NLA 编辑器：片断选中变化。
    NlaSelection,
    /// NLA 编辑器：片断位置值变化（拖拽）。
    NlaStripPosition,
    /// NLA 编辑器：片断结构变化（移动/重排/插入）。
    NlaStripStructure,
    /// NLA 编辑器：轨道属性行更新（启用/锁定）。
    NlaTrackProperties,
    /// NLA 编辑器：轨道结构新增（row_added）。
    NlaTrackAdded,
    /// 视频会话结构投影。
    VideoStructure,
    /// 视频选择与主视频标量投影。
    VideoSelection,
    /// 视频视口位置投影。
    VideoPosition,
    /// 视频静音投影。
    VideoMute,
    /// 视频时间条与活跃时间投影。
    VideoTime,
    /// 视频播放状态投影。
    VideoPlayback,
    /// 视频当前播放音轨投影。
    VideoAudioTrack,
}

impl Tag {
    /// 该变更属于哪个数据域。
    pub fn data_id(&self) -> DataId {
        match self {
            Tag::StructureChanged
            | Tag::SequenceCollapsedToggled { .. }
            | Tag::EventTimecodeChanged { .. }
            | Tag::TimecodePreviewChanged { .. }
            | Tag::EventsTimecodeChanged { .. }
            | Tag::EventTextChanged { .. }
            | Tag::ItemsInserted { .. }
            | Tag::ItemsRemoved { .. }
            | Tag::ItemsPasted { .. }
            | Tag::GroupsUngrouped { .. } => DataId::Sequence,

            Tag::SelectionChanged { .. }
            | Tag::SelectionMarquee { .. }
            | Tag::SelectionDeselectedAll => DataId::Selection,

            Tag::ScreenInitialized
            | Tag::ScreenResized { .. }
            | Tag::ScaleChanged { .. }
            | Tag::AreaTypeChanged { .. }
            | Tag::AreasSplit { .. }
            | Tag::AreasJoined { .. } => DataId::ScreenLayout,

            Tag::SplitPreviewUpdated
            | Tag::JoinPreviewUpdated
            | Tag::PreviewCleared
            | Tag::EdgeUpdated { .. }
            | Tag::DragEnded { .. } => DataId::DragState,

            Tag::GraphStructureChanged
            | Tag::GraphKeyframesChanged { .. }
            | Tag::GraphSelectionChanged { .. }
            | Tag::GraphDragEnded
            | Tag::GraphChannelChanged { .. }
            | Tag::GraphScalarsChanged
            | Tag::NlaStructureChanged
            | Tag::NlaSelectionChanged { .. }
            | Tag::NlaStripPositionChanged { .. }
            | Tag::NlaStripStructureChanged { .. }
            | Tag::NlaTrackChanged { .. }
            | Tag::NlaTrackAdded { .. } => DataId::Animation,

            Tag::NodeGraph { .. } => DataId::NodeGraph,

            Tag::VideoStructureChanged { .. }
            | Tag::VideoSelectionChanged
            | Tag::VideoMainChanged
            | Tag::VideoPositionChanged { .. }
            | Tag::VideoMuteChanged { .. }
            | Tag::VideoTimeChanged { .. }
            | Tag::VideoPlaybackChanged
            | Tag::VideoAudioTrackChanged { .. } => DataId::Video,
        }
    }

    /// 该变更的操作语义分类（操作路由键）。
    pub fn kind(&self) -> TagKind {
        match self {
            Tag::StructureChanged
            | Tag::ItemsInserted { .. }
            | Tag::ItemsRemoved { .. }
            | Tag::ItemsPasted { .. }
            | Tag::GroupsUngrouped { .. } => TagKind::SequenceStructure,

            Tag::EventTimecodeChanged { .. }
            | Tag::TimecodePreviewChanged { .. }
            | Tag::EventsTimecodeChanged { .. } => TagKind::SequenceTimecode,

            Tag::EventTextChanged { .. } => TagKind::SequenceText,

            Tag::SequenceCollapsedToggled { .. } => TagKind::SequenceCollapse,

            Tag::SelectionChanged { .. }
            | Tag::SelectionMarquee { .. }
            | Tag::SelectionDeselectedAll => TagKind::Selection,

            Tag::ScreenInitialized
            | Tag::ScreenResized { .. }
            | Tag::ScaleChanged { .. }
            | Tag::AreasSplit { .. }
            | Tag::AreasJoined { .. } => TagKind::ScreenStructure,

            Tag::AreaTypeChanged { .. } => TagKind::AreaType,

            Tag::SplitPreviewUpdated | Tag::JoinPreviewUpdated | Tag::PreviewCleared => {
                TagKind::DragOverlay
            }

            Tag::EdgeUpdated { .. } => TagKind::DragEdgeMove,

            Tag::DragEnded { .. } => TagKind::DragEnded,

            Tag::GraphStructureChanged => TagKind::GraphStructure,
            Tag::GraphKeyframesChanged { .. } => TagKind::GraphKeyframes,
            Tag::GraphSelectionChanged { .. } => TagKind::GraphSelection,
            Tag::GraphDragEnded => TagKind::GraphDragEnded,
            Tag::GraphChannelChanged { .. } => TagKind::GraphChannel,
            Tag::GraphScalarsChanged => TagKind::GraphScalars,

            Tag::NlaStructureChanged => TagKind::NlaStructure,
            Tag::NlaSelectionChanged { .. } => TagKind::NlaSelection,
            Tag::NlaStripPositionChanged { .. } => TagKind::NlaStripPosition,
            Tag::NlaStripStructureChanged { .. } => TagKind::NlaStripStructure,
            Tag::NlaTrackChanged { .. } => TagKind::NlaTrackProperties,
            Tag::NlaTrackAdded { .. } => TagKind::NlaTrackStructure,

            Tag::NodeGraph { change, .. } => match change {
                NodeGraphChange::NodesInserted { .. } | NodeGraphChange::NodesRemoved { .. } => {
                    TagKind::NodeGraphStructure
                }
                NodeGraphChange::LinksChanged { .. } => TagKind::NodeGraphLinks,
                NodeGraphChange::SocketValueChanged { .. }
                | NodeGraphChange::ReferenceChanged { .. } => TagKind::NodeGraphSocket,
                NodeGraphChange::ChainSwitched { .. } => TagKind::NodeGraphChain,
            },

            Tag::VideoStructureChanged { .. } => TagKind::VideoStructure,
            Tag::VideoSelectionChanged => TagKind::VideoSelection,
            Tag::VideoMainChanged => TagKind::VideoMain,
            Tag::VideoPositionChanged { .. } => TagKind::VideoPosition,
            Tag::VideoMuteChanged { .. } => TagKind::VideoMute,
            Tag::VideoTimeChanged { .. } => TagKind::VideoTime,
            Tag::VideoPlaybackChanged => TagKind::VideoPlayback,
            Tag::VideoAudioTrackChanged { .. } => TagKind::VideoAudioTrack,
        }
    }
}
