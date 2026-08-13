use anyhow::Result;

use crate::{
    bridge::types::{SplitPreviewOverlayData, ThemeSettings},
    screen::{
        area::{ScrArea, ScrAreaKey},
        edge::ScrEdgeKey,
        map::ScrAreaMap,
        vertex::{ScrVertKey, VertPosition},
    },
};

impl ScrAreaMap {
    pub fn split_area_by_coordinates(
        &mut self,
        area_key: ScrAreaKey,
        split_coord: u32,
        is_horizontal_split: bool,
        theme_settings: &ThemeSettings,
    ) -> Result<ScrAreaKey> {
        self.split_area_by_coordinates_inner(
            area_key,
            split_coord,
            is_horizontal_split,
            theme_settings,
            true,
        )
    }

    /// 分割一个区域（拖拽入口）。
    ///
    /// `split_data` 的坐标基于 `tot_rct`（含边距收缩后的物理像素，来自
    /// `calculate_split_areas`）。`split_area_by_coordinates` 内部按**原始顶点
    /// 坐标**（raw，未收缩）建边/校验——两者差一个 margin。这里把 tot_rct
    /// 坐标换算回 raw 坐标，并改用与预览一致的 `min_area_size` 校验
    /// （`strict=false`），保证预览通过的分割执行时不被误拒绝。
    pub fn split_area(
        &mut self,
        area_key: ScrAreaKey,
        split_data: SplitPreviewOverlayData,
        theme_settings: &ThemeSettings,
    ) -> Result<ScrAreaKey> {
        let is_horizontal_split = split_data.width1 != split_data.width2;
        let margin = self.area_margin(area_key, theme_settings);
        let split_coord = if is_horizontal_split {
            split_data.x2 as u32
        } else {
            split_data.y2 as u32
        } + margin;
        self.split_area_by_coordinates_inner(
            area_key,
            split_coord,
            is_horizontal_split,
            theme_settings,
            false,
        )
    }

    /// 区域边距（物理像素）：`has_margin`（非全屏/非临时/非单 area）时才存在。
    /// 用于把 tot_rct 坐标换算回 raw 坐标。
    fn area_margin(&self, area_key: ScrAreaKey, theme_settings: &ThemeSettings) -> u32 {
        let area = self.areas.get(area_key).unwrap();
        if area.has_margin() {
            theme_settings.calculate_margin(false)
        } else {
            0
        }
    }

    fn split_area_by_coordinates_inner(
        &mut self,
        area_key: ScrAreaKey,
        split_coord: u32,
        is_horizontal_split: bool,
        theme_settings: &ThemeSettings,
        strict: bool,
    ) -> Result<ScrAreaKey> {
        let (
            vert_rect,
            v1,
            v2,
            v3,
            v4,
            area_type,
            is_temp_window,
            is_fullscreen,
            is_single_area,
            area_has_margin,
        ) = {
            let area = self
                .areas
                .get(area_key)
                .ok_or_else(|| anyhow::anyhow!("源区域 {:?} 不存在", area_key))?;
            // 使用原始几何信息（边距调整之前的）
            let vert_rect = area.geometry_rect(&self.verts);
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
                area.has_margin(),
            )
        };
        // 分割后两侧都必须 ≥ 阈值，否则拒绝分割。
        // - `strict`（join 内部 trim 调用）：用 `min + 2*margin`——收缩边距后
        //   两侧仍须 ≥ min，防止产生过小/负尺寸区域（calculate_totrct_and_size
        //   收缩后 ymax<ymin panic）。
        // - 非 strict（用户拖拽）：与预览 `calculate_split_areas` 的校验一致，
        //   用 `min_area_size`——预览已保证坐标合理，这里避免双重严格导致
        //   合法分割被误拒绝（用户报告的分割报错）。
        let min = theme_settings.get_area_min_size();
        let margin = if area_has_margin {
            theme_settings.calculate_margin(false)
        } else {
            0
        };
        let min_effective = if strict {
            min.saturating_add(margin.saturating_mul(2))
        } else {
            min
        };
        if is_horizontal_split {
            let w = vert_rect.width();
            let left = split_coord.saturating_sub(vert_rect.xmin);
            let right = w.saturating_sub(left);
            if left < min_effective || right < min_effective {
                return Err(anyhow::anyhow!(
                    "分割点 {} 产生过小区域（最小 {}，需两侧都满足）",
                    split_coord,
                    min_effective
                ));
            }
        } else {
            let h = vert_rect.height();
            let top = split_coord.saturating_sub(vert_rect.ymin);
            let bottom = h.saturating_sub(top);
            if top < min_effective || bottom < min_effective {
                return Err(anyhow::anyhow!(
                    "分割点 {} 产生过小区域（最小 {}，需两侧都满足）",
                    split_coord,
                    min_effective
                ));
            }
        }
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
            source_area.calculate_totrct_and_size(&self.verts, theme_settings);
            let new_area_mut = self.areas.get_mut(new_area_key).unwrap();
            new_area_mut.calculate_totrct_and_size(&self.verts, theme_settings);

            // 清理重复的边和顶点（参考Blender的实现）
            self.remove_duplicate_edges();
            self.remove_unused_edges();

            log::info!("水平拆分完成，创建了新区域: {:?}", new_area_key);
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
            source_area.calculate_totrct_and_size(&self.verts, theme_settings);
            let new_area_mut = self.areas.get_mut(new_area_key).unwrap();
            new_area_mut.calculate_totrct_and_size(&self.verts, theme_settings);

            // 清理重复的边和顶点（参考Blender的实现）
            self.remove_duplicate_edges();
            self.remove_unused_edges();

            log::info!("垂直拆分完成，创建了新区域: {:?}", new_area_key);
            Ok(new_area_key)
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
        theme_settings: &ThemeSettings,
    ) -> SplitPreviewOverlayData {
        let area = self.areas.get(area_key).unwrap();
        let (tot_rct, winx, winy) = area.layout_rect();

        // 定义最小区域尺寸（像素）
        let min_area_size = theme_settings.get_area_min_size();

        // 根据原始偏移量的绝对值大小决定分割方向
        let is_horizontal_split = original_offset_x.abs() > original_offset_y.abs();

        if is_horizontal_split {
            // 水平分割（垂直分割线）
            let split_x = tot_rct.xmin + relative_offset_x as u32;

            // WARN: TODO: 这里返回如果实现边拆分后需要合并距离近的顶点
            // 检查是否可以进行水平分割
            if winx < min_area_size * 2
                || split_x < tot_rct.xmin + min_area_size
                || split_x > tot_rct.xmax - min_area_size
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
            if winy < min_area_size * 2
                || split_y < tot_rct.ymin + min_area_size
                || split_y > tot_rct.ymax - min_area_size
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
    use crate::screen::area::AreaType;
    use crate::screen::area::ScrAreaKey;
    use crate::screen::map::ScrAreaMap;

    /// 构造单 area 布局：raw (0,0)-(800,600)，非 single（有 margin）。
    fn make_margin_area() -> (ScrAreaMap, ScrAreaKey) {
        let mut m = ScrAreaMap::new();
        let v1 = m.add_vertex_to_geometry(0, 0);
        let v2 = m.add_vertex_to_geometry(800, 0);
        let v3 = m.add_vertex_to_geometry(800, 600);
        let v4 = m.add_vertex_to_geometry(0, 600);
        m.add_edge_to_geometry(v1, v2);
        m.add_edge_to_geometry(v2, v3);
        m.add_edge_to_geometry(v3, v4);
        m.add_edge_to_geometry(v4, v1);
        let key = m.areas.insert(crate::screen::area::ScrArea::new(
            v1,
            v2,
            v3,
            v4,
            AreaType::LineEditor,
            false,
            false,
            false,
        ));
        (m, key)
    }

    #[test]
    fn split_at_margin_boundary_does_not_false_reject() {
        // Bug 回归：preview 用 tot_rct 坐标（收缩 margin），split_area_by_coordinates
        // 用 raw 坐标校验。分割线靠近边界时，margin 偏移导致"过小区域"误判。
        let theme = crate::bridge::types::ThemeSettings::new(1.0, 10.0, 10.0, 5.0, 58);
        let (mut m, key) = make_margin_area();
        {
            let area = m.areas.get_mut(key).unwrap();
            area.calculate_totrct_and_size(&m.verts, &theme);
        }

        // tot_rct = raw 收缩 margin。raw.xmin=0，margin=10 → tot.xmin=10
        let tot = m.areas.get(key).unwrap().tot_rct();

        // 模拟 preview：split_x 在 tot_rct 内使右侧恰好 = min（预览校验通过）
        let min: u32 = 58;
        let split_x_tot = tot.xmax.saturating_sub(min);
        let preview = m.calculate_split_areas(
            key,
            100.0,
            0.0,
            (split_x_tot - tot.xmin) as f64,
            0.0,
            &theme,
        );
        assert!(preview.width2 >= min as f32, "preview 右侧应 ≥ min");

        // 执行分割（split_area 内部把 tot_rct 转 raw，修复后两侧都满足）
        let result = m.split_area(key, preview, &theme);
        assert!(result.is_ok(), "合法分割不应被误拒绝: {:?}", result.err());
    }

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
