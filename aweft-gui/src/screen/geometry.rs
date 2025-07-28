use super::types::*;
use slotmap::SlotMap;

impl ScrAreaMap {
    /// 创建一个空的区域地图
    pub fn new() -> Self {
        Self {
            verts: SlotMap::with_key(),
            edges: SlotMap::with_key(),
            areas: SlotMap::with_key(),
        }
    }

    /// 创建具有正确DPI缩放的初始屏幕
    pub fn create_initial_screen_with_scaling(
        &mut self, 
        width: u32, 
        height: u32,
        ui_settings: &UIScaleSettings
    ) -> ScrAreaKey {
        // 创建四个顶点
        let bottom_left = self.create_vertex(0, 0);
        let top_left = self.create_vertex(0, height - 1);
        let top_right = self.create_vertex(width - 1, height - 1);
        let bottom_right = self.create_vertex(width - 1, 0);
        
        // 创建边
        self.create_edge(bottom_left, top_left);
        self.create_edge(top_left, top_right);
        self.create_edge(top_right, bottom_right);
        self.create_edge(bottom_right, bottom_left);
        
        // 创建区域 - 遵循Blender方式：只传入顶点引用和区域类型
        let mut area = ScrArea::new(
            bottom_left,
            top_left,
            top_right,
            bottom_right,
            AreaType::Empty,
        );
        
        // 窗口矩形
        let window_rect = Rcti {
            xmin: 0,
            xmax: width - 1,
            ymin: 0,
            ymax: height - 1,
        };
        
        // 关键：区域创建后立即计算几何属性
        // 这体现了"顶点->几何计算->区域属性"的正确顺序
        area.calculate_totrct_and_size(
            &self.verts,
            &window_rect,
            ui_settings,
            false, // 非全屏
            false, // 非临时窗口
            true,  // 单个区域
        );
        
        // 验证几何已正确计算
        debug_assert!(area.is_geometry_calculated(), "区域几何属性计算失败");
        
        self.areas.insert(area)
    }

    /// 更新所有区域的winx/winy，当窗口大小或DPI改变时调用
    pub fn update_all_areas_with_scaling(
        &mut self,
        window_rect: &Rcti,
        ui_settings: &UIScaleSettings,
        is_fullscreen: bool,
        is_temp_window: bool,
    ) {
        let is_single_area = self.areas.len() == 1;
        
        // 收集需要更新的区域keys（避免借用冲突）
        let area_keys: Vec<ScrAreaKey> = self.areas.keys().collect();
        
        for area_key in area_keys {
            if let Some(area) = self.areas.get_mut(area_key) {
                area.calculate_totrct_and_size(
                    &self.verts,
                    window_rect,
                    ui_settings,
                    is_fullscreen,
                    is_temp_window,
                    is_single_area,
                );
            }
        }
    }

    /// 查找给定顶点相关的所有边
    pub fn find_edges_for_vertex(&self, vertex_key: ScrVertKey) -> Vec<ScrEdgeKey> {
        let mut edges = Vec::new();
        for (edge_key, edge) in &self.edges {
            if edge.v1 == vertex_key || edge.v2 == vertex_key {
                edges.push(edge_key);
            }
        }
        edges
    }

    /// 查找连接两个顶点的边
    pub fn find_edge_between_vertices(&self, v1: ScrVertKey, v2: ScrVertKey) -> Option<ScrEdgeKey> {
        for (edge_key, edge) in &self.edges {
            if (edge.v1 == v1 && edge.v2 == v2) || (edge.v1 == v2 && edge.v2 == v1) {
                return Some(edge_key);
            }
        }
        None
    }

    /// 获取区域的所有顶点（按顺序）
    pub fn get_area_vertices_ordered(&self, area_key: ScrAreaKey) -> Option<[ScrVertKey; 4]> {
        if let Some(area) = self.areas.get(area_key) {
            Some([
                area.v_bottom_left,
                area.v_top_left,
                area.v_top_right,
                area.v_bottom_right,
            ])
        } else {
            None
        }
    }

    /// 查找可以分割的点
    pub fn find_split_point(
        &self,
        area_key: ScrAreaKey,
        mouse_x: i16,
        mouse_y: i16,
    ) -> Option<(SplitDirection, f32)> {
        // TODO: 你需要实现这个函数
        // 1. 获取指定的 area
        // 2. 检查鼠标位置是否在 area 内
        // 3. 根据鼠标位置计算分割方向和比例
        // 4. 返回分割信息

        unimplemented!("你需要实现查找分割点的逻辑")
    }

    /// 分割一个区域
    pub fn split_area(
        &mut self,
        area_key: ScrAreaKey,
        direction: SplitDirection,
        factor: f32,
    ) -> Option<ScrAreaKey> {
        // TODO: 实现区域分割 - 遵循新的设计原则
        // 
        // 正确的实现步骤：
        // 1. 获取要分割的原始区域的四个顶点
        // 2. 根据分割方向和比例，在适当位置创建新的顶点
        // 3. 创建新的边来连接这些顶点
        // 4. 修改原始区域的顶点引用（通过更新其v_xxx字段）
        // 5. 创建新区域 - 只传入顶点引用和区域类型
        // 6. 对两个区域都调用calculate_totrct_and_size()来计算几何属性
        // 7. 返回新区域的key
        //
        // 重要：区域的几何属性完全由顶点位置决定，不应该直接设置winx/winy

        unimplemented!("区域分割功能待实现 - 需要遵循'顶点->计算->几何'的设计原则")
    }

    /// 辅助函数：创建一个新顶点（简化版）
    fn create_vertex(&mut self, x: u32, y: u32) -> ScrVertKey {
        let new_vert = ScrVert {
            newv: None,
            vec: Vec2u32 { x, y },
            flag: 0,
            edit_flag: 0,
        };
        self.verts.insert(new_vert)
    }

    /// 辅助函数：创建一条新边（简化版）
    fn create_edge(&mut self, v1: ScrVertKey, v2: ScrVertKey) -> ScrEdgeKey {
        let new_edge = ScrEdge {
            v1,
            v2,
            border: false,
            flag: 0,
        };
        self.edges.insert(new_edge)
    }


    /// 添加新顶点到现有几何结构中
    /// 这个函数展示了如何在不手动管理双向链表的情况下添加新元素
    pub fn add_vertex_to_geometry(&mut self, x: u32, y: u32) -> ScrVertKey {
        // 简单地创建新顶点，不需要设置prev/next
        // SlotMap会自动管理存储和遍历顺序
        self.create_vertex(x, y)
    }

    /// 添加新边到现有几何结构中
    pub fn add_edge_to_geometry(&mut self, v1: ScrVertKey, v2: ScrVertKey) -> ScrEdgeKey {
        // 简单地创建新边，不需要设置prev/next
        // 几何关系通过v1/v2维护，SlotMap管理存储
        self.create_edge(v1, v2)
    }

    /// 插入顶点到两个已存在顶点之间的边上
    pub fn insert_vertex_on_edge(&mut self, edge_key: ScrEdgeKey, x: u32, y: u32) -> Option<ScrVertKey> {
        if let Some(edge) = self.edges.get(edge_key) {
            let v1 = edge.v1;
            let v2 = edge.v2;
            
            // 创建新顶点
            let new_vertex = self.create_vertex(x, y);
            
            // 移除原边
            self.edges.remove(edge_key);
            
            // 创建两条新边
            self.create_edge(v1, new_vertex);
            self.create_edge(new_vertex, v2);
            
            Some(new_vertex)
        } else {
            None
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_create_vertex_simple() {
        let mut screen_map = ScrAreaMap::new();

        // 测试创建一个独立的顶点
        let vertex_key = screen_map.create_vertex(100, 200);

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
    fn test_create_edge_simple() {
        let mut screen_map = ScrAreaMap::new();

        // 先创建两个顶点
        let v1 = screen_map.create_vertex(0, 0);
        let v2 = screen_map.create_vertex(100, 100);

        // 创建连接这两个顶点的边
        let edge_key = screen_map.create_edge(v1, v2);

        // 验证边是否正确创建
        let edge = screen_map.edges.get(edge_key).unwrap();
        assert_eq!(edge.v1, v1);
        assert_eq!(edge.v2, v2);
        assert_eq!(edge.border, false);
        assert_eq!(edge.flag, 0);

        // 验证 SlotMap 中确实有这条边
        assert_eq!(screen_map.edges.len(), 1);
        assert_eq!(screen_map.verts.len(), 2);
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

    #[test]
    fn test_add_multiple_edges() {
        let mut screen_map = ScrAreaMap::new();

        // 创建顶点
        let v1 = screen_map.create_vertex(0, 0);
        let v2 = screen_map.create_vertex(100, 0);
        let v3 = screen_map.create_vertex(100, 100);

        // 添加多条边
        let e1 = screen_map.add_edge_to_geometry(v1, v2);
        let e2 = screen_map.add_edge_to_geometry(v2, v3);

        // 验证所有边都被正确添加
        assert_eq!(screen_map.edges.len(), 2);
        assert!(screen_map.edges.contains_key(e1));
        assert!(screen_map.edges.contains_key(e2));
    }

    #[test]
    fn test_find_edges_for_vertex() {
        let mut screen_map = ScrAreaMap::new();

        let v1 = screen_map.create_vertex(0, 0);
        let v2 = screen_map.create_vertex(100, 0);
        let v3 = screen_map.create_vertex(100, 100);

        let e1 = screen_map.create_edge(v1, v2);
        let e2 = screen_map.create_edge(v1, v3);

        // 查找v1相关的边
        let edges = screen_map.find_edges_for_vertex(v1);
        assert_eq!(edges.len(), 2);
        assert!(edges.contains(&e1));
        assert!(edges.contains(&e2));

        // 查找v2相关的边
        let edges = screen_map.find_edges_for_vertex(v2);
        assert_eq!(edges.len(), 1);
        assert!(edges.contains(&e1));
    }

    #[test]
    fn test_insert_vertex_on_edge() {
        let mut screen_map = ScrAreaMap::new();

        let v1 = screen_map.create_vertex(0, 0);
        let v2 = screen_map.create_vertex(100, 0);
        let edge = screen_map.create_edge(v1, v2);

        // 在边中间插入顶点
        let new_vertex = screen_map.insert_vertex_on_edge(edge, 50, 0);
        
        assert!(new_vertex.is_some());
        let new_v = new_vertex.unwrap();
        
        // 验证新顶点被创建
        assert_eq!(screen_map.verts.len(), 3);
        let vertex = screen_map.verts.get(new_v).unwrap();
        assert_eq!(vertex.vec.x, 50);
        assert_eq!(vertex.vec.y, 0);
        
        // 验证原边被移除，新边被创建
        assert_eq!(screen_map.edges.len(), 2);
        assert!(!screen_map.edges.contains_key(edge));
    }

    #[test]
    fn test_empty_map() {
        let screen_map = ScrAreaMap::new();
        assert_eq!(screen_map.verts.len(), 0);
        assert_eq!(screen_map.edges.len(), 0);
        assert_eq!(screen_map.areas.len(), 0);
    }

    #[test]
    fn test_area_size_calculation() {
        let mut screen_map = ScrAreaMap::new();
        let ui_settings = UIScaleSettings::default();

        // 创建一个1920x1080的屏幕
        let area_key = screen_map.create_initial_screen_with_scaling(1920, 1080, &ui_settings);

        let area = screen_map.areas.get(area_key).unwrap();

        // 验证几何已正确计算
        assert!(area.is_geometry_calculated(), "区域几何应该已计算");
        
        // 验证计算结果合理
        let (tot_rct, winx, winy) = area.geometry();
        assert!(winx > 0, "区域宽度应该大于0，实际值: {}", winx);
        assert!(winy > 0, "区域高度应该大于0，实际值: {}", winy);
        assert!(winx <= 1920, "区域宽度不应超过窗口宽度，实际值: {}", winx);
        assert!(winy <= 1080, "区域高度不应超过窗口高度，实际值: {}", winy);

        println!("测试区域大小: {}x{}", winx, winy);
        println!("区域边界: ({}, {}) 到 ({}, {})", 
                 tot_rct.xmin, tot_rct.ymin, tot_rct.xmax, tot_rct.ymax);
    }
}
