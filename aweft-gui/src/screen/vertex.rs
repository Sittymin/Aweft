use super::primitives::Vec2u32;
use slotmap::new_key_type;

new_key_type! {
    pub struct ScrVertKey;
}
/// 屏幕顶点（Screen Vertex），构成区域布局的基础元素
#[derive(Debug, Clone)]
pub struct ScrVert {
    // 移除手动维护的双向链表指针 - 让SlotMap来管理顺序
    // 保留 newv 字段用于重复顶点合并功能
    pub newv: Option<ScrVertKey>, // 用于重复顶点合并的新顶点引用
    pub vec: Vec2u32,
    // NOTE: Maybe is not necessary (Copy from Blender code)
    // first one used internally, second one for tools
    pub flag: i16,
    pub edit_flag: i16,
}
// 用于在ScrAreaMap当中匹配设置ScrArea
pub enum VertPosition {
    TopLeft(ScrVertKey),
    TopRight(ScrVertKey),
    BottomRight(ScrVertKey),
    BottomLeft(ScrVertKey),
}

#[cfg(test)]
mod tests {
    use crate::screen::map::ScrAreaMap;

    #[test]
    fn test_create_vertex_simple() {
        let mut screen_map = <ScrAreaMap>::new();

        // 测试创建一个独立的顶点
        let vertex_key = screen_map.add_vertex_to_geometry(100, 200);

        // 验证顶点是否正确创建
        let vertex = screen_map.verts.get(vertex_key).unwrap();
        assert_eq!(vertex.vec.x, 100);
        assert_eq!(vertex.vec.y, 200);
        assert_eq!(vertex.newv, None);
        assert_eq!(vertex.flag, 0);
        assert_eq!(vertex.edit_flag, 0);

        // 验证 SlotMap 中确实有这个顶点
        assert_eq!(screen_map.verts.len(), 1);
    }

    #[test]
    fn test_add_multiple_vertices() {
        let mut screen_map = ScrAreaMap::new();

        // 添加多个顶点
        let v1 = screen_map.add_vertex_to_geometry(0, 0);
        let v2 = screen_map.add_vertex_to_geometry(100, 0);
        let v3 = screen_map.add_vertex_to_geometry(100, 100);

        // 验证所有顶点都被正确添加
        assert_eq!(screen_map.verts.len(), 3);
        assert!(screen_map.verts.contains_key(v1));
        assert!(screen_map.verts.contains_key(v2));
        assert!(screen_map.verts.contains_key(v3));
    }
}
