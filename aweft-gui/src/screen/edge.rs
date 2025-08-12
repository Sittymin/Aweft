use super::vertex::ScrVertKey;
use slotmap::new_key_type;

new_key_type! {
    pub struct ScrEdgeKey;
}
/// 屏幕边缘（Screen Edge），连接两个顶点
#[derive(Debug, Clone)]
pub struct ScrEdge {
    // 移除手动维护的双向链表指针 - 让SlotMap来管理顺序
    pub v1: ScrVertKey, // 起始顶点
    pub v2: ScrVertKey, // 结束顶点
    pub border: bool,   // 标记是否为屏幕最外围的边缘
    pub flag: i16,
}
