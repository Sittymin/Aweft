use anyhow::{Context, Result};
use slotmap::SlotMap;

use crate::types::{ActionZoneCorner, UISettings};

use super::{
    area::{AreaType, ScrArea, ScrAreaKey},
    edge::{ScrEdge, ScrEdgeKey},
    primitives::{Rcti, Vec2f64, Vec2u32},
    vertex::{ScrVert, ScrVertKey, VertPosition},
};

/// 屏幕区域地图，管理所有的顶点、边和区域
#[derive(Debug)]
pub struct ScrAreaMap {
    pub verts: SlotMap<ScrVertKey, ScrVert>, // 存储所有顶点
    pub edges: SlotMap<ScrEdgeKey, ScrEdge>, // 存储所有边
    pub areas: SlotMap<ScrAreaKey, ScrArea>, // 存储所有区域
}
impl ScrAreaMap {
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
        // 左上角边界
        self.create_edge(v_top_left, v_mid_top); // 上边界
        self.create_edge(v_mid_top, v_mid_middle); // 右边界
        self.create_edge(v_mid_middle, v_left_middle); // 下边界
        self.create_edge(v_left_middle, v_top_left); // 左边界

        // 右上角边界
        self.create_edge(v_mid_top, v_top_right);
        self.create_edge(v_top_right, v_right_middle);
        self.create_edge(v_right_middle, v_mid_middle);
        self.create_edge(v_mid_middle, v_mid_top);

        // 下方边界
        self.create_edge(v_left_middle, v_right_middle);
        self.create_edge(v_right_middle, v_bottom_right);
        self.create_edge(v_bottom_right, v_bottom_left);
        self.create_edge(v_bottom_left, v_left_middle);

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
            false,
            false,
            false,
        );
        area1.calculate_totrct_and_size(&self.verts, ui_settings);
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
            false,
            false,
            false,
        );
        area2.calculate_totrct_and_size(&self.verts, ui_settings);
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
            false,
            false,
            false,
        );
        area3.calculate_totrct_and_size(&self.verts, ui_settings);
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
    pub fn update_all_areas_rcti(&mut self, ui_settings: &UISettings) {
        let is_single_area = self.areas.len() == 1;

        // 收集需要更新的区域keys（避免借用冲突）
        let area_keys: Vec<ScrAreaKey> = self.areas.keys().collect();

        for area_key in area_keys {
            if let Some(area) = self.areas.get_mut(area_key) {
                if is_single_area {
                    area.is_single_area = is_single_area;
                }
                area.calculate_totrct_and_size(&self.verts, ui_settings);
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
                area.v_bottom_left(),
                area.v_top_left(),
                area.v_top_right(),
                area.v_bottom_right(),
            ])
        } else {
            None
        }
    }
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
        if let Some((existed_v_key, _existed_v)) = self
            .verts
            .iter()
            .find(|(_v_key, v)| v.vec.x == x && v.vec.y == y)
        {
            return existed_v_key;
        }
        self.create_vertex(x, y)
    }

    /// 添加新边到现有几何结构中
    pub fn add_edge_to_geometry(&mut self, v1: ScrVertKey, v2: ScrVertKey) -> ScrEdgeKey {
        // 检查是否已存在相同的边
        if let Some(existed_edge_key) = self.find_edge_between_vertices(v1, v2) {
            // 如果边已存在，直接返回现有的边
            return existed_edge_key;
        }
        self.create_edge(v1, v2)
    }
    /// 将所有顶点按窗口边缘吸附规则校正
    pub fn enforce_window_edge_snap(&mut self, window_rect: &Rcti, ui_settings: &UISettings) {
        let d2 = ui_settings.get_edge_filter_boundary().round() as u32; // 2x
        let d1 = ui_settings.get_edge_snap_target().round() as u32; // 1x

        let top_line = window_rect.ymin.saturating_add(d1);
        let bottom_line = window_rect.ymax.saturating_sub(d1);
        let left_line = window_rect.xmin.saturating_add(d1);
        let right_line = window_rect.xmax.saturating_sub(d1);

        let top_thresh = window_rect.ymin.saturating_add(d2);
        let bottom_thresh = window_rect.ymax.saturating_sub(d2);
        let left_thresh = window_rect.xmin.saturating_add(d2);
        let right_thresh = window_rect.xmax.saturating_sub(d2);

        for (_, v) in &mut self.verts {
            // 上
            if v.vec.y < top_thresh {
                v.vec.y = top_line;
            }
            // 右
            if v.vec.x > right_thresh {
                v.vec.x = right_line;
            }
            // 下
            if v.vec.y > bottom_thresh {
                v.vec.y = bottom_line;
            }
            // 左
            if v.vec.x < left_thresh {
                v.vec.x = left_line;
            }
        }
    }
    pub fn new() -> Self {
        ScrAreaMap {
            verts: SlotMap::with_key(),
            edges: SlotMap::with_key(),
            areas: SlotMap::with_key(),
        }
    }
    pub fn set_area_vert(
        &mut self,
        area_key: ScrAreaKey,
        vert_pos_and_key: VertPosition,
    ) -> Result<()> {
        let area = self
            .areas
            .get_mut(area_key)
            .context(format!("不存在的area: {:?}", area_key))?;
        area.set_vert(vert_pos_and_key)
    }
    /// 计算指定角落的绝对坐标位置
    fn calculate_corner_position(
        area: &ScrArea,
        corner: ActionZoneCorner,
        offset_x: f64,
        offset_y: f64,
        actionzone_side_length: f64,
    ) -> Vec2f64 {
        let base_pos = match corner {
            ActionZoneCorner::TopLeft => Vec2f64 {
                x: area.tot_rct().xmin as f64,
                y: area.tot_rct().ymin as f64,
            },
            ActionZoneCorner::TopRight => Vec2f64 {
                x: (area.tot_rct().xmax as f64 - actionzone_side_length),
                y: area.tot_rct().ymin as f64,
            },
            ActionZoneCorner::BottomLeft => Vec2f64 {
                x: area.tot_rct().xmin as f64,
                y: (area.tot_rct().ymax as f64 - actionzone_side_length),
            },
            ActionZoneCorner::BottomRight => Vec2f64 {
                x: (area.tot_rct().xmax as f64 - actionzone_side_length),
                y: (area.tot_rct().ymax as f64 - actionzone_side_length),
            },
        };

        Vec2f64 {
            x: base_pos.x + offset_x,
            y: base_pos.y + offset_y,
        }
    }

    pub fn find_area_with_actionzone(
        &self,
        area_key: ScrAreaKey,
        corner: ActionZoneCorner,
        offset_x: f64,
        offset_y: f64,
        actionzone_side_length: f64,
    ) -> Option<ScrAreaKey> {
        let area = self.areas.get(area_key).unwrap();
        let mouse_pos = Self::calculate_corner_position(
            area,
            corner,
            offset_x,
            offset_y,
            actionzone_side_length,
        );
        // println!("当前计算出的鼠标绝对位置: {:#?}", mouse_pos);

        self.find_area_at_position(mouse_pos.x as u32, mouse_pos.y as u32)
    }

    /// 将偏移量转换为相对于区域左上角的偏移量
    pub fn convert_corner_offset_to_relative_offset(
        &self,
        area_key: ScrAreaKey,
        corner: ActionZoneCorner,
        offset_x: f64,
        offset_y: f64,
        actionzone_side_length: f64,
    ) -> (f64, f64) {
        let area = self.areas.get(area_key).unwrap();
        let absolute_pos = Self::calculate_corner_position(
            area,
            corner,
            offset_x,
            offset_y,
            actionzone_side_length,
        );

        (
            absolute_pos.x - area.tot_rct().xmin as f64,
            absolute_pos.y - area.tot_rct().ymin as f64,
        )
    }
    /// 根据物理像素坐标查找鼠标所在的区域
    pub fn find_area_at_position(&self, x: u32, y: u32) -> Option<ScrAreaKey> {
        for (area_key, area) in &self.areas {
            let (totrct, _, _) = area.geometry();

            // 使用物理像素坐标进行比较
            if x >= totrct.xmin && x <= totrct.xmax && y >= totrct.ymin && y <= totrct.ymax {
                return Some(area_key);
            }
        }
        None
    }

    /// 根据区域类型查找区域（类似Blender的spacetype过滤）
    pub fn find_area_at_position_with_type(
        &self,
        x: f32,
        y: f32,
        area_type: Option<AreaType>,
    ) -> Option<ScrAreaKey> {
        for (area_key, area) in &self.areas {
            let (totrct, _, _) = area.geometry();

            if x >= totrct.xmin as f32
                && x <= totrct.xmax as f32
                && y >= totrct.ymin as f32
                && y <= totrct.ymax as f32
            {
                // 如果指定了类型，检查类型是否匹配
                if let Some(target_type) = &area_type {
                    if &area.area_type == target_type {
                        return Some(area_key);
                    }
                } else {
                    // 没有指定类型，返回第一个匹配的区域
                    return Some(area_key);
                }
            }
        }
        None
    }
    pub fn scale_vertices_to_window(
        &mut self,
        old_rect: &Rcti,
        new_rect: &Rcti,
        ui_setting: &UISettings,
    ) {
        // 1. 计算缩放比例
        let scale_x =
            (new_rect.xmax - new_rect.xmin) as f64 / (old_rect.xmax - old_rect.xmin) as f64;
        let scale_y =
            (new_rect.ymax - new_rect.ymin) as f64 / (old_rect.ymax - old_rect.ymin) as f64;

        // 2. 缩放所有顶点
        for (_, vertex) in &mut self.verts {
            // 计算相对位置
            let rel_x = (vertex.vec.x - old_rect.xmin) as f64;
            let rel_y = (vertex.vec.y - old_rect.ymin) as f64;

            // 应用缩放
            vertex.vec.x = new_rect.xmin + (rel_x * scale_x).round() as u32;
            vertex.vec.y = new_rect.ymin + (rel_y * scale_y).round() as u32;

            // 边界限制
            vertex.vec.x = vertex.vec.x.clamp(new_rect.xmin, new_rect.xmax);
            vertex.vec.y = vertex.vec.y.clamp(new_rect.ymin, new_rect.ymax);
        }

        // 3. 重新计算所有区域的几何
        self.update_all_areas_rcti(ui_setting);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::types::UISettings;

    #[test]
    fn test_create_edge_simple() {
        let mut screen_map = ScrAreaMap::new();

        // 先创建两个顶点
        let v1 = screen_map.add_vertex_to_geometry(0, 0);
        let v2 = screen_map.add_vertex_to_geometry(100, 100);

        // 创建连接这两个顶点的边
        let edge_key = screen_map.add_edge_to_geometry(v1, v2);

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
    fn test_add_multiple_edges() {
        let mut screen_map = ScrAreaMap::new();

        // 创建顶点
        let v1 = screen_map.add_vertex_to_geometry(0, 0);
        let v2 = screen_map.add_vertex_to_geometry(100, 0);
        let v3 = screen_map.add_vertex_to_geometry(100, 100);

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

        let v1 = screen_map.add_vertex_to_geometry(0, 0);
        let v2 = screen_map.add_vertex_to_geometry(100, 0);
        let v3 = screen_map.add_vertex_to_geometry(100, 100);

        let e1 = screen_map.add_edge_to_geometry(v1, v2);
        let e2 = screen_map.add_edge_to_geometry(v1, v3);

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
    fn test_empty_map() {
        let screen_map = ScrAreaMap::new();
        assert_eq!(screen_map.verts.len(), 0);
        assert_eq!(screen_map.edges.len(), 0);
        assert_eq!(screen_map.areas.len(), 0);
    }
}
