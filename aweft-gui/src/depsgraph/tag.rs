use std::rc::Rc;

use crate::screen::area::{AreaType, ScrAreaKey};
use crate::screen::edge::ScrEdgeKey;
use aweft_model::ids::{ItemId, NodeId, SocketId};
use aweft_model::structure::ItemLocation;
use chrono::TimeDelta;

/// 数据域：可被标记变更的权威数据。
/// 值对应 AppState 直接持有的数据
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum DataId {
    /// 项目字幕结构（行/分组/时间码/文本/层级）。
    ProjectStructure,
    /// 选中状态（LineEditor 选中集等）。
    Selection,
    /// 屏幕布局（areas/edges/verts 几何与类型）。
    ScreenLayout,
    /// 拖拽状态（分割/合并/边移动预览）。
    DragState,
    /// 动画数据（AnimData/Action，供曲线/Dope/NLA 投影）。
    Animation,
    /// 节点图数据（NodeGraph/ApplicationGraph 的节点/连线/端口值）。
    NodeGraph,
}

/// 节点图类型（tag 内区分样式图/应用图，定义在 model 侧）。
pub use aweft_model::node_graph::NodeGraphKind;

/// 节点图一次变更的精确描述（投影据此精确定位局部更新）。
#[derive(Debug, Clone)]
pub enum NodeGraphChange {
    /// 新增节点（含请求位置，供投影构建快照 + row_added）。
    NodesInserted {
        node_ids: Vec<NodeId>,
        positions: std::collections::HashMap<NodeId, (f32, f32)>,
    },
    /// 删除节点（投影移除行 + row_removed + 连线重映射）。
    NodesRemoved {
        node_ids: Vec<NodeId>,
    },
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
    ReferenceChanged {
        node_id: NodeId,
    },
    /// 新建链后切到它（投影：派生链菜单 + 设当前链 + 全量重建快照）。`target` 为目标链下标。
    ChainSwitched {
        target: usize,
    },
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
    // ── ProjectStructure ──────────────────────────────────────────────
    /// 行/分组结构变化（增删、层级调整）——需要重建行列表索引。
    StructureChanged,
    /// 分组折叠状态切换（局部：只影响过滤结果）。
    GroupCollapsedToggled {
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
    /// 事件时间码拖拽预览（不修改 structure，仅 UI 显示预览值）。
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
    /// 移除多项（已从 project.structure 移除）。
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
    AnimationStructureChanged,
    /// 动画编辑器：关键帧数据值变化（拖拽/插值/手柄调整）。
    /// 投影重建受影响通道的曲线段/手柄/连接线，并重算汇总条带中受影响的 ms 条目。
    AnimationKeyframesChanged {
        /// 数据变化的通道。
        curves: Vec<usize>,
        /// 汇总条带需要重算的 ms（拖拽移动时含旧位置 + 新位置）。
        summary_ms: Vec<i64>,
        /// 关键帧拖拽的被拖通道（投影对其做相邻手柄软钳制预览替换）。
        dragged_curve: Option<usize>,
        /// 从汇总条带拖拽时：被拖标记下标 + 累计时间增量（投影就地更新其 time，
        /// 不 insert/remove——拖拽期间重建汇总模型会破坏 Slint grab）。
        dragged_summary: Option<(usize, f32)>,
        /// 手柄拖拽时被移动的手柄（被拖 + 同步）：投影据此重建 preview_handles
        /// （软钳制有效控制点预览，拖拽上下文依赖）。
        handle_preview: Option<Vec<(usize, usize, bool)>>,
    },
    /// 动画编辑器：关键帧/通道选中变化。
    /// 投影同步 selected_display、汇总条目选中掩码、手柄 selected 视觉。
    AnimationSelectionChanged {
        /// 汇总条带需要重算选中掩码的 ms。
        summary_ms: Vec<i64>,
    },
    /// 动画编辑器：通道属性变化（可见/激活/可编辑/颜色模式）。
    /// 投影更新连接线（visible/颜色）与汇总条目（可见性影响聚合）。
    AnimationChannelChanged {
        curve: usize,
    },
    /// 动画编辑器：标量变化（播放头/当前通道/选中数/插值聚合）。
    /// 投影只同步 Slint 全局标量。
    AnimationScalarsChanged,

    // ── NodeGraph ─────────────────────────────────────────────────────
    /// 节点图结构/值变化（携带精确操作信息，投影据此局部更新）。
    NodeGraph {
        kind: NodeGraphKind,
        change: NodeGraphChange,
    },
}

/// 操作语义分类：同一语义的 Tag 归为一类，供操作按 `handles` 精确路由。
///
/// 一个操作声明它处理哪些 kind（`Operation::handles`），只有 kind 命中的
/// tag 才会被路由到该操作——避免把所有 tags 塞给所有操作。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum TagKind {
    /// 行/分组结构增删、层级调整（需重建或精确 insert/remove）。
    LineStructure,
    /// 行时间码值变化（含批量/预览/取消）。
    LineTimecode,
    /// 行文本元素变化。
    LineText,
    /// 分组折叠切换。
    GroupCollapse,
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
    AnimationStructure,
    /// 动画编辑器：关键帧数据值变化。
    AnimationKeyframes,
    /// 动画编辑器：选中变化。
    AnimationSelection,
    /// 动画编辑器：通道属性变化。
    AnimationChannel,
    /// 动画编辑器：标量变化。
    AnimationScalars,
    /// 节点图：结构（节点增删）。
    NodeGraphStructure,
    /// 节点图：连线（增删/重连）。
    NodeGraphLinks,
    /// 节点图：端口值/引用（单行更新）。
    NodeGraphSocket,
    /// 节点图：新建链（全量重建）。
    NodeGraphChain,
}

/// 操作唯一标识（编译期检查的枚举，可无限扩展加变体）。
///
/// 用于跨组件 `depends_on` 引用（Blender 的 OperationCode）。变体**全局唯一**，
/// 前缀隐含组件维度（`Line*` 属 LineEditor，`Screen*` 属 Screen）——投影组件
/// 是核心库随版本更新的内容，无运行时动态注册需求，故不需组件名字符串。
/// 拼写错误在编译期暴露，而非运行时 panic。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum OperationCode {
    /// LineEditor 行结构操作。
    LineStructure,
    /// LineEditor 时间码操作。
    LineTimecode,
    /// LineEditor 折叠操作。
    LineCollapse,
    /// LineEditor 选中操作。
    LineSelection,
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
    AnimationStructure,
    /// 动画编辑器：关键帧值变化（段/手柄/连接线/汇总增量重建）。
    AnimationCurve,
    /// 动画编辑器：选中变化（显示掩码/汇总掩码/手柄选中）。
    AnimationSelection,
    /// 动画编辑器：通道属性变化（连接线/汇总）。
    AnimationChannel,
    /// 动画编辑器：标量同步到 Slint 全局。
    AnimationScalars,
}

impl Tag {
    /// 该变更属于哪个数据域。
    pub fn data_id(&self) -> DataId {
        match self {
            Tag::StructureChanged
            | Tag::GroupCollapsedToggled { .. }
            | Tag::EventTimecodeChanged { .. }
            | Tag::TimecodePreviewChanged { .. }
            | Tag::EventsTimecodeChanged { .. }
            | Tag::EventTextChanged { .. }
            | Tag::ItemsInserted { .. }
            | Tag::ItemsRemoved { .. }
            | Tag::ItemsPasted { .. }
            | Tag::GroupsUngrouped { .. } => DataId::ProjectStructure,

            Tag::SelectionChanged { .. } => DataId::Selection,

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

            Tag::AnimationStructureChanged
            | Tag::AnimationKeyframesChanged { .. }
            | Tag::AnimationSelectionChanged { .. }
            | Tag::AnimationChannelChanged { .. }
            | Tag::AnimationScalarsChanged => DataId::Animation,

            Tag::NodeGraph { .. } => DataId::NodeGraph,
        }
    }

    /// 该变更的操作语义分类（操作路由键）。
    pub fn kind(&self) -> TagKind {
        match self {
            Tag::StructureChanged
            | Tag::ItemsInserted { .. }
            | Tag::ItemsRemoved { .. }
            | Tag::ItemsPasted { .. }
            | Tag::GroupsUngrouped { .. } => TagKind::LineStructure,

            Tag::EventTimecodeChanged { .. }
            | Tag::TimecodePreviewChanged { .. }
            | Tag::EventsTimecodeChanged { .. } => TagKind::LineTimecode,

            Tag::EventTextChanged { .. } => TagKind::LineText,

            Tag::GroupCollapsedToggled { .. } => TagKind::GroupCollapse,

            Tag::SelectionChanged { .. } => TagKind::Selection,

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

            Tag::AnimationStructureChanged => TagKind::AnimationStructure,
            Tag::AnimationKeyframesChanged { .. } => TagKind::AnimationKeyframes,
            Tag::AnimationSelectionChanged { .. } => TagKind::AnimationSelection,
            Tag::AnimationChannelChanged { .. } => TagKind::AnimationChannel,
            Tag::AnimationScalarsChanged => TagKind::AnimationScalars,

            Tag::NodeGraph { change, .. } => match change {
                NodeGraphChange::NodesInserted { .. } | NodeGraphChange::NodesRemoved { .. } => {
                    TagKind::NodeGraphStructure
                }
                NodeGraphChange::LinksChanged { .. } => TagKind::NodeGraphLinks,
                NodeGraphChange::SocketValueChanged { .. } | NodeGraphChange::ReferenceChanged { .. } => {
                    TagKind::NodeGraphSocket
                }
                NodeGraphChange::ChainSwitched { .. } => TagKind::NodeGraphChain,
            },
        }
    }
}
