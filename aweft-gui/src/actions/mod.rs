pub mod animation;
pub mod area;
pub mod drag;
pub mod sequence;
pub mod media;
pub mod node_graph;
pub mod screen;

/// 顶层 Action - 按功能域分类
#[derive(Debug, Clone)]
pub enum Action {
    /// 屏幕/窗口相关操作
    Screen(screen::ScreenAction),
    /// 拖拽相关操作
    Drag(drag::DragAction),
    /// 区域相关操作
    Area(area::AreaAction),
    /// 媒体相关操作
    Media(media::MediaAction),
    Sequence(sequence::SequenceAction),
    /// 节点图（样式图/应用图）结构/值操作
    NodeGraph(node_graph::NodeGraphAction),
    /// 动画域（统一曲线编辑器/摄影表/NLA，共享 DataId::Animation）
    Animation(animation::AnimationAction),
}
