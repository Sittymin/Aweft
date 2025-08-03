use anyhow::Result;

use crate::SplitPreviewOverlayData;
use crate::types::{
    AreaType, Rcti, ScrArea, ScrAreaKey, ScrAreaMap, ScrEdge, ScrEdgeKey, ScrVert, ScrVertKey,
    SplitDirection, UISettings, Vec2u32,
};

impl ScrAreaMap {
    /// 创建具有正确DPI缩放的初始屏幕（Slint坐标系）
    pub fn create_initial_screen_with_scaling(
        &mut self,
        width: u32,
        height: u32,
        ui_settings: &UISettings,
    ) -> ScrAreaKey {
        // 创建四个顶点（Slint坐标系，左上角原点）
        let top_left = self.create_vertex(0, 0);
        let top_right = self.create_vertex(width - 1, 0);
        let bottom_right = self.create_vertex(width - 1, height - 1);
        let bottom_left = self.create_vertex(0, height - 1);

        // 创建边
        self.create_edge(top_left, top_right);
        self.create_edge(top_right, bottom_right);
        self.create_edge(bottom_right, bottom_left);
        self.create_edge(bottom_left, top_left);

        // 创建区域 - 顶点顺序：左上、右上、右下、左下
        let mut area = ScrArea::new(
            top_left,
            top_right,
            bottom_right,
            bottom_left,
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

    /// 创建三个区域的布局：第一行水平两个区域，第二行独占一个区域
    ///
    /// 布局示意图 (Slint坐标系，左上角原点，y向下增长)：
    /// (0,0) +-------+-------+ (width,0)
    ///       | Area1 | Area2 |  <- 第一行，各占50%宽度
    ///       +-------+-------+ (0,mid_y) -> (width,mid_y)
    ///       |    Area3      |  <- 第二行，独占100%宽度
    /// (0,height) +-----------+ (width,height)
    pub fn create_three_area_layout(
        &mut self,
        width: &u32,
        height: &u32,
        ui_settings: &UISettings,
    ) -> Vec<ScrAreaKey> {
        // 计算分割点
        let mid_x = width / 2; // 水平中点
        let mid_y = height / 2; // 垂直中点

        // 创建6个顶点（Slint坐标系，左上角原点）
        // 外围4个顶点
        let v_top_left = self.create_vertex(0, 0); // 左上角
        let v_top_right = self.create_vertex(width - 1, 0); // 右上角
        let v_bottom_right = self.create_vertex(width - 1, height - 1); // 右下角
        let v_bottom_left = self.create_vertex(0, height - 1); // 左下角

        // 内部2个顶点（只需要这两个！）
        let v_mid_top = self.create_vertex(mid_x, 0); // 上边中点
        let v_left_middle = self.create_vertex(0, mid_y); // 左边中点
        let v_right_middle = self.create_vertex(width - 1, mid_y); // 右边中点
        let v_mid_middle = self.create_vertex(mid_x, mid_y); // 中心点

        // 创建边
        // 外围边界
        self.create_edge(v_top_left, v_top_right); // 上边界
        self.create_edge(v_top_right, v_bottom_right); // 右边界
        self.create_edge(v_bottom_right, v_bottom_left); // 下边界
        self.create_edge(v_bottom_left, v_top_left); // 左边界

        // 内部分割线
        self.create_edge(v_mid_top, v_mid_middle); // 垂直分割线（上半段）
        self.create_edge(v_left_middle, v_right_middle); // 水平分割线

        // 连接分割线到边界
        self.create_edge(v_top_left, v_mid_top); // 上边左半段
        self.create_edge(v_mid_top, v_top_right); // 上边右半段
        self.create_edge(v_left_middle, v_mid_middle); // 左边下半段到中心
        self.create_edge(v_mid_middle, v_right_middle); // 中心到右边中点

        // 窗口矩形（Slint坐标系）
        let window_rect = Rcti {
            xmin: 0,
            xmax: width - 1,
            ymin: 0,
            ymax: height - 1,
        };

        // 创建三个区域
        let mut areas = Vec::new();

        // 区域1：左上角 (Timeline)
        // 顶点顺序：左上、右上、右下、左下
        let mut area1 = ScrArea::new(
            v_top_left,    // 左上
            v_mid_top,     // 右上（上边中点）
            v_mid_middle,  // 右下（中心点）
            v_left_middle, // 左下（左边中点）
            AreaType::Timeline,
        );
        area1.calculate_totrct_and_size(
            &self.verts,
            &window_rect,
            ui_settings,
            false,
            false,
            false,
        );
        let area1_key = self.areas.insert(area1);
        areas.push(area1_key);

        // 区域2：右上角 (Video)
        // 顶点顺序：左上、右上、右下、左下
        let mut area2 = ScrArea::new(
            v_mid_top,      // 左上（上边中点）
            v_top_right,    // 右上
            v_right_middle, // 右下（右边中点）
            v_mid_middle,   // 左下（中心点）
            AreaType::Video,
        );
        area2.calculate_totrct_and_size(
            &self.verts,
            &window_rect,
            ui_settings,
            false,
            false,
            false,
        );
        let area2_key = self.areas.insert(area2);
        areas.push(area2_key);

        // 区域3：下方整行 (SubtitleEditor)
        // 顶点顺序：左上、右上、右下、左下
        let mut area3 = ScrArea::new(
            v_left_middle,  // 左上（左边中点）
            v_right_middle, // 右上（右边中点）
            v_bottom_right, // 右下
            v_bottom_left,  // 左下
            AreaType::SubtitleEditor,
        );
        area3.calculate_totrct_and_size(
            &self.verts,
            &window_rect,
            ui_settings,
            false,
            false,
            false,
        );
        let area3_key = self.areas.insert(area3);
        areas.push(area3_key);

        // 验证所有区域几何都已正确计算
        for &area_key in &areas {
            let area = self.areas.get(area_key).unwrap();
            debug_assert!(
                area.is_geometry_calculated(),
                "区域 {:?} 几何属性计算失败",
                area_key
            );
        }

        areas
    }

    /// 更新所有区域的winx/winy，当窗口大小或DPI改变时调用
    pub fn update_all_areas_rcti(
        &mut self,
        window_rect: &Rcti,
        ui_settings: &UISettings,
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
        split_data: SplitPreviewOverlayData,
        window_rect: &Rcti,
        ui_settings: &UISettings,
    ) -> Result<ScrAreaKey> {
        let is_horizontal_split = split_data.width1 != split_data.width2;
        if is_horizontal_split {
            let split_x = split_data.x2 as u32;
            let (original_rect, v1, v2, v3, v4, area_type) = {
                let source_area = self
                    .areas
                    .get(area_key)
                    .ok_or_else(|| anyhow::anyhow!("源区域 {:?} 不存在", area_key))?;
                let (rect, _, _) = source_area.geometry();
                (
                    *rect,
                    source_area.v_top_left,
                    source_area.v_top_right,
                    source_area.v_bottom_right,
                    source_area.v_bottom_left,
                    source_area.area_type.clone(),
                )
            };

            let new_vert_top = self.add_vertex_to_geometry(split_x, original_rect.ymin);
            let new_vert_bottom = self.add_vertex_to_geometry(split_x, original_rect.ymax);

            // 左部分
            self.add_edge_to_geometry(v1, new_vert_top);
            self.add_edge_to_geometry(v4, new_vert_bottom);
            // 右部分
            self.add_edge_to_geometry(new_vert_top, v2);
            self.add_edge_to_geometry(new_vert_bottom, v3);
            self.add_edge_to_geometry(new_vert_top, new_vert_bottom);

            let new_area = ScrArea::new(new_vert_top, v2, v3, new_vert_bottom, area_type);
            let new_area_key = self.areas.insert(new_area);

            // 修改原始区域
            let source_area = self.areas.get_mut(area_key).unwrap();
            source_area.v_top_right = new_vert_top;
            source_area.v_bottom_right = new_vert_bottom;

            // 重新计算两个区域的几何属性
            source_area.calculate_totrct_and_size(
                &self.verts,
                window_rect,
                ui_settings,
                false,
                false,
                false,
            );
            let new_area_mut = self.areas.get_mut(new_area_key).unwrap();
            new_area_mut.calculate_totrct_and_size(
                &self.verts,
                window_rect,
                ui_settings,
                false,
                false,
                false,
            );

            println!("水平拆分完成，创建了新区域: {:?}", new_area_key);
            Ok(new_area_key)
        } else {
            let split_y = split_data.y2 as u32;
            let (original_rect, v1, v2, v3, v4, area_type) = {
                let source_area = self
                    .areas
                    .get(area_key)
                    .ok_or_else(|| anyhow::anyhow!("源区域 {:?} 不存在", area_key))?;
                let (rect, _, _) = source_area.geometry();
                (
                    *rect,
                    source_area.v_top_left,
                    source_area.v_top_right,
                    source_area.v_bottom_right,
                    source_area.v_bottom_left,
                    source_area.area_type.clone(),
                )
            };
            let new_vert_left = self.add_vertex_to_geometry(original_rect.xmin, split_y);
            let new_vert_right = self.add_vertex_to_geometry(original_rect.xmax, split_y);

            // 上部分
            self.add_edge_to_geometry(v1, new_vert_left);
            self.add_edge_to_geometry(v2, new_vert_right);
            // 下部分
            self.add_edge_to_geometry(new_vert_left, v4);
            self.add_edge_to_geometry(new_vert_right, v3);
            self.add_edge_to_geometry(new_vert_left, new_vert_right);

            let new_area = ScrArea::new(new_vert_left, new_vert_right, v3, v4, area_type);
            let new_area_key = self.areas.insert(new_area);

            // 修改原始区域
            let source_area = self.areas.get_mut(area_key).unwrap();
            source_area.v_bottom_left = new_vert_left;
            source_area.v_bottom_right = new_vert_right;

            // 重新计算两个区域的几何属性
            source_area.calculate_totrct_and_size(
                &self.verts,
                window_rect,
                ui_settings,
                false,
                false,
                false,
            );
            let new_area_mut = self.areas.get_mut(new_area_key).unwrap();
            new_area_mut.calculate_totrct_and_size(
                &self.verts,
                window_rect,
                ui_settings,
                false,
                false,
                false,
            );

            println!("水平拆分完成，创建了新区域: {:?}", new_area_key);
            Ok(new_area_key)
        }
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
    pub fn insert_vertex_on_edge(
        &mut self,
        edge_key: ScrEdgeKey,
        x: u32,
        y: u32,
    ) -> Option<ScrVertKey> {
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
        let mut screen_map = <ScrAreaMap>::new();

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
        let ui_settings = UISettings::default();

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
        println!(
            "区域边界: ({}, {}) 到 ({}, {})",
            tot_rct.xmin, tot_rct.ymin, tot_rct.xmax, tot_rct.ymax
        );
    }
}
