use anyhow::{Context, Result, anyhow};

use crate::types::{
    AreaType, Rcti, ScrArea, ScrAreaKey, ScrAreaMap, ScrEdge, ScrEdgeKey, ScrVert, ScrVertKey,
    UISettings, Vec2u32, VertPosition,
};
use crate::{CloseAreaOverlayData, JoinHighlightOverlayData, SplitPreviewOverlayData};

/// 描述两个区域的邻接关系
#[derive(Debug, Clone, Copy)]
pub enum Adjacency {
    /// Source 在 Target 上方
    TopToBottom { top: Rcti, bottom: Rcti },
    /// Source 在 Target 下方
    BottomToTop { bottom: Rcti, top: Rcti },
    /// Source 在 Target 左方
    LeftToRight { left: Rcti, right: Rcti },
    /// Source 在 Target 右方
    RightToLeft { right: Rcti, left: Rcti },
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

    /// 辅助函数：检测邻接关系
    fn detect_adjacency(
        &self,
        source_rcti: Rcti,
        target_rcti: Rcti,
        min_overlap: u32,
    ) -> Option<Adjacency> {
        // 安全减法
        let overlap_x = (source_rcti.xmax.min(target_rcti.xmax))
            .saturating_sub(source_rcti.xmin.max(target_rcti.xmin));
        let overlap_y = (source_rcti.ymax.min(target_rcti.ymax))
            .saturating_sub(source_rcti.ymin.max(target_rcti.ymin));

        // ░━━━━━━━━░
        // ░ source ░
        // ░━━━━━━━━░
        // ░ target ░
        // ░━━━━━━━━░
        if source_rcti.ymax == target_rcti.ymin && overlap_x >= min_overlap {
            Some(Adjacency::TopToBottom {
                top: source_rcti,
                bottom: target_rcti,
            })
        }
        // ░━━━━━━━━░
        // ░ target ░
        // ░━━━━━━━━░
        // ░ source ░
        // ░━━━━━━━━░
        else if source_rcti.ymin == target_rcti.ymax && overlap_x >= min_overlap {
            Some(Adjacency::BottomToTop {
                bottom: source_rcti,
                top: target_rcti,
            })
        }
        // ░░░░░░░░░░░░░░░░░░░
        // ┃ source ┃ target ┃
        // ░░░░░░░░░░░░░░░░░░░
        else if source_rcti.xmax == target_rcti.xmin && overlap_y >= min_overlap {
            Some(Adjacency::LeftToRight {
                left: source_rcti,
                right: target_rcti,
            })
        }
        // ░░░░░░░░░░░░░░░░░░░
        // ┃ target ┃ source ┃
        // ░░░░░░░░░░░░░░░░░░░
        else if source_rcti.xmin == target_rcti.xmax && overlap_y >= min_overlap {
            Some(Adjacency::RightToLeft {
                right: source_rcti,
                left: target_rcti,
            })
        } else {
            None
        }
    }

    pub fn preview_join_adjacent_areas(
        &self,
        source_area_key: ScrAreaKey,
        adjacency: Adjacency,
        ui_settings: &UISettings,
    ) -> Result<(JoinHighlightOverlayData, Vec<CloseAreaOverlayData>)> {
        let (source_area_rcti, source_area_width, source_area_height) = self
            .areas
            .get(source_area_key)
            .context("找不到Area")?
            .geometry();
        let mut join_highlight_overlay_data = JoinHighlightOverlayData {
            height: source_area_height as f32,
            width: source_area_width as f32,
            x: source_area_rcti.xmin as f32,
            y: source_area_rcti.ymin as f32,
        };
        // 产生的 close 区域
        let mut close_areas_overlay_data: Vec<CloseAreaOverlayData> = Vec::new();
        match adjacency {
            Adjacency::TopToBottom { top, bottom } => {
                join_highlight_overlay_data.height += bottom.height() as f32;
                //   ┏━━━━━━░
                //   ┃source░
                // ┏━┻━━━━━━░
                // ┃        ░
                // ┃  target░
                // ┃        ░
                // ┗━━━━━━━━░
                if top.xmin > bottom.xmin {
                    ();
                }
                // ┏━━━━━━━━░
                // ┃        ░
                // ┃  source░
                // ┃        ░
                // ┗━┳━━━━━━░
                //   ┃target░
                //   ┗━━━━━━░
                else if top.xmin < bottom.xmin {
                    let close_width = bottom.xmin - top.xmin + 1;
                    let close_height = top.height();
                    close_areas_overlay_data.push(CloseAreaOverlayData {
                        x: top.xmin as f32,
                        y: top.ymin as f32,
                        width: close_width as f32,
                        height: close_height as f32,
                    });
                    join_highlight_overlay_data.x = bottom.xmin as f32;
                    join_highlight_overlay_data.width -= close_width as f32;
                }

                // ░━━━━━━┓
                // ░source┃
                // ░━━━━━━┻━┓
                // ░        ┃
                // ░target  ┃
                // ░        ┃
                // ░━━━━━━━━┛
                if top.xmax < bottom.xmax {
                    ();
                }
                // ░━━━━━━━━┓
                // ░        ┃
                // ░source  ┃
                // ░        ┃
                // ░━━━━━━┳━┛
                // ░target┃
                // ░━━━━━━┛
                else if top.xmax > bottom.xmax {
                    let close_width = top.xmax - bottom.xmax + 1;
                    let close_height = top.height();
                    close_areas_overlay_data.push(CloseAreaOverlayData {
                        x: bottom.xmax as f32,
                        y: top.ymin as f32,
                        width: close_width as f32,
                        height: close_height as f32,
                    });
                    join_highlight_overlay_data.width -= close_width as f32;
                }
            }
            Adjacency::BottomToTop { bottom, top } => {
                join_highlight_overlay_data.y = top.ymin as f32;
                join_highlight_overlay_data.height += top.height() as f32;
                // ┏━━━━━━━━░
                // ┃        ░
                // ┃  target░
                // ┃        ░
                // ┗━┳━━━━━━░
                //   ┃source░
                //   ┗━━━━━━░
                if bottom.xmin > top.xmin {
                    ();
                }
                //   ┏━━━━━━░
                //   ┃target░
                // ┏━┻━━━━━━░
                // ┃        ░
                // ┃  source░
                // ┃        ░
                // ┗━━━━━━━━░
                else if bottom.xmin < top.xmin {
                    let close_width = top.xmin - bottom.xmin + 1;
                    let close_height = bottom.height();
                    close_areas_overlay_data.push(CloseAreaOverlayData {
                        x: bottom.xmin as f32,
                        y: bottom.ymin as f32,
                        width: close_width as f32,
                        height: close_height as f32,
                    });
                    join_highlight_overlay_data.x = top.xmin as f32;
                    join_highlight_overlay_data.width -= close_width as f32;
                }

                // ░━━━━━━━━┓
                // ░        ┃
                // ░target  ┃
                // ░        ┃
                // ░━━━━━━┳━┛
                // ░source┃
                // ░━━━━━━┛
                if bottom.xmax < top.xmax {
                    ();
                }
                // ░━━━━━━┓
                // ░target┃
                // ░━━━━━━┻━┓
                // ░        ┃
                // ░source  ┃
                // ░        ┃
                // ░━━━━━━━━┛
                else if bottom.xmax > top.xmax {
                    let close_width = bottom.xmax - top.xmax + 1;
                    let close_height = bottom.height();
                    close_areas_overlay_data.push(CloseAreaOverlayData {
                        x: top.xmax as f32,
                        y: bottom.ymin as f32,
                        width: close_width as f32,
                        height: close_height as f32,
                    });
                    join_highlight_overlay_data.width -= close_width as f32;
                }
            }
            Adjacency::LeftToRight { left, right } => {
                join_highlight_overlay_data.width += right.width() as f32;
                //        ┏━━━━━━━━━━┓
                // ┏━━━━━━┫          ┃
                // ┃source┃  target  ┃
                // ░░░░░░░░░░░░░░░░░░░
                if left.ymin > right.ymin {
                    ();
                }
                // ┏━━━━━━━━━━┓
                // ┃          ┣━━━━━━┓
                // ┃  source  ┃target┃
                // ░░░░░░░░░░░░░░░░░░░
                else if left.ymin < right.ymin {
                    let close_width = left.width();
                    let close_height = right.ymin - left.ymin + 1;
                    close_areas_overlay_data.push(CloseAreaOverlayData {
                        x: left.xmin as f32,
                        y: left.ymin as f32,
                        width: close_width as f32,
                        height: close_height as f32,
                    });
                    join_highlight_overlay_data.y = right.ymin as f32;
                    join_highlight_overlay_data.height -= close_height as f32;
                    println!("触发：{:#?}", left)
                }

                // ░░░░░░░░░░░░░░░░░░░
                // ┃source┃  target  ┃
                // ┗━━━━━━┫          ┃
                //        ┗━━━━━━━━━━┛
                if left.ymax < right.ymax {
                    ();
                }
                // ░░░░░░░░░░░░░░░░░░░
                // ┃  source  ┃target┃
                // ┃          ┣━━━━━━┛
                // ┗━━━━━━━━━━┛
                else if left.ymax > right.ymax {
                    let close_width = left.width();
                    let close_height = left.ymax - right.ymax + 1;
                    close_areas_overlay_data.push(CloseAreaOverlayData {
                        x: left.xmin as f32,
                        y: right.ymax as f32,
                        width: close_width as f32,
                        height: close_height as f32,
                    });
                    join_highlight_overlay_data.height -= close_height as f32;
                }
            }
            Adjacency::RightToLeft { right, left } => {
                join_highlight_overlay_data.x = left.xmin as f32;
                join_highlight_overlay_data.width += left.width() as f32;
                // ┏━━━━━━━━━━┓
                // ┃          ┣━━━━━━┓
                // ┃  target  ┃source┃
                // ░░░░░░░░░░░░░░░░░░░
                if right.ymin > left.ymin {
                    ();
                }
                //        ┏━━━━━━━━━━┓
                // ┏━━━━━━┫          ┃
                // ┃target┃  source  ┃
                // ░░░░░░░░░░░░░░░░░░░
                else if right.ymin < left.ymin {
                    let close_width = right.width();
                    let close_height = left.ymin - right.ymin + 1;
                    close_areas_overlay_data.push(CloseAreaOverlayData {
                        x: right.xmin as f32,
                        y: right.ymin as f32,
                        width: close_width as f32,
                        height: close_height as f32,
                    });
                    join_highlight_overlay_data.y = left.ymin as f32;
                    join_highlight_overlay_data.height -= close_height as f32;
                }

                // ░░░░░░░░░░░░░░░░░░░
                // ┃  target  ┃source┃
                // ┃          ┣━━━━━━┛
                // ┗━━━━━━━━━━┛
                if right.ymax < left.ymax {
                    ();
                }
                // ░░░░░░░░░░░░░░░░░░░
                // ┃target┃  source  ┃
                // ┗━━━━━━┫          ┃
                //        ┗━━━━━━━━━━┛
                else if right.ymax > left.ymax {
                    let close_width = right.width();
                    let close_height = right.ymax - left.ymax + 1;
                    close_areas_overlay_data.push(CloseAreaOverlayData {
                        x: right.xmin as f32,
                        y: left.ymax as f32,
                        width: close_width as f32,
                        height: close_height as f32,
                    });
                    join_highlight_overlay_data.height -= close_height as f32;
                }
            }
        };
        Ok((join_highlight_overlay_data, close_areas_overlay_data))
    }

    /// 合并两个相邻的区域
    /// 这个函数实现了Blender中screen_area_join_aligned的逻辑
    pub fn apply_join_adjacent_areas(
        &mut self,
        source_area_key: ScrAreaKey,
        target_area_key: ScrAreaKey,
        adjacency: Adjacency,
        ui_settings: &UISettings,
    ) -> Result<()> {
        if source_area_key == target_area_key {
            return Ok(());
        }

        // 产生的空白区域
        let mut new_empty_areas = Vec::new();
        let mut merged_area_key: ScrAreaKey = target_area_key;
        let mut working_source_area_key: ScrAreaKey = source_area_key;

        match adjacency {
            Adjacency::TopToBottom { top, bottom } => {
                //   ┏━━━━━━░
                //   ┃source░
                // ┏━┻━━━━━━░
                // ┃        ░
                // ┃  target░
                // ┃        ░
                // ┗━━━━━━━━░
                if top.xmin > bottom.xmin {
                    merged_area_key = self.split_area_by_coordinates(
                        merged_area_key,
                        top.xmin,
                        true,
                        ui_settings,
                    )?;
                }
                // ┏━━━━━━━━░
                // ┃        ░
                // ┃  source░
                // ┃        ░
                // ┗━┳━━━━━━░
                //   ┃target░
                //   ┗━━━━━━░
                else if top.xmin < bottom.xmin {
                    new_empty_areas.push(working_source_area_key.clone());
                    working_source_area_key = self.split_area_by_coordinates(
                        working_source_area_key,
                        bottom.xmin,
                        true,
                        ui_settings,
                    )?;
                }

                // ░━━━━━━┓
                // ░source┃
                // ░━━━━━━┻━┓
                // ░        ┃
                // ░target  ┃
                // ░        ┃
                // ░━━━━━━━━┛
                if top.xmax < bottom.xmax {
                    self.split_area_by_coordinates(merged_area_key, top.xmax, true, ui_settings)?;
                }
                // ░━━━━━━━━┓
                // ░        ┃
                // ░source  ┃
                // ░        ┃
                // ░━━━━━━┳━┛
                // ░target┃
                // ░━━━━━━┛
                else if top.xmax > bottom.xmax {
                    new_empty_areas.push(self.split_area_by_coordinates(
                        working_source_area_key,
                        bottom.xmax,
                        true,
                        ui_settings,
                    )?)
                }

                // 开始合并对齐的面板
                self.set_area_vert(
                    working_source_area_key,
                    VertPosition::BottomLeft(
                        self.areas
                            .get(merged_area_key)
                            .context(format!("合并区域不存在: {:?}", merged_area_key))?
                            .v_bottom_left(),
                    ),
                )?;
                self.set_area_vert(
                    working_source_area_key,
                    VertPosition::BottomRight(
                        self.areas
                            .get(merged_area_key)
                            .context(format!("合并区域不存在: {:?}", merged_area_key))?
                            .v_bottom_right(),
                    ),
                )?;
                self.add_edge_to_geometry(
                    self.areas
                        .get(working_source_area_key)
                        .context("不存在的source_area_key")?
                        .v_top_left(),
                    self.areas
                        .get(working_source_area_key)
                        .context("不存在的source_area_key")?
                        .v_bottom_left(),
                );
                self.add_edge_to_geometry(
                    self.areas
                        .get(working_source_area_key)
                        .context("不存在的source_area_key")?
                        .v_top_right(),
                    self.areas
                        .get(working_source_area_key)
                        .context("不存在的source_area_key")?
                        .v_bottom_right(),
                );
            }
            Adjacency::BottomToTop { bottom, top } => {
                // ┏━━━━━━━━░
                // ┃        ░
                // ┃  target░
                // ┃        ░
                // ┗━┳━━━━━━░
                //   ┃source░
                //   ┗━━━━━━░
                if bottom.xmin > top.xmin {
                    merged_area_key = self.split_area_by_coordinates(
                        target_area_key,
                        bottom.xmin,
                        true,
                        ui_settings,
                    )?;
                }
                //   ┏━━━━━━░
                //   ┃target░
                // ┏━┻━━━━━━░
                // ┃        ░
                // ┃  source░
                // ┃        ░
                // ┗━━━━━━━━░
                else if bottom.xmin < top.xmin {
                    new_empty_areas.push(working_source_area_key.clone());
                    working_source_area_key = self.split_area_by_coordinates(
                        working_source_area_key,
                        top.xmin,
                        true,
                        ui_settings,
                    )?
                }

                // ░━━━━━━━━┓
                // ░        ┃
                // ░target  ┃
                // ░        ┃
                // ░━━━━━━┳━┛
                // ░source┃
                // ░━━━━━━┛
                if bottom.xmax < top.xmax {
                    self.split_area_by_coordinates(
                        merged_area_key,
                        bottom.xmax,
                        true,
                        ui_settings,
                    )?;
                }
                // ░━━━━━━┓
                // ░target┃
                // ░━━━━━━┻━┓
                // ░        ┃
                // ░source  ┃
                // ░        ┃
                // ░━━━━━━━━┛
                else if bottom.xmax > top.xmax {
                    new_empty_areas.push(self.split_area_by_coordinates(
                        working_source_area_key,
                        top.xmax,
                        true,
                        ui_settings,
                    )?)
                }

                // 开始合并对齐的面板
                self.set_area_vert(
                    working_source_area_key,
                    VertPosition::TopLeft(
                        self.areas
                            .get(merged_area_key)
                            .context(format!("合并区域不存在: {:?}", merged_area_key))?
                            .v_top_left(),
                    ),
                )?;
                self.set_area_vert(
                    working_source_area_key,
                    VertPosition::TopRight(
                        self.areas
                            .get(merged_area_key)
                            .context(format!("合并区域不存在: {:?}", merged_area_key))?
                            .v_top_right(),
                    ),
                )?;
                self.add_edge_to_geometry(
                    self.areas
                        .get(working_source_area_key)
                        .context("不存在的source_area_key")?
                        .v_bottom_left(),
                    self.areas
                        .get(working_source_area_key)
                        .context("不存在的source_area_key")?
                        .v_top_left(),
                );
                self.add_edge_to_geometry(
                    self.areas
                        .get(working_source_area_key)
                        .context("不存在的source_area_key")?
                        .v_top_right(),
                    self.areas
                        .get(working_source_area_key)
                        .context("不存在的source_area_key")?
                        .v_bottom_right(),
                );
            }
            Adjacency::LeftToRight { left, right } => {
                //        ┏━━━━━━━━━━┓
                // ┏━━━━━━┫          ┃
                // ┃source┃  target  ┃
                // ░░░░░░░░░░░░░░░░░░░
                if left.ymin > right.ymin {
                    merged_area_key = self.split_area_by_coordinates(
                        merged_area_key,
                        left.ymin,
                        false,
                        ui_settings,
                    )?;
                }
                // ┏━━━━━━━━━━┓
                // ┃          ┣━━━━━━┓
                // ┃  source  ┃target┃
                // ░░░░░░░░░░░░░░░░░░░
                else if left.ymin < right.ymin {
                    new_empty_areas.push(working_source_area_key.clone());
                    working_source_area_key = self.split_area_by_coordinates(
                        working_source_area_key,
                        right.ymin,
                        false,
                        ui_settings,
                    )?;
                }

                // ░░░░░░░░░░░░░░░░░░░
                // ┃source┃  target  ┃
                // ┗━━━━━━┫          ┃
                //        ┗━━━━━━━━━━┛
                if left.ymax < right.ymax {
                    self.split_area_by_coordinates(merged_area_key, left.ymax, false, ui_settings)?;
                }
                // ░░░░░░░░░░░░░░░░░░░
                // ┃  source  ┃target┃
                // ┃          ┣━━━━━━┛
                // ┗━━━━━━━━━━┛
                else if left.ymax > right.ymax {
                    new_empty_areas.push(self.split_area_by_coordinates(
                        working_source_area_key,
                        right.ymax,
                        false,
                        ui_settings,
                    )?)
                }
                // 开始合并对齐的面板
                self.set_area_vert(
                    working_source_area_key,
                    VertPosition::TopRight(
                        self.areas
                            .get(merged_area_key)
                            .context("不存在合并area")?
                            .v_top_right(),
                    ),
                )?;
                self.set_area_vert(
                    working_source_area_key,
                    VertPosition::BottomRight(
                        self.areas
                            .get(merged_area_key)
                            .context("不存在合并area")?
                            .v_bottom_right(),
                    ),
                )?;
                self.add_edge_to_geometry(
                    self.areas
                        .get(working_source_area_key)
                        .context(format!("源区域不存在: {:?}", working_source_area_key))?
                        .v_top_left(),
                    self.areas
                        .get(working_source_area_key)
                        .context(format!("源区域不存在: {:?}", working_source_area_key))?
                        .v_top_right(),
                );
                self.add_edge_to_geometry(
                    self.areas
                        .get(source_area_key)
                        .context(format!("源区域不存在: {:?}", working_source_area_key))?
                        .v_bottom_left(),
                    self.areas
                        .get(source_area_key)
                        .context(format!("源区域不存在: {:?}", working_source_area_key))?
                        .v_bottom_right(),
                );
            }
            Adjacency::RightToLeft { right, left } => {
                // ┏━━━━━━━━━━┓
                // ┃          ┣━━━━━━┓
                // ┃  target  ┃source┃
                // ░░░░░░░░░░░░░░░░░░░
                if right.ymin > left.ymin {
                    merged_area_key = self.split_area_by_coordinates(
                        merged_area_key,
                        right.ymin,
                        false,
                        ui_settings,
                    )?;
                }
                //        ┏━━━━━━━━━━┓
                // ┏━━━━━━┫          ┃
                // ┃target┃  source  ┃
                // ░░░░░░░░░░░░░░░░░░░
                else if right.ymin < left.ymin {
                    new_empty_areas.push(working_source_area_key.clone());
                    working_source_area_key = self.split_area_by_coordinates(
                        working_source_area_key,
                        left.ymin,
                        false,
                        ui_settings,
                    )?;
                }

                // ░░░░░░░░░░░░░░░░░░░
                // ┃  target  ┃source┃
                // ┃          ┣━━━━━━┛
                // ┗━━━━━━━━━━┛
                if right.ymax < left.ymax {
                    self.split_area_by_coordinates(
                        merged_area_key,
                        right.ymax,
                        false,
                        ui_settings,
                    )?;
                }
                // ░░░░░░░░░░░░░░░░░░░
                // ┃target┃  source  ┃
                // ┗━━━━━━┫          ┃
                //        ┗━━━━━━━━━━┛
                else if right.ymax > left.ymax {
                    new_empty_areas.push(self.split_area_by_coordinates(
                        working_source_area_key,
                        left.ymax,
                        false,
                        ui_settings,
                    )?)
                }
                // 开始合并对齐的面板
                self.set_area_vert(
                    working_source_area_key,
                    VertPosition::TopLeft(
                        self.areas
                            .get(merged_area_key)
                            .context("不存在合并area")?
                            .v_top_left(),
                    ),
                )?;
                self.set_area_vert(
                    working_source_area_key,
                    VertPosition::BottomLeft(
                        self.areas
                            .get(merged_area_key)
                            .context("不存在合并area")?
                            .v_bottom_left(),
                    ),
                )?;
                self.add_edge_to_geometry(
                    self.areas
                        .get(working_source_area_key)
                        .context(format!("源区域不存在: {:?}", working_source_area_key))?
                        .v_top_left(),
                    self.areas
                        .get(working_source_area_key)
                        .context(format!("源区域不存在: {:?}", working_source_area_key))?
                        .v_top_right(),
                );
                self.add_edge_to_geometry(
                    self.areas
                        .get(working_source_area_key)
                        .context(format!("源区域不存在: {:?}", working_source_area_key))?
                        .v_bottom_right(),
                    self.areas
                        .get(working_source_area_key)
                        .context(format!("源区域不存在: {:?}", working_source_area_key))?
                        .v_bottom_left(),
                );
            }
        }

        // 删除目标区域
        self.areas.remove(merged_area_key);

        // 清理重复的边和顶点
        self.remove_duplicate_edges();
        self.remove_unused_edges();

        // 递归清理产生的empty区域
        if !new_empty_areas.is_empty() {
            self.recursive_fill_close_areas(new_empty_areas, ui_settings)?;
        }

        // 重新计算所有区域的几何数据
        self.update_all_areas_rcti(ui_settings);

        Ok(())
    }

    fn fill_close_area(
        &mut self,
        close_area_key: ScrAreaKey,
        ui_settings: &UISettings,
    ) -> Result<Vec<ScrAreaKey>> {
        let best_neighbor = self
            .find_best_neighbor_for_close_area(
                self.areas.get(close_area_key).context("不存在的Area")?,
            )
            .context(format!(
                "竟然找不到其他任何可能的填充邻居: {:#?}",
                close_area_key
            ))?;

        // 是合并相似，但是不管大合小还是小合大分割出来的都是空白区域
        if best_neighbor == close_area_key {
            panic!(
                "Error: best_neighbor cannot be the close_area_key. INFO: {:#?}",
                close_area_key
            );
        }

        // 最小重叠要求 (TODO: calculate_split_areas 当中一致，需要之后合并)
        #[cfg(test)]
        const MIN_AREA_SIZE: u32 = 1; // 测试时使用更小的值便于口算
        #[cfg(not(test))]
        const MIN_AREA_SIZE: u32 = 50; // 生产环境使用正常值

        let neighbor_rect = self
            .areas
            .get(best_neighbor)
            .context(format!("源区域不存在: {:?}", best_neighbor))?
            .get_raw_geometry(&self.verts);

        let close_rect = self
            .areas
            .get(close_area_key)
            .context(format!("目标区域不存在: {:?}", close_area_key))?
            .get_raw_geometry(&self.verts);

        println!(
            "邻居区域坐标: ({}, {}) 到 ({}, {})",
            neighbor_rect.xmin, neighbor_rect.ymin, neighbor_rect.xmax, neighbor_rect.ymax
        );
        println!(
            "empty区域坐标: ({}, {}) 到 ({}, {})",
            close_rect.xmin, close_rect.ymin, close_rect.xmax, close_rect.ymax
        );

        // 检测相邻关系
        let adjacency = self
            .detect_adjacency(neighbor_rect, close_rect, MIN_AREA_SIZE)
            .context("Area 坐标不一致：没有检测到任何相邻关系")?;

        // 产生的空白区域
        let mut new_empty_areas = Vec::new();
        let mut merged_area_key: ScrAreaKey = close_area_key;
        let mut working_neighbor_area_key: ScrAreaKey = best_neighbor;

        match adjacency {
            Adjacency::TopToBottom {
                top: neighbor_rect,
                bottom: close_rect,
            } => {
                //   ┏━━━━━━━━░
                //   ┃neighbor░
                // ┏━┻━━━━━━━━░
                // ┃    close ░
                // ┗━━━━━━━━━━░
                if neighbor_rect.xmin > close_rect.xmin {
                    new_empty_areas.push(merged_area_key.clone());
                    merged_area_key = self.split_area_by_coordinates(
                        merged_area_key,
                        neighbor_rect.xmin,
                        true,
                        ui_settings,
                    )?;
                }
                // ┏━━━━━━━━━━░
                // ┃ neighbor ░
                // ┗━┳━━━━━━━━░
                //   ┃  close ░
                //   ┗━━━━━━━━░
                else if neighbor_rect.xmin < close_rect.xmin {
                    new_empty_areas.push(working_neighbor_area_key.clone());
                    working_neighbor_area_key = self.split_area_by_coordinates(
                        working_neighbor_area_key,
                        close_rect.xmin,
                        true,
                        ui_settings,
                    )?;
                }

                // ░━━━━━━━━┓
                // ░neighbor┃
                // ░━━━━━━━━┻━┓
                // ░ close    ┃
                // ░━━━━━━━━━━┛
                if neighbor_rect.xmax < close_rect.xmax {
                    new_empty_areas.push(self.split_area_by_coordinates(
                        merged_area_key,
                        neighbor_rect.xmax,
                        true,
                        ui_settings,
                    )?);
                }
                // ░━━━━━━━━━━┓
                // ░ neighbor ┃
                // ░━━━━━━━━┳━┛
                // ░ close  ┃
                // ░━━━━━━━━┛
                else if neighbor_rect.xmax > close_rect.xmax {
                    new_empty_areas.push(self.split_area_by_coordinates(
                        working_neighbor_area_key,
                        close_rect.xmax,
                        true,
                        ui_settings,
                    )?);
                }

                // 开始合并对齐的面板
                self.set_area_vert(
                    working_neighbor_area_key,
                    VertPosition::BottomLeft(
                        self.areas
                            .get(merged_area_key)
                            .context(format!("合并区域不存在: {:?}", merged_area_key))?
                            .v_bottom_left(),
                    ),
                )?;
                self.set_area_vert(
                    working_neighbor_area_key,
                    VertPosition::BottomRight(
                        self.areas
                            .get(merged_area_key)
                            .context(format!("合并区域不存在: {:?}", merged_area_key))?
                            .v_bottom_right(),
                    ),
                )?;
                self.add_edge_to_geometry(
                    self.areas
                        .get(working_neighbor_area_key)
                        .context("不存在的neighbor_area_key")?
                        .v_top_left(),
                    self.areas
                        .get(working_neighbor_area_key)
                        .context("不存在的neighbor_area_key")?
                        .v_bottom_left(),
                );
                self.add_edge_to_geometry(
                    self.areas
                        .get(working_neighbor_area_key)
                        .context("不存在的neighbor_area_key")?
                        .v_top_right(),
                    self.areas
                        .get(working_neighbor_area_key)
                        .context("不存在的neighbor_area_key")?
                        .v_bottom_right(),
                );
            }
            Adjacency::BottomToTop {
                bottom: neighbor_rect,
                top: close_rect,
            } => {
                // ┏━━━━━━━━━━░
                // ┃    close ░
                // ┗━┳━━━━━━━━░
                //   ┃neighbor░
                //   ┗━━━━━━━━░
                if neighbor_rect.xmin > close_rect.xmin {
                    new_empty_areas.push(merged_area_key.clone());
                    merged_area_key = self.split_area_by_coordinates(
                        merged_area_key,
                        neighbor_rect.xmin,
                        true,
                        ui_settings,
                    )?;
                }
                //   ┏━━━━━━━━░
                //   ┃  close ░
                // ┏━┻━━━━━━━━░
                // ┃ neighbor ░
                // ┗━━━━━━━━━━░
                else if neighbor_rect.xmin < close_rect.xmin {
                    new_empty_areas.push(working_neighbor_area_key.clone());
                    working_neighbor_area_key = self.split_area_by_coordinates(
                        working_neighbor_area_key,
                        close_rect.xmin,
                        true,
                        ui_settings,
                    )?;
                }

                // ░━━━━━━━━━━┓
                // ░ close    ┃
                // ░━━━━━━━━┳━┛
                // ░neighbor┃
                // ░━━━━━━━━┛
                if neighbor_rect.xmax < close_rect.xmax {
                    new_empty_areas.push(self.split_area_by_coordinates(
                        merged_area_key,
                        neighbor_rect.xmax,
                        true,
                        ui_settings,
                    )?);
                // ░━━━━━━━━┓
                // ░ close  ┃
                // ░━━━━━━━━┻━┓
                // ░ neighbor ┃
                // ░━━━━━━━━━━┛
                } else if neighbor_rect.xmax > close_rect.xmax {
                    new_empty_areas.push(self.split_area_by_coordinates(
                        working_neighbor_area_key,
                        close_rect.xmax,
                        true,
                        ui_settings,
                    )?);
                }

                // 开始合并对齐的面板
                self.set_area_vert(
                    working_neighbor_area_key,
                    VertPosition::TopLeft(
                        self.areas
                            .get(merged_area_key)
                            .context(format!("合并区域不存在: {:?}", merged_area_key))?
                            .v_top_left(),
                    ),
                )?;
                self.set_area_vert(
                    working_neighbor_area_key,
                    VertPosition::TopRight(
                        self.areas
                            .get(merged_area_key)
                            .context(format!("合并区域不存在: {:?}", merged_area_key))?
                            .v_top_right(),
                    ),
                )?;
                self.add_edge_to_geometry(
                    self.areas
                        .get(working_neighbor_area_key)
                        .context("不存在的neighbor_area_key")?
                        .v_bottom_left(),
                    self.areas
                        .get(working_neighbor_area_key)
                        .context("不存在的neighbor_area_key")?
                        .v_top_left(),
                );
                self.add_edge_to_geometry(
                    self.areas
                        .get(working_neighbor_area_key)
                        .context("不存在的neighbor_area_key")?
                        .v_top_right(),
                    self.areas
                        .get(working_neighbor_area_key)
                        .context("不存在的neighbor_area_key")?
                        .v_bottom_right(),
                );
            }
            Adjacency::LeftToRight {
                left: neighbor_rect,
                right: close_rect,
            } => {
                //          ┏━━━━━━━━━┓
                // ┏━━━━━━━━┫         ┃
                // ┃neighbor┃  close  ┃
                // ░░░░░░░░░░░░░░░░░░░░
                if neighbor_rect.ymin > close_rect.ymin {
                    new_empty_areas.push(merged_area_key.clone());
                    merged_area_key = self.split_area_by_coordinates(
                        merged_area_key,
                        neighbor_rect.ymin,
                        false,
                        ui_settings,
                    )?;
                // ┏━━━━━━━━┓
                // ┃        ┣━━━━━━━━━┓
                // ┃neighbor┃  close  ┃
                // ░░░░░░░░░░░░░░░░░░░░
                } else if neighbor_rect.ymin < close_rect.ymin {
                    new_empty_areas.push(working_neighbor_area_key.clone());
                    working_neighbor_area_key = self.split_area_by_coordinates(
                        working_neighbor_area_key,
                        close_rect.ymin,
                        false,
                        ui_settings,
                    )?;
                }

                // ░░░░░░░░░░░░░░░░░░░░
                // ┃neighbor┃  close  ┃
                // ┗━━━━━━━━┫         ┃
                //          ┗━━━━━━━━━┛
                if neighbor_rect.ymax < close_rect.ymax {
                    new_empty_areas.push(self.split_area_by_coordinates(
                        merged_area_key,
                        neighbor_rect.ymax,
                        false,
                        ui_settings,
                    )?);
                // ░░░░░░░░░░░░░░░░░░░░
                // ┃neighbor┃  close  ┃
                // ┃        ┣━━━━━━━━━┛
                // ┗━━━━━━━━┛
                } else if neighbor_rect.ymax > close_rect.ymax {
                    new_empty_areas.push(self.split_area_by_coordinates(
                        working_neighbor_area_key,
                        close_rect.ymax,
                        false,
                        ui_settings,
                    )?);
                }
                // 开始合并对齐的面板
                self.set_area_vert(
                    working_neighbor_area_key,
                    VertPosition::TopRight(
                        self.areas
                            .get(merged_area_key)
                            .context("不存在合并area")?
                            .v_top_right(),
                    ),
                )?;
                self.set_area_vert(
                    working_neighbor_area_key,
                    VertPosition::BottomRight(
                        self.areas
                            .get(merged_area_key)
                            .context("不存在合并area")?
                            .v_bottom_right(),
                    ),
                )?;
                self.add_edge_to_geometry(
                    self.areas
                        .get(working_neighbor_area_key)
                        .context(format!("源区域不存在: {:?}", working_neighbor_area_key))?
                        .v_top_left(),
                    self.areas
                        .get(working_neighbor_area_key)
                        .context(format!("源区域不存在: {:?}", working_neighbor_area_key))?
                        .v_top_right(),
                );
                self.add_edge_to_geometry(
                    self.areas
                        .get(working_neighbor_area_key)
                        .context(format!("源区域不存在: {:?}", working_neighbor_area_key))?
                        .v_bottom_left(),
                    self.areas
                        .get(working_neighbor_area_key)
                        .context(format!("源区域不存在: {:?}", working_neighbor_area_key))?
                        .v_bottom_right(),
                );
            }
            Adjacency::RightToLeft {
                right: neighbor_rect,
                left: close_rect,
            } => {
                // ┏━━━━━━━━━┓
                // ┃         ┣━━━━━━━━┓
                // ┃  close  ┃neighbor┃
                // ░░░░░░░░░░░░░░░░░░░░
                if neighbor_rect.ymin > close_rect.ymin {
                    new_empty_areas.push(merged_area_key.clone());
                    merged_area_key = self.split_area_by_coordinates(
                        merged_area_key,
                        neighbor_rect.ymin,
                        false,
                        ui_settings,
                    )?;
                //           ┏━━━━━━━━┓
                // ┏━━━━━━━━━┫        ┃
                // ┃  close  ┃neighbor┃
                // ░░░░░░░░░░░░░░░░░░░░
                } else if neighbor_rect.ymin < close_rect.ymin {
                    new_empty_areas.push(working_neighbor_area_key.clone());
                    working_neighbor_area_key = self.split_area_by_coordinates(
                        working_neighbor_area_key,
                        close_rect.ymin,
                        false,
                        ui_settings,
                    )?;
                }

                // ░░░░░░░░░░░░░░░░░░░░
                // ┃  close  ┃neighbor┃
                // ┃         ┣━━━━━━━━┛
                // ┗━━━━━━━━━┛
                if neighbor_rect.ymax < close_rect.ymax {
                    new_empty_areas.push(self.split_area_by_coordinates(
                        merged_area_key,
                        neighbor_rect.ymax,
                        false,
                        ui_settings,
                    )?);
                // ░░░░░░░░░░░░░░░░░░░░
                // ┃  close  ┃neighbor┃
                // ┗━━━━━━━━━┫        ┃
                //           ┗━━━━━━━━┛
                } else if neighbor_rect.ymax > close_rect.ymax {
                    new_empty_areas.push(self.split_area_by_coordinates(
                        working_neighbor_area_key,
                        close_rect.ymax,
                        false,
                        ui_settings,
                    )?);
                }
                // 开始合并对齐的面板
                self.set_area_vert(
                    working_neighbor_area_key,
                    VertPosition::TopLeft(
                        self.areas
                            .get(merged_area_key)
                            .context("不存在合并area")?
                            .v_top_left(),
                    ),
                )?;
                self.set_area_vert(
                    working_neighbor_area_key,
                    VertPosition::BottomLeft(
                        self.areas
                            .get(merged_area_key)
                            .context("不存在合并area")?
                            .v_bottom_left(),
                    ),
                )?;
                self.add_edge_to_geometry(
                    self.areas
                        .get(working_neighbor_area_key)
                        .context(format!("源区域不存在: {:?}", working_neighbor_area_key))?
                        .v_top_left(),
                    self.areas
                        .get(working_neighbor_area_key)
                        .context(format!("源区域不存在: {:?}", working_neighbor_area_key))?
                        .v_top_right(),
                );
                self.add_edge_to_geometry(
                    self.areas
                        .get(working_neighbor_area_key)
                        .context(format!("源区域不存在: {:?}", working_neighbor_area_key))?
                        .v_bottom_right(),
                    self.areas
                        .get(working_neighbor_area_key)
                        .context(format!("源区域不存在: {:?}", working_neighbor_area_key))?
                        .v_bottom_left(),
                );
            }
        }

        // 删除被合并的empty区域
        self.areas.remove(merged_area_key);

        // 清理重复的边和顶点
        self.remove_duplicate_edges();
        self.remove_unused_edges();

        Ok(new_empty_areas)
    }

    /// 递归清理所有empty区域
    pub fn recursive_fill_close_areas(
        &mut self,
        close_areas: Vec<ScrAreaKey>,
        ui_settings: &UISettings,
    ) -> Result<()> {
        const MAX_ITERATIONS: usize = 100;
        let mut current_close_areas = close_areas;
        let mut iteration = 0;

        while !current_close_areas.is_empty() && iteration < MAX_ITERATIONS {
            iteration += 1;

            let mut next_close_areas = Vec::new();

            for close_area_key in current_close_areas {
                // 检查区域是否仍然存在（可能在之前的处理中被删除了）
                if self.areas.contains_key(close_area_key) {
                    match self.fill_close_area(close_area_key, ui_settings) {
                        Ok(mut new_closes) => {
                            next_close_areas.append(&mut new_closes);
                        }
                        Err(e) => {
                            eprintln!("填充 close 区域 {:?} 失败: {}", close_area_key, e);
                            // 继续处理其他区域，不中断整个流程
                        }
                    }
                } else {
                    eprintln!("Close 区域 {:?} 已不存在，跳过处理", close_area_key);
                }
            }

            current_close_areas = next_close_areas;
        }

        if iteration >= MAX_ITERATIONS {
            println!(
                "警告: 达到最大迭代次数 {}，可能存在无限循环",
                MAX_ITERATIONS
            );
            if !current_close_areas.is_empty() {
                println!(
                    "仍有 {} 个 close 区域未处理: {:?}",
                    current_close_areas.len(),
                    current_close_areas
                );
            }
        }

        Ok(())
    }

    /// 寻找空位的最佳邻居
    fn find_best_neighbor_for_close_area(&self, empty_area: &ScrArea) -> Option<ScrAreaKey> {
        let mut best_neighbor = None;
        let mut best_alignment = 0.0f64;

        // 遍历所有区域
        for (neighbor_key, neighbor) in &self.areas {
            if let Some(alignment) = self.calculate_edge_alignment(neighbor, empty_area) {
                if alignment > best_alignment {
                    best_alignment = alignment;
                    best_neighbor = Some(neighbor_key);
                }
            }
        }

        best_neighbor
    }

    // 计算两个Area的对齐度(目标应该是empty_area)
    pub fn calculate_edge_alignment(
        &self,
        source_area: &ScrArea,
        target_area: &ScrArea,
    ) -> Option<f64> {
        let r_source = source_area.get_raw_geometry(&self.verts);
        let r_target = target_area.get_raw_geometry(&self.verts);

        // 将坐标转换为 f64 以进行精确计算
        let (rs_xmin, rs_xmax) = (r_source.xmin as f64, r_source.xmax as f64);
        let (rs_ymin, rs_ymax) = (r_source.ymin as f64, r_source.ymax as f64);
        let (rt_xmin, rt_xmax) = (r_target.xmin as f64, r_target.xmax as f64);
        let (rt_ymin, rt_ymax) = (r_target.ymin as f64, r_target.ymax as f64);

        // 检查垂直相邻（左右关系）
        if rs_xmax == rt_xmin || rs_xmin == rt_xmax {
            // 计算垂直方向上的重叠长度
            let overlap_start = f64::max(rs_ymin, rt_ymin);
            let overlap_end = f64::min(rs_ymax, rt_ymax);
            let shared_length = overlap_end - overlap_start;

            // 必须有实际的重叠部分
            if shared_length > f64::EPSILON {
                let source_side_length = rs_ymax - rs_ymin;
                if source_side_length > f64::EPSILON {
                    return Some(shared_length / source_side_length);
                }
            }
        }
        // 检查水平相邻（上下关系）
        else if rs_ymax == rt_ymin || rs_ymin == rt_ymax {
            // 计算水平方向上的重叠长度
            let overlap_start = f64::max(rs_xmin, rt_xmin);
            let overlap_end = f64::min(rs_xmax, rt_xmax);
            let shared_length = overlap_end - overlap_start;

            // 必须有实际的重叠部分
            if shared_length > f64::EPSILON {
                let source_side_length = rs_xmax - rs_xmin;
                if source_side_length > f64::EPSILON {
                    return Some(shared_length / source_side_length);
                }
            }
        }

        // 如果不相邻或没有重叠，则返回 None
        None
    }

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
        // 检查是否已存在相同的边
        if let Some(existing_edge_key) = self.find_edge_between_vertices(v1, v2) {
            // 如果边已存在，直接返回现有的边
            return existing_edge_key;
        }

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

    /// 清理重复的边（参考Blender的BKE_screen_remove_double_scredges）
    fn remove_duplicate_edges(&mut self) {
        let edge_keys: Vec<ScrEdgeKey> = self.edges.keys().collect();
        let mut edges_to_remove = Vec::new();

        for (i, &edge_key1) in edge_keys.iter().enumerate() {
            if let Some(edge1) = self.edges.get(edge_key1) {
                // 检查与后续边的重复
                for &edge_key2 in edge_keys.iter().skip(i + 1) {
                    if let Some(edge2) = self.edges.get(edge_key2) {
                        // 检查是否是相同的边（考虑方向）
                        if (edge1.v1 == edge2.v1 && edge1.v2 == edge2.v2)
                            || (edge1.v1 == edge2.v2 && edge1.v2 == edge2.v1)
                        {
                            edges_to_remove.push(edge_key2);
                        }
                    }
                }
            }
        }

        // 移除重复的边
        let removed_count = edges_to_remove.len();
        for edge_key in edges_to_remove {
            self.edges.remove(edge_key);
        }

        if removed_count > 0 {
            // println!("清理了 {} 条重复边", removed_count);
        }
    }

    /// 清理未使用的边（参考Blender的BKE_screen_remove_unused_scredges）
    fn remove_unused_edges(&mut self) {
        // 标记所有被区域使用的边
        let mut used_edges = std::collections::HashSet::new();

        for area in self.areas.values() {
            // 检查区域的四条边
            let area_edges = [
                (area.v_top_left(), area.v_top_right()),
                (area.v_top_right(), area.v_bottom_right()),
                (area.v_bottom_right(), area.v_bottom_left()),
                (area.v_bottom_left(), area.v_top_left()),
            ];

            for (v1, v2) in area_edges {
                // 查找连接这两个顶点的边
                if let Some(edge_key) = self.find_edge_between_vertices(v1, v2) {
                    used_edges.insert(edge_key);
                }
            }
        }

        // 移除未使用的边
        let edge_keys: Vec<ScrEdgeKey> = self.edges.keys().collect();
        let mut unused_edges = Vec::new();

        for &edge_key in &edge_keys {
            if !used_edges.contains(&edge_key) {
                unused_edges.push(edge_key);
            }
        }

        let removed_count = unused_edges.len();
        for edge_key in unused_edges {
            self.edges.remove(edge_key);
        }

        if removed_count > 0 {
            println!("清理了 {} 条未使用的边", removed_count);
        }
    }

    pub fn detect_adjacency_by_keys(
        &self,
        source_area_key: ScrAreaKey,
        target_area_key: ScrAreaKey,
    ) -> Option<Adjacency> {
        // 最小重叠要求 (与 apply_join_adjacent_areas 保持一致)
        #[cfg(test)]
        const MIN_AREA_SIZE: u32 = 1; // 测试时使用更小的值便于口算
        #[cfg(not(test))]
        const MIN_AREA_SIZE: u32 = 50; // 生产环境使用正常值

        let source_rect = self
            .areas
            .get(source_area_key)?
            .get_raw_geometry(&self.verts);
        let target_rect = self
            .areas
            .get(target_area_key)?
            .get_raw_geometry(&self.verts);

        self.detect_adjacency(source_rect, target_rect, MIN_AREA_SIZE)
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
    fn test_duplicate_edge_removal() {
        let mut screen_map = ScrAreaMap::new();

        // 创建顶点
        let v1 = screen_map.create_vertex(0, 0);
        let v2 = screen_map.create_vertex(100, 0);

        // 创建相同的边两次
        let edge1 = screen_map.create_edge(v1, v2);
        let edge2 = screen_map.create_edge(v1, v2);

        // 验证确实创建了两条边
        assert_eq!(screen_map.edges.len(), 2);

        // 运行清理函数
        screen_map.remove_duplicate_edges();

        // 验证重复边被清理
        assert_eq!(screen_map.edges.len(), 1);
    }

    #[test]
    fn test_unused_edge_removal() {
        let mut screen_map = ScrAreaMap::new();

        // 创建顶点
        let v1 = screen_map.create_vertex(0, 0);
        let v2 = screen_map.create_vertex(100, 0);
        let v3 = screen_map.create_vertex(100, 100);
        let v4 = screen_map.create_vertex(0, 100);

        // 创建区域
        let area = ScrArea::new(v1, v2, v3, v4, AreaType::Empty, true, false, false);
        let area_key = screen_map.areas.insert(area);

        // 创建区域使用的边
        screen_map.create_edge(v1, v2);
        screen_map.create_edge(v2, v3);
        screen_map.create_edge(v3, v4);
        screen_map.create_edge(v4, v1);

        // 创建一些未使用的边
        let v5 = screen_map.create_vertex(50, 50);
        screen_map.create_edge(v1, v5);
        screen_map.create_edge(v5, v3);

        // 验证初始状态
        assert_eq!(screen_map.edges.len(), 6);

        // 运行清理函数
        screen_map.remove_unused_edges();

        // 验证未使用的边被清理
        assert_eq!(screen_map.edges.len(), 4);
    }

    #[test]
    fn test_join_adjacent_areas_small_to_large() {
        let mut screen_map = ScrAreaMap::new();
        let ui_settings = UISettings::default();

        // 创建顶点：使用10以内的坐标便于口算
        // 大区域：(0,0) 到 (8,6) - 8x6
        // 小区域：(2,6) 到 (6,8) - 4x2，位于大区域下方
        let v1 = screen_map.create_vertex(0, 0); // 大区域左上
        let v2 = screen_map.create_vertex(8, 0); // 大区域右上
        let v3 = screen_map.create_vertex(8, 6); // 大区域右下
        let v4 = screen_map.create_vertex(0, 6); // 大区域左下

        let v5 = screen_map.create_vertex(2, 6); // 小区域左上
        let v6 = screen_map.create_vertex(6, 6); // 小区域右上
        let v7 = screen_map.create_vertex(6, 8); // 小区域右下
        let v8 = screen_map.create_vertex(2, 8); // 小区域左下

        // 创建边
        // 大区域的边
        screen_map.create_edge(v1, v2); // 上边
        screen_map.create_edge(v2, v3); // 右边
        screen_map.create_edge(v3, v4); // 下边
        screen_map.create_edge(v4, v1); // 左边

        // 小区域的边
        screen_map.create_edge(v5, v6); // 上边
        screen_map.create_edge(v6, v7); // 右边
        screen_map.create_edge(v7, v8); // 下边
        screen_map.create_edge(v8, v5); // 左边

        // 创建两个区域
        let mut large_area = ScrArea::new(v1, v2, v3, v4, AreaType::Video, true, false, false);
        large_area.calculate_totrct_and_size(&screen_map.verts, &ui_settings);
        let large_area_key = screen_map.areas.insert(large_area);

        let mut small_area = ScrArea::new(v5, v6, v7, v8, AreaType::Timeline, true, false, false);
        small_area.calculate_totrct_and_size(&screen_map.verts, &ui_settings);
        let small_area_key = screen_map.areas.insert(small_area);

        // 验证初始状态
        assert_eq!(screen_map.areas.len(), 2);
        assert_eq!(screen_map.verts.len(), 8);
        assert_eq!(screen_map.edges.len(), 8);

        // 验证区域几何
        let large_area = screen_map.areas.get(large_area_key).unwrap();
        let small_area = screen_map.areas.get(small_area_key).unwrap();

        let large_rect = large_area.get_raw_geometry(&screen_map.verts);
        let small_rect = small_area.get_raw_geometry(&screen_map.verts);

        assert_eq!(large_rect.xmin, 0);
        assert_eq!(large_rect.ymin, 0);
        assert_eq!(large_rect.xmax, 8);
        assert_eq!(large_rect.ymax, 6);

        assert_eq!(small_rect.xmin, 2);
        assert_eq!(small_rect.ymin, 6);
        assert_eq!(small_rect.xmax, 6);
        assert_eq!(small_rect.ymax, 8);

        // 执行合并：小区域指向大区域
        let adjacency = screen_map
            .detect_adjacency(small_rect, large_rect, 1)
            .context("非相邻Area或重叠不过多")
            .unwrap();
        let result = screen_map.apply_join_adjacent_areas(
            small_area_key,
            large_area_key,
            adjacency,
            &ui_settings,
        );
        assert!(result.is_ok(), "合并应该成功: {:?}", result);

        // 验证合并后的结果：应该有3个区域
        assert_eq!(screen_map.areas.len(), 3, "合并后应该有3个区域");

        // 验证每个区域的类型、坐标、大小和顶点
        let mut found_left_area = false;
        let mut found_center_area = false;
        let mut found_right_area = false;

        for (area_key, area) in &screen_map.areas {
            let rect = area.get_raw_geometry(&screen_map.verts);
            let width = rect.xmax - rect.xmin;
            let height = rect.ymax - rect.ymin;

            // 获取区域的四个顶点坐标
            let top_left = screen_map.verts.get(area.v_top_left()).unwrap();
            let top_right = screen_map.verts.get(area.v_top_right()).unwrap();
            let bottom_right = screen_map.verts.get(area.v_bottom_right()).unwrap();
            let bottom_left = screen_map.verts.get(area.v_bottom_left()).unwrap();

            // 左侧区域：(0,0) 到 (2,6) - Video类型
            if rect.xmin == 0 && rect.xmax == 2 && rect.ymin == 0 && rect.ymax == 6 {
                assert!(
                    matches!(area.area_type, AreaType::Video),
                    "左侧区域应该是Video类型"
                );
                assert_eq!(width, 2, "左侧区域宽度应该是2");
                assert_eq!(height, 6, "左侧区域高度应该是6");

                // 验证顶点坐标
                assert_eq!(top_left.vec.x, 0, "左侧区域左上角x坐标应该是0");
                assert_eq!(top_left.vec.y, 0, "左侧区域左上角y坐标应该是0");
                assert_eq!(top_right.vec.x, 2, "左侧区域右上角x坐标应该是2");
                assert_eq!(top_right.vec.y, 0, "左侧区域右上角y坐标应该是0");
                assert_eq!(bottom_right.vec.x, 2, "左侧区域右下角x坐标应该是2");
                assert_eq!(bottom_right.vec.y, 6, "左侧区域右下角y坐标应该是6");
                assert_eq!(bottom_left.vec.x, 0, "左侧区域左下角x坐标应该是0");
                assert_eq!(bottom_left.vec.y, 6, "左侧区域左下角y坐标应该是6");

                found_left_area = true;
            }
            // 中间区域：(2,0) 到 (6,8) - Timeline类型
            else if rect.xmin == 2 && rect.xmax == 6 && rect.ymin == 0 && rect.ymax == 8 {
                assert!(
                    matches!(area.area_type, AreaType::Timeline),
                    "中间区域应该是Timeline类型"
                );
                assert_eq!(width, 4, "中间区域宽度应该是4");
                assert_eq!(height, 8, "中间区域高度应该是8");

                // 验证顶点坐标
                assert_eq!(top_left.vec.x, 2, "中间区域左上角x坐标应该是2");
                assert_eq!(top_left.vec.y, 0, "中间区域左上角y坐标应该是0");
                assert_eq!(top_right.vec.x, 6, "中间区域右上角x坐标应该是6");
                assert_eq!(top_right.vec.y, 0, "中间区域右上角y坐标应该是0");
                assert_eq!(bottom_right.vec.x, 6, "中间区域右下角x坐标应该是6");
                assert_eq!(bottom_right.vec.y, 8, "中间区域右下角y坐标应该是8");
                assert_eq!(bottom_left.vec.x, 2, "中间区域左下角x坐标应该是2");
                assert_eq!(bottom_left.vec.y, 8, "中间区域左下角y坐标应该是8");

                found_center_area = true;
            }
            // 右侧区域：(6,0) 到 (8,6) - Video类型
            else if rect.xmin == 6 && rect.xmax == 8 && rect.ymin == 0 && rect.ymax == 6 {
                assert!(
                    matches!(area.area_type, AreaType::Video),
                    "右侧区域应该是Video类型"
                );
                assert_eq!(width, 2, "右侧区域宽度应该是2");
                assert_eq!(height, 6, "右侧区域高度应该是6");

                // 验证顶点坐标
                assert_eq!(top_left.vec.x, 6, "右侧区域左上角x坐标应该是6");
                assert_eq!(top_left.vec.y, 0, "右侧区域左上角y坐标应该是0");
                assert_eq!(top_right.vec.x, 8, "右侧区域右上角x坐标应该是8");
                assert_eq!(top_right.vec.y, 0, "右侧区域右上角y坐标应该是0");
                assert_eq!(bottom_right.vec.x, 8, "右侧区域右下角x坐标应该是8");
                assert_eq!(bottom_right.vec.y, 6, "右侧区域右下角y坐标应该是6");
                assert_eq!(bottom_left.vec.x, 6, "右侧区域左下角x坐标应该是6");
                assert_eq!(bottom_left.vec.y, 6, "右侧区域左下角y坐标应该是6");

                found_right_area = true;
            } else {
                panic!(
                    "发现意外的区域: {:?} 类型={:?} 坐标=({}, {}) 到 ({}, {}) 大小={}x{}\n\
                    顶点坐标: 左上({}, {}), 右上({}, {}), 右下({}, {}), 左下({}, {})",
                    area_key,
                    area.area_type,
                    rect.xmin,
                    rect.ymin,
                    rect.xmax,
                    rect.ymax,
                    width,
                    height,
                    top_left.vec.x,
                    top_left.vec.y,
                    top_right.vec.x,
                    top_right.vec.y,
                    bottom_right.vec.x,
                    bottom_right.vec.y,
                    bottom_left.vec.x,
                    bottom_left.vec.y
                );
            }
        }

        // 验证所有预期的区域都被找到
        assert!(found_left_area, "应该找到左侧区域");
        assert!(found_center_area, "应该找到中间区域");
        assert!(found_right_area, "应该找到右侧区域");
    }

    #[test]
    fn test_join_adjacent_areas_left_aligned() {
        let mut screen_map = ScrAreaMap::new();
        let ui_settings = UISettings::default();

        // 创建顶点：左边对齐的情况
        // 大区域：(0,0) 到 (8,6) - 8x6
        // 小区域：(0,6) 到 (4,8) - 4x2，位于大区域下方且左边对齐
        let v1 = screen_map.create_vertex(0, 0); // 大区域左上
        let v2 = screen_map.create_vertex(8, 0); // 大区域右上
        let v3 = screen_map.create_vertex(8, 6); // 大区域右下
        let v4 = screen_map.create_vertex(0, 6); // 大区域左下

        let v5 = screen_map.create_vertex(0, 6); // 小区域左上（与大区域左下对齐）
        let v6 = screen_map.create_vertex(4, 6); // 小区域右上
        let v7 = screen_map.create_vertex(4, 8); // 小区域右下
        let v8 = screen_map.create_vertex(0, 8); // 小区域左下

        // 创建边
        // 大区域的边
        screen_map.create_edge(v1, v2); // 上边
        screen_map.create_edge(v2, v3); // 右边
        screen_map.create_edge(v3, v4); // 下边
        screen_map.create_edge(v4, v1); // 左边

        // 小区域的边
        screen_map.create_edge(v5, v6); // 上边
        screen_map.create_edge(v6, v7); // 右边
        screen_map.create_edge(v7, v8); // 下边
        screen_map.create_edge(v8, v5); // 左边

        // 创建两个区域
        let mut large_area = ScrArea::new(v1, v2, v3, v4, AreaType::Video, true, false, false);
        large_area.calculate_totrct_and_size(&screen_map.verts, &ui_settings);
        let large_area_key = screen_map.areas.insert(large_area);

        let mut small_area = ScrArea::new(v5, v6, v7, v8, AreaType::Timeline, true, false, false);
        small_area.calculate_totrct_and_size(&screen_map.verts, &ui_settings);
        let small_area_key = screen_map.areas.insert(small_area);

        // 验证初始状态
        assert_eq!(screen_map.areas.len(), 2);
        assert_eq!(screen_map.verts.len(), 8);
        assert_eq!(screen_map.edges.len(), 8);

        // 验证区域几何
        let large_area = screen_map.areas.get(large_area_key).unwrap();
        let small_area = screen_map.areas.get(small_area_key).unwrap();

        let large_rect = large_area.get_raw_geometry(&screen_map.verts);
        let small_rect = small_area.get_raw_geometry(&screen_map.verts);

        assert_eq!(large_rect.xmin, 0);
        assert_eq!(large_rect.ymin, 0);
        assert_eq!(large_rect.xmax, 8);
        assert_eq!(large_rect.ymax, 6);

        assert_eq!(small_rect.xmin, 0);
        assert_eq!(small_rect.ymin, 6);
        assert_eq!(small_rect.xmax, 4);
        assert_eq!(small_rect.ymax, 8);

        // 执行合并：小区域指向大区域
        let adjacency = screen_map
            .detect_adjacency(small_rect, large_rect, 1)
            .context("非相邻Area或重叠不过多")
            .unwrap();
        let result = screen_map.apply_join_adjacent_areas(
            small_area_key,
            large_area_key,
            adjacency,
            &ui_settings,
        );
        assert!(result.is_ok(), "合并应该成功: {:?}", result);

        // 验证合并后的结果：应该有2个区域
        assert_eq!(screen_map.areas.len(), 2, "合并后应该有2个区域");

        // 验证每个区域的类型、坐标、大小和顶点
        let mut found_left_area = false;
        let mut found_right_area = false;

        for (area_key, area) in &screen_map.areas {
            let rect = area.get_raw_geometry(&screen_map.verts);
            let width = rect.xmax - rect.xmin;
            let height = rect.ymax - rect.ymin;

            // 获取区域的四个顶点坐标
            let top_left = screen_map.verts.get(area.v_top_left()).unwrap();
            let top_right = screen_map.verts.get(area.v_top_right()).unwrap();
            let bottom_right = screen_map.verts.get(area.v_bottom_right()).unwrap();
            let bottom_left = screen_map.verts.get(area.v_bottom_left()).unwrap();

            // 左侧区域：(0,0) 到 (4,8) - Timeline类型（合并后的区域）
            if rect.xmin == 0 && rect.xmax == 4 && rect.ymin == 0 && rect.ymax == 8 {
                assert!(
                    matches!(area.area_type, AreaType::Timeline),
                    "左侧区域应该是Timeline类型"
                );
                assert_eq!(width, 4, "左侧区域宽度应该是4");
                assert_eq!(height, 8, "左侧区域高度应该是8");

                // 验证顶点坐标
                assert_eq!(top_left.vec.x, 0, "左侧区域左上角x坐标应该是0");
                assert_eq!(top_left.vec.y, 0, "左侧区域左上角y坐标应该是0");
                assert_eq!(top_right.vec.x, 4, "左侧区域右上角x坐标应该是4");
                assert_eq!(top_right.vec.y, 0, "左侧区域右上角y坐标应该是0");
                assert_eq!(bottom_right.vec.x, 4, "左侧区域右下角x坐标应该是4");
                assert_eq!(bottom_right.vec.y, 8, "左侧区域右下角y坐标应该是8");
                assert_eq!(bottom_left.vec.x, 0, "左侧区域左下角x坐标应该是0");
                assert_eq!(bottom_left.vec.y, 8, "左侧区域左下角y坐标应该是8");

                found_left_area = true;
            }
            // 右侧区域：(4,0) 到 (8,6) - Video类型（剩余的大区域部分）
            else if rect.xmin == 4 && rect.xmax == 8 && rect.ymin == 0 && rect.ymax == 6 {
                assert!(
                    matches!(area.area_type, AreaType::Video),
                    "右侧区域应该是Video类型"
                );
                assert_eq!(width, 4, "右侧区域宽度应该是4");
                assert_eq!(height, 6, "右侧区域高度应该是6");

                // 验证顶点坐标
                assert_eq!(top_left.vec.x, 4, "右侧区域左上角x坐标应该是4");
                assert_eq!(top_left.vec.y, 0, "右侧区域左上角y坐标应该是0");
                assert_eq!(top_right.vec.x, 8, "右侧区域右上角x坐标应该是8");
                assert_eq!(top_right.vec.y, 0, "右侧区域右上角y坐标应该是0");
                assert_eq!(bottom_right.vec.x, 8, "右侧区域右下角x坐标应该是8");
                assert_eq!(bottom_right.vec.y, 6, "右侧区域右下角y坐标应该是6");
                assert_eq!(bottom_left.vec.x, 4, "右侧区域左下角x坐标应该是4");
                assert_eq!(bottom_left.vec.y, 6, "右侧区域左下角y坐标应该是6");

                found_right_area = true;
            } else {
                panic!(
                    "发现意外的区域: {:?} 类型={:?} 坐标=({}, {}) 到 ({}, {}) 大小={}x{}\n\
                    顶点坐标: 左上({}, {}), 右上({}, {}), 右下({}, {}), 左下({}, {})",
                    area_key,
                    area.area_type,
                    rect.xmin,
                    rect.ymin,
                    rect.xmax,
                    rect.ymax,
                    width,
                    height,
                    top_left.vec.x,
                    top_left.vec.y,
                    top_right.vec.x,
                    top_right.vec.y,
                    bottom_right.vec.x,
                    bottom_right.vec.y,
                    bottom_left.vec.x,
                    bottom_left.vec.y
                );
            }
        }

        // 验证所有预期的区域都被找到
        assert!(found_left_area, "应该找到左侧区域");
        assert!(found_right_area, "应该找到右侧区域");

        println!("左边对齐合并测试成功！");
    }

    #[test]
    fn test_join_adjacent_areas_right_aligned() {
        let mut screen_map = ScrAreaMap::new();
        let ui_settings = UISettings::default();

        // 创建顶点：右边对齐的情况
        // 大区域：(0,0) 到 (8,6) - 8x6
        // 小区域：(4,6) 到 (8,8) - 4x2，位于大区域下方且右边对齐
        let v1 = screen_map.create_vertex(0, 0); // 大区域左上
        let v2 = screen_map.create_vertex(8, 0); // 大区域右上
        let v3 = screen_map.create_vertex(8, 6); // 大区域右下
        let v4 = screen_map.create_vertex(0, 6); // 大区域左下

        let v5 = screen_map.create_vertex(4, 6); // 小区域左上
        let v6 = screen_map.create_vertex(8, 6); // 小区域右上（与大区域右下对齐）
        let v7 = screen_map.create_vertex(8, 8); // 小区域右下
        let v8 = screen_map.create_vertex(4, 8); // 小区域左下

        // 创建边
        // 大区域的边
        screen_map.create_edge(v1, v2); // 上边
        screen_map.create_edge(v2, v3); // 右边
        screen_map.create_edge(v3, v4); // 下边
        screen_map.create_edge(v4, v1); // 左边

        // 小区域的边
        screen_map.create_edge(v5, v6); // 上边
        screen_map.create_edge(v6, v7); // 右边
        screen_map.create_edge(v7, v8); // 下边
        screen_map.create_edge(v8, v5); // 左边

        // 创建两个区域
        let mut large_area = ScrArea::new(v1, v2, v3, v4, AreaType::Video, true, false, false);
        large_area.calculate_totrct_and_size(&screen_map.verts, &ui_settings);
        let large_area_key = screen_map.areas.insert(large_area);

        let mut small_area = ScrArea::new(v5, v6, v7, v8, AreaType::Timeline, true, false, false);
        small_area.calculate_totrct_and_size(&screen_map.verts, &ui_settings);
        let small_area_key = screen_map.areas.insert(small_area);

        // 验证初始状态
        assert_eq!(screen_map.areas.len(), 2);
        assert_eq!(screen_map.verts.len(), 8);
        assert_eq!(screen_map.edges.len(), 8);

        // 验证区域几何
        let large_area = screen_map.areas.get(large_area_key).unwrap();
        let small_area = screen_map.areas.get(small_area_key).unwrap();

        let large_rect = large_area.get_raw_geometry(&screen_map.verts);
        let small_rect = small_area.get_raw_geometry(&screen_map.verts);

        assert_eq!(large_rect.xmin, 0);
        assert_eq!(large_rect.ymin, 0);
        assert_eq!(large_rect.xmax, 8);
        assert_eq!(large_rect.ymax, 6);

        assert_eq!(small_rect.xmin, 4);
        assert_eq!(small_rect.ymin, 6);
        assert_eq!(small_rect.xmax, 8);
        assert_eq!(small_rect.ymax, 8);

        // 执行合并：小区域指向大区域
        let adjacency = screen_map
            .detect_adjacency(small_rect, large_rect, 1)
            .context("非相邻Area或重叠不过多")
            .unwrap();
        let result = screen_map.apply_join_adjacent_areas(
            small_area_key,
            large_area_key,
            adjacency,
            &ui_settings,
        );
        assert!(result.is_ok(), "合并应该成功: {:?}", result);

        // 验证合并后的结果：应该有2个区域
        assert_eq!(screen_map.areas.len(), 2, "合并后应该有2个区域");

        // 验证每个区域的类型、坐标、大小和顶点
        let mut found_left_area = false;
        let mut found_right_area = false;

        for (area_key, area) in &screen_map.areas {
            let rect = area.get_raw_geometry(&screen_map.verts);
            let width = rect.xmax - rect.xmin;
            let height = rect.ymax - rect.ymin;

            // 获取区域的四个顶点坐标
            let top_left = screen_map.verts.get(area.v_top_left()).unwrap();
            let top_right = screen_map.verts.get(area.v_top_right()).unwrap();
            let bottom_right = screen_map.verts.get(area.v_bottom_right()).unwrap();
            let bottom_left = screen_map.verts.get(area.v_bottom_left()).unwrap();

            // 左侧区域：(0,0) 到 (4,6) - Video类型（剩余的大区域部分）
            if rect.xmin == 0 && rect.xmax == 4 && rect.ymin == 0 && rect.ymax == 6 {
                assert!(
                    matches!(area.area_type, AreaType::Video),
                    "左侧区域应该是Video类型"
                );
                assert_eq!(width, 4, "左侧区域宽度应该是4");
                assert_eq!(height, 6, "左侧区域高度应该是6");

                // 验证顶点坐标
                assert_eq!(top_left.vec.x, 0, "左侧区域左上角x坐标应该是0");
                assert_eq!(top_left.vec.y, 0, "左侧区域左上角y坐标应该是0");
                assert_eq!(top_right.vec.x, 4, "左侧区域右上角x坐标应该是4");
                assert_eq!(top_right.vec.y, 0, "左侧区域右上角y坐标应该是0");
                assert_eq!(bottom_right.vec.x, 4, "左侧区域右下角x坐标应该是4");
                assert_eq!(bottom_right.vec.y, 6, "左侧区域右下角y坐标应该是6");
                assert_eq!(bottom_left.vec.x, 0, "左侧区域左下角x坐标应该是0");
                assert_eq!(bottom_left.vec.y, 6, "左侧区域左下角y坐标应该是6");

                found_left_area = true;
            }
            // 右侧区域：(4,0) 到 (8,8) - Timeline类型（合并后的区域）
            else if rect.xmin == 4 && rect.xmax == 8 && rect.ymin == 0 && rect.ymax == 8 {
                assert!(
                    matches!(area.area_type, AreaType::Timeline),
                    "右侧区域应该是Timeline类型"
                );
                assert_eq!(width, 4, "右侧区域宽度应该是4");
                assert_eq!(height, 8, "右侧区域高度应该是8");

                // 验证顶点坐标
                assert_eq!(top_left.vec.x, 4, "右侧区域左上角x坐标应该是4");
                assert_eq!(top_left.vec.y, 0, "右侧区域左上角y坐标应该是0");
                assert_eq!(top_right.vec.x, 8, "右侧区域右上角x坐标应该是8");
                assert_eq!(top_right.vec.y, 0, "右侧区域右上角y坐标应该是0");
                assert_eq!(bottom_right.vec.x, 8, "右侧区域右下角x坐标应该是8");
                assert_eq!(bottom_right.vec.y, 8, "右侧区域右下角y坐标应该是8");
                assert_eq!(bottom_left.vec.x, 4, "右侧区域左下角x坐标应该是4");
                assert_eq!(bottom_left.vec.y, 8, "右侧区域左下角y坐标应该是8");

                found_right_area = true;
            } else {
                panic!(
                    "发现意外的区域: {:?} 类型={:?} 坐标=({}, {}) 到 ({}, {}) 大小={}x{}\n\
                    顶点坐标: 左上({}, {}), 右上({}, {}), 右下({}, {}), 左下({}, {})",
                    area_key,
                    area.area_type,
                    rect.xmin,
                    rect.ymin,
                    rect.xmax,
                    rect.ymax,
                    width,
                    height,
                    top_left.vec.x,
                    top_left.vec.y,
                    top_right.vec.x,
                    top_right.vec.y,
                    bottom_right.vec.x,
                    bottom_right.vec.y,
                    bottom_left.vec.x,
                    bottom_left.vec.y
                );
            }
        }

        // 验证所有预期的区域都被找到
        assert!(found_left_area, "应该找到左侧区域");
        assert!(found_right_area, "应该找到右侧区域");

        println!("右边对齐合并测试成功！");
    }

    #[test]
    fn test_join_adjacent_areas_complex_six_areas() {
        // WARN: 测试时注意设置参数让它**不要**应用边距，不然 1px 大小的面板会导致负数溢出
        let mut screen_map = ScrAreaMap::new();
        let ui_settings = UISettings::default();

        // 创建六个区域的顶点
        // Area1: (0,0) 到 (5,7) - 5x7
        let area1_tl = screen_map.create_vertex(0, 0); // 左上
        let area1_tr = screen_map.create_vertex(5, 0); // 右上
        let area1_br = screen_map.create_vertex(5, 7); // 右下
        let area1_bl = screen_map.create_vertex(0, 7); // 左下

        // Area2: (0,7) 到 (5,9) - 5x2
        let area2_tl = area1_bl; // 左上（与Area1共享）
        let area2_tr = area1_br; // 右上（与Area1共享）
        let area2_br = screen_map.create_vertex(5, 9); // 右下
        let area2_bl = screen_map.create_vertex(0, 9); // 左下

        // Area3: (0,9) 到 (5,10) - 5x1
        let area3_tl = area2_bl; // 左上（与Area2共享）
        let area3_tr = area2_br; // 右上（与Area2共享）
        let area3_br = screen_map.create_vertex(5, 10); // 右下
        let area3_bl = screen_map.create_vertex(0, 10); // 左下

        // Area4: (5,0) 到 (8,5) - 3x5
        let area4_tl = area1_tr; // 左上（与Area1共享）
        let area4_tr = screen_map.create_vertex(8, 0); // 右上
        let area4_br = screen_map.create_vertex(8, 5); // 右下
        let area4_bl = screen_map.create_vertex(5, 5); // 左下

        // Area5: (8,0) 到 (10,5) - 2x5
        let area5_tl = area4_tr; // 左上（与Area4共享）
        let area5_tr = screen_map.create_vertex(10, 0); // 右上
        let area5_br = screen_map.create_vertex(10, 5); // 右下
        let area5_bl = area4_br; // 左下（与Area4共享）

        // Area6: (5,5) 到 (10,10) - 5x5
        let area6_tl = area4_bl; // 左上（与Area4共享）
        let area6_tr = area5_br; // 右上（与Area5共享）
        let area6_br = screen_map.create_vertex(10, 10); // 右下
        let area6_bl = area3_br; // 左下（与Area3共享）

        // 创建边
        // Area1的边
        screen_map.create_edge(area1_tl, area1_tr);
        screen_map.create_edge(area1_tr, area1_br);
        screen_map.create_edge(area1_br, area1_bl);
        screen_map.create_edge(area1_bl, area1_tl);

        // Area2的边
        screen_map.create_edge(area2_tl, area2_tr);
        screen_map.create_edge(area2_tr, area2_br);
        screen_map.create_edge(area2_br, area2_bl);
        screen_map.create_edge(area2_bl, area2_tl);

        // Area3的边
        screen_map.create_edge(area3_tl, area3_tr);
        screen_map.create_edge(area3_tr, area3_br);
        screen_map.create_edge(area3_br, area3_bl);
        screen_map.create_edge(area3_bl, area3_tl);

        // Area4的边
        screen_map.create_edge(area4_tl, area4_tr);
        screen_map.create_edge(area4_tr, area4_br);
        screen_map.create_edge(area4_br, area4_bl);
        screen_map.create_edge(area4_bl, area4_tl);

        // Area5的边
        screen_map.create_edge(area5_tl, area5_tr);
        screen_map.create_edge(area5_tr, area5_br);
        screen_map.create_edge(area5_br, area5_bl);
        screen_map.create_edge(area5_bl, area5_tl);

        // Area6的边
        screen_map.create_edge(area6_tl, area6_tr);
        screen_map.create_edge(area6_tr, area6_br);
        screen_map.create_edge(area6_br, area6_bl);
        screen_map.create_edge(area6_bl, area6_tl);

        let mut area1 = ScrArea::new(
            area1_tl,
            area1_tr,
            area1_br,
            area1_bl,
            AreaType::Video,
            true,
            false,
            false,
        );
        let tl = screen_map.verts.get(area1_tl).unwrap();
        let tr = screen_map.verts.get(area1_tr).unwrap();
        let bl = screen_map.verts.get(area1_bl).unwrap();
        area1.calculate_totrct_and_size(&screen_map.verts, &ui_settings);
        let area1_key = screen_map.areas.insert(area1);

        let mut area2 = ScrArea::new(
            area2_tl,
            area2_tr,
            area2_br,
            area2_bl,
            AreaType::Timeline,
            true,
            false,
            false,
        );
        let tl = screen_map.verts.get(area2_tl).unwrap();
        let tr = screen_map.verts.get(area2_tr).unwrap();
        let bl = screen_map.verts.get(area2_bl).unwrap();
        area2.calculate_totrct_and_size(&screen_map.verts, &ui_settings);
        let area2_key = screen_map.areas.insert(area2);

        let mut area3 = ScrArea::new(
            area3_tl,
            area3_tr,
            area3_br,
            area3_bl,
            AreaType::SubtitleEditor,
            true,
            false,
            false,
        );
        let tl = screen_map.verts.get(area3_tl).unwrap();
        let tr = screen_map.verts.get(area3_tr).unwrap();
        let bl = screen_map.verts.get(area3_bl).unwrap();
        area3.calculate_totrct_and_size(&screen_map.verts, &ui_settings);
        let area3_key = screen_map.areas.insert(area3);

        let mut area4 = ScrArea::new(
            area4_tl,
            area4_tr,
            area4_br,
            area4_bl,
            AreaType::Properties,
            true,
            false,
            false,
        );
        let tl = screen_map.verts.get(area4_tl).unwrap();
        let tr = screen_map.verts.get(area4_tr).unwrap();
        let bl = screen_map.verts.get(area4_bl).unwrap();
        area4.calculate_totrct_and_size(&screen_map.verts, &ui_settings);
        let area4_key = screen_map.areas.insert(area4);

        let mut area5 = ScrArea::new(
            area5_tl,
            area5_tr,
            area5_br,
            area5_bl,
            AreaType::Empty,
            true,
            false,
            false,
        );
        let tl = screen_map.verts.get(area5_tl).unwrap();
        let tr = screen_map.verts.get(area5_tr).unwrap();
        let bl = screen_map.verts.get(area5_bl).unwrap();
        area5.calculate_totrct_and_size(&screen_map.verts, &ui_settings);
        let area5_key = screen_map.areas.insert(area5);

        let mut area6 = ScrArea::new(
            area6_tl,
            area6_tr,
            area6_br,
            area6_bl,
            AreaType::Empty,
            true,
            false,
            false,
        );
        let tl = screen_map.verts.get(area6_tl).unwrap();
        let tr = screen_map.verts.get(area6_tr).unwrap();
        let bl = screen_map.verts.get(area6_bl).unwrap();
        area6.calculate_totrct_and_size(&screen_map.verts, &ui_settings);
        let area6_key = screen_map.areas.insert(area6);

        // 验证初始状态
        assert_eq!(screen_map.areas.len(), 6);

        // 验证各区域的几何
        let area1_rect = screen_map
            .areas
            .get(area1_key)
            .unwrap()
            .get_raw_geometry(&screen_map.verts);
        let area2_rect = screen_map
            .areas
            .get(area2_key)
            .unwrap()
            .get_raw_geometry(&screen_map.verts);
        let area3_rect = screen_map
            .areas
            .get(area3_key)
            .unwrap()
            .get_raw_geometry(&screen_map.verts);
        let area4_rect = screen_map
            .areas
            .get(area4_key)
            .unwrap()
            .get_raw_geometry(&screen_map.verts);
        let area5_rect = screen_map
            .areas
            .get(area5_key)
            .unwrap()
            .get_raw_geometry(&screen_map.verts);
        let area6_rect = screen_map
            .areas
            .get(area6_key)
            .unwrap()
            .get_raw_geometry(&screen_map.verts);

        assert_eq!(area1_rect.xmin, 0);
        assert_eq!(area1_rect.ymin, 0);
        assert_eq!(area1_rect.xmax, 5);
        assert_eq!(area1_rect.ymax, 7);
        assert_eq!(area2_rect.xmin, 0);
        assert_eq!(area2_rect.ymin, 7);
        assert_eq!(area2_rect.xmax, 5);
        assert_eq!(area2_rect.ymax, 9);
        assert_eq!(area3_rect.xmin, 0);
        assert_eq!(area3_rect.ymin, 9);
        assert_eq!(area3_rect.xmax, 5);
        assert_eq!(area3_rect.ymax, 10);
        assert_eq!(area4_rect.xmin, 5);
        assert_eq!(area4_rect.ymin, 0);
        assert_eq!(area4_rect.xmax, 8);
        assert_eq!(area4_rect.ymax, 5);
        assert_eq!(area5_rect.xmin, 8);
        assert_eq!(area5_rect.ymin, 0);
        assert_eq!(area5_rect.xmax, 10);
        assert_eq!(area5_rect.ymax, 5);
        assert_eq!(area6_rect.xmin, 5);
        assert_eq!(area6_rect.ymin, 5);
        assert_eq!(area6_rect.xmax, 10);
        assert_eq!(area6_rect.ymax, 10);

        // 执行合并：Area6(source) 指向 Area2(target)
        let adjacency = screen_map
            .detect_adjacency(area6_rect, area2_rect, 1)
            .context("非相邻Area或重叠不过多")
            .unwrap();
        let result =
            screen_map.apply_join_adjacent_areas(area6_key, area2_key, adjacency, &ui_settings);

        println!("\n合并结果: {:?}", result);

        println!("\n合并后状态:");
        println!("  区域数量: {}", screen_map.areas.len());
        println!("  顶点数量: {}", screen_map.verts.len());
        println!("  边数量: {}", screen_map.edges.len());

        // 打印合并后的所有区域坐标
        println!("\n=== 合并后剩余区域坐标信息 ===");
        for (area_key, area) in &screen_map.areas {
            let rect = area.get_raw_geometry(&screen_map.verts);
            let width = rect.xmax - rect.xmin;
            let height = rect.ymax - rect.ymin;

            // 获取区域的四个顶点坐标
            let top_left = screen_map.verts.get(area.v_top_left()).unwrap();
            let top_right = screen_map.verts.get(area.v_top_right()).unwrap();
            let bottom_right = screen_map.verts.get(area.v_bottom_right()).unwrap();
            let bottom_left = screen_map.verts.get(area.v_bottom_left()).unwrap();

            println!("区域 {:?}:", area_key);
            println!("  类型: {:?}", area.area_type);
            println!(
                "  坐标: ({}, {}) 到 ({}, {})",
                rect.xmin, rect.ymin, rect.xmax, rect.ymax
            );
            println!("  大小: {}x{}", width, height);
            println!("  顶点坐标:");
            println!("    左上: ({}, {})", top_left.vec.x, top_left.vec.y);
            println!("    右上: ({}, {})", top_right.vec.x, top_right.vec.y);
            println!("    右下: ({}, {})", bottom_right.vec.x, bottom_right.vec.y);
            println!("    左下: ({}, {})", bottom_left.vec.x, bottom_left.vec.y);
            println!();
        }

        println!("=== 复杂六区域合并测试完成 ===");
    }

    #[test]
    fn test_join_non_adjacent_areas_should_fail() {
        let mut screen_map = ScrAreaMap::new();
        let ui_settings = UISettings::default();

        println!("=== 测试不相邻区域合并（应该失败） ===");

        // 创建两个不相邻的区域
        // Area1: (0,0) 到 (3,3) - 3x3
        let area1_tl = screen_map.create_vertex(0, 0);
        let area1_tr = screen_map.create_vertex(3, 0);
        let area1_br = screen_map.create_vertex(3, 3);
        let area1_bl = screen_map.create_vertex(0, 3);

        // Area2: (5,5) 到 (8,8) - 3x3 (与Area1不相邻)
        let area2_tl = screen_map.create_vertex(5, 5);
        let area2_tr = screen_map.create_vertex(8, 5);
        let area2_br = screen_map.create_vertex(8, 8);
        let area2_bl = screen_map.create_vertex(5, 8);

        // 创建边
        screen_map.create_edge(area1_tl, area1_tr);
        screen_map.create_edge(area1_tr, area1_br);
        screen_map.create_edge(area1_br, area1_bl);
        screen_map.create_edge(area1_bl, area1_tl);

        screen_map.create_edge(area2_tl, area2_tr);
        screen_map.create_edge(area2_tr, area2_br);
        screen_map.create_edge(area2_br, area2_bl);
        screen_map.create_edge(area2_bl, area2_tl);

        // 创建区域
        let mut area1 = ScrArea::new(
            area1_tl,
            area1_tr,
            area1_br,
            area1_bl,
            AreaType::Video,
            true,
            false,
            false,
        );
        area1.calculate_totrct_and_size(&screen_map.verts, &ui_settings);
        let area1_key = screen_map.areas.insert(area1);

        let mut area2 = ScrArea::new(
            area2_tl,
            area2_tr,
            area2_br,
            area2_bl,
            AreaType::Timeline,
            true,
            false,
            false,
        );
        area2.calculate_totrct_and_size(&screen_map.verts, &ui_settings);
        let area2_key = screen_map.areas.insert(area2);

        println!("尝试合并不相邻的区域...");

        // 尝试合并不相邻的区域，应该返回错误
        let area1_rect = screen_map
            .areas
            .get(area1_key)
            .unwrap()
            .get_raw_geometry(&screen_map.verts);
        let area2_rect = screen_map
            .areas
            .get(area2_key)
            .unwrap()
            .get_raw_geometry(&screen_map.verts);
        let adjacency_result = screen_map.detect_adjacency(area1_rect, area2_rect, 1);

        // 检测相邻关系应该失败，因为区域不相邻
        assert!(adjacency_result.is_none(), "不相邻区域应该返回 None");

        // 如果真的有相邻关系（不应该发生），测试合并操作
        if let Some(adjacency) = adjacency_result {
            let result =
                screen_map.apply_join_adjacent_areas(area1_key, area2_key, adjacency, &ui_settings);
            assert!(result.is_err(), "合并不相邻区域应该失败");
        }
        println!("✓ 正确检测到不相邻区域并返回错误");

        // 验证区域没有被修改
        assert_eq!(screen_map.areas.len(), 2);
        println!("✓ 区域数量保持不变");

        println!("=== 不相邻区域合并测试完成 ===");
    }
}
