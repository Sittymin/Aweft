use anyhow::Result;

use crate::{
    screen::{
        area::{ScrArea, ScrAreaKey},
        edge::ScrEdgeKey,
        map::ScrAreaMap,
        vertex::{ScrVertKey, VertPosition},
    },
    types::{SplitPreviewOverlayData, UISettings},
};

impl ScrAreaMap {
    pub fn split_area_by_coordinates(
        &mut self,
        area_key: ScrAreaKey,
        split_coord: u32,
        is_horizontal_split: bool,
        ui_settings: &UISettings,
    ) -> Result<ScrAreaKey> {
        let (vert_rect, v1, v2, v3, v4, area_type, is_temp_window, is_fullscreen, is_single_area) = {
            let area = self
                .areas
                .get(area_key)
                .ok_or_else(|| anyhow::anyhow!("源区域 {:?} 不存在", area_key))?;
            // 使用原始几何信息（边距调整之前的）
            let vert_rect = area.get_raw_geometry(&self.verts);
            (
                vert_rect,
                area.v_top_left(),
                area.v_top_right(),
                area.v_bottom_right(),
                area.v_bottom_left(),
                area.area_type.clone(),
                area.is_temp_window,
                area.is_fullscreen,
                area.is_single_area,
            )
        };
        if is_horizontal_split {
            // 注意：split_coord 已经包含了 +1 的补偿，所以这里直接使用
            let new_vert_top = self.add_vertex_to_geometry(split_coord, vert_rect.ymin);
            let new_vert_bottom = self.add_vertex_to_geometry(split_coord, vert_rect.ymax);
            // 左部分
            self.add_edge_to_geometry(v1, new_vert_top);
            self.add_edge_to_geometry(v4, new_vert_bottom);
            // 右部分
            self.add_edge_to_geometry(new_vert_top, v2);
            self.add_edge_to_geometry(new_vert_bottom, v3);
            self.add_edge_to_geometry(new_vert_top, new_vert_bottom);

            let new_area = ScrArea::new(
                new_vert_top,
                v2,
                v3,
                new_vert_bottom,
                area_type,
                is_temp_window,
                is_fullscreen,
                is_single_area,
            );
            let new_area_key = self.areas.insert(new_area);

            // 修改原始区域
            self.set_area_vert(area_key, VertPosition::TopRight(new_vert_top))?;
            self.set_area_vert(area_key, VertPosition::BottomRight(new_vert_bottom))?;

            // 重新计算两个区域的几何属性
            let source_area = self.areas.get_mut(area_key).unwrap();
            source_area.calculate_totrct_and_size(&self.verts, ui_settings);
            let new_area_mut = self.areas.get_mut(new_area_key).unwrap();
            new_area_mut.calculate_totrct_and_size(&self.verts, ui_settings);

            // 清理重复的边和顶点（参考Blender的实现）
            self.remove_duplicate_edges();
            self.remove_unused_edges();

            println!("水平拆分完成，创建了新区域: {:?}", new_area_key);
            Ok(new_area_key)
        } else {
            let new_vert_left = self.add_vertex_to_geometry(vert_rect.xmin, split_coord);
            let new_vert_right = self.add_vertex_to_geometry(vert_rect.xmax, split_coord);

            // 上部分
            self.add_edge_to_geometry(v1, new_vert_left);
            self.add_edge_to_geometry(v2, new_vert_right);
            // 下部分
            self.add_edge_to_geometry(new_vert_left, v4);
            self.add_edge_to_geometry(new_vert_right, v3);
            self.add_edge_to_geometry(new_vert_left, new_vert_right);

            let new_area = ScrArea::new(
                new_vert_left,
                new_vert_right,
                v3,
                v4,
                area_type,
                is_temp_window,
                is_fullscreen,
                is_single_area,
            );
            let new_area_key = self.areas.insert(new_area);

            // 修改原始区域
            self.set_area_vert(area_key, VertPosition::BottomLeft(new_vert_left))?;
            self.set_area_vert(area_key, VertPosition::BottomRight(new_vert_right))?;

            // 重新计算两个区域的几何属性
            let source_area = self.areas.get_mut(area_key).unwrap();
            source_area.calculate_totrct_and_size(&self.verts, ui_settings);
            let new_area_mut = self.areas.get_mut(new_area_key).unwrap();
            new_area_mut.calculate_totrct_and_size(&self.verts, ui_settings);

            // 清理重复的边和顶点（参考Blender的实现）
            self.remove_duplicate_edges();
            self.remove_unused_edges();

            println!("垂直拆分完成，创建了新区域: {:?}", new_area_key);
            Ok(new_area_key)
        }
    }

    /// 分割一个区域
    pub fn split_area(
        &mut self,
        area_key: ScrAreaKey,
        split_data: SplitPreviewOverlayData,
        ui_settings: &UISettings,
    ) -> Result<ScrAreaKey> {
        let is_horizontal_split = split_data.width1 != split_data.width2;
        if is_horizontal_split {
            let split_x = split_data.x2 as u32;
            self.split_area_by_coordinates(area_key, split_x, is_horizontal_split, ui_settings)
        } else {
            let split_y = split_data.y2 as u32;
            self.split_area_by_coordinates(area_key, split_y, is_horizontal_split, ui_settings)
        }
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
            let new_vertex = self.add_vertex_to_geometry(x, y);

            // 移除原边
            self.edges.remove(edge_key);

            // 创建两条新边
            self.add_edge_to_geometry(v1, new_vertex);
            self.add_edge_to_geometry(new_vertex, v2);

            Some(new_vertex)
        } else {
            None
        }
    }
    pub fn calculate_split_areas(
        &self,
        area_key: ScrAreaKey,
        original_offset_x: f64,
        original_offset_y: f64,
        relative_offset_x: f64,
        relative_offset_y: f64,
    ) -> SplitPreviewOverlayData {
        let area = self.areas.get(area_key).unwrap();
        let (tot_rct, winx, winy) = area.geometry();

        // 定义最小区域尺寸（像素）
        const MIN_AREA_SIZE: u32 = 50;

        // 根据原始偏移量的绝对值大小决定分割方向
        let is_horizontal_split = original_offset_x.abs() > original_offset_y.abs();

        if is_horizontal_split {
            // 水平分割（垂直分割线）
            let split_x = tot_rct.xmin + relative_offset_x as u32;

            // WARN: TODO: 这里返回如果实现边拆分后需要合并距离近的顶点
            // 检查是否可以进行水平分割
            if winx < MIN_AREA_SIZE * 2
                || split_x < tot_rct.xmin + MIN_AREA_SIZE
                || split_x > tot_rct.xmax - MIN_AREA_SIZE
            {
                // 无法分割，返回完整的原始区域数据
                return SplitPreviewOverlayData {
                    x1: tot_rct.xmin as f32,
                    y1: tot_rct.ymin as f32,
                    width1: winx as f32,
                    height1: winy as f32,
                    x2: 0.0,
                    y2: 0.0,
                    width2: 0.0,
                    height2: 0.0,
                };
            }

            // 第一个区域（左侧）
            let x1 = tot_rct.xmin;
            let y1 = tot_rct.ymin;
            let width1 = split_x - tot_rct.xmin;
            let height1 = winy;

            // 第二个区域（右侧）
            let x2 = split_x;
            let y2 = tot_rct.ymin;
            // ymax - ymin 会少一个像素，所以加上(后续类似的相减一样)
            let width2 = tot_rct.xmax - split_x + 1;
            let height2 = winy;

            SplitPreviewOverlayData {
                x1: x1 as f32,
                y1: y1 as f32,
                width1: width1 as f32,
                height1: height1 as f32,
                x2: x2 as f32,
                y2: y2 as f32,
                width2: width2 as f32,
                height2: height2 as f32,
            }
        } else {
            // 垂直分割（水平分割线）
            let split_y = tot_rct.ymin + relative_offset_y as u32;

            // 检查是否可以进行垂直分割
            if winy < MIN_AREA_SIZE * 2
                || split_y < tot_rct.ymin + MIN_AREA_SIZE
                || split_y > tot_rct.ymax - MIN_AREA_SIZE
            {
                // 无法分割，返回完整的原始区域数据
                return SplitPreviewOverlayData {
                    x1: tot_rct.xmin as f32,
                    y1: tot_rct.ymin as f32,
                    width1: winx as f32,
                    height1: winy as f32,
                    x2: 0.0,
                    y2: 0.0,
                    width2: 0.0,
                    height2: 0.0,
                };
            }

            // 第一个区域（上方）
            let x1 = tot_rct.xmin;
            let y1 = tot_rct.ymin;
            let width1 = winx;
            let height1 = split_y - tot_rct.ymin;

            // 第二个区域（下方）
            let x2 = tot_rct.xmin;
            let y2 = split_y;
            let width2 = winx;
            let height2 = tot_rct.ymax - split_y + 1;

            SplitPreviewOverlayData {
                x1: x1 as f32,
                y1: y1 as f32,
                width1: width1 as f32,
                height1: height1 as f32,
                x2: x2 as f32,
                y2: y2 as f32,
                width2: width2 as f32,
                height2: height2 as f32,
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::screen::map::ScrAreaMap;

    #[test]
    fn test_insert_vertex_on_edge() {
        let mut screen_map = ScrAreaMap::new();

        let v1 = screen_map.add_vertex_to_geometry(0, 0);
        let v2 = screen_map.add_vertex_to_geometry(100, 0);
        let edge = screen_map.add_edge_to_geometry(v1, v2);

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
}
