use anyhow::{Context, Result};

use crate::{
    screen::{
        area::{ScrArea, ScrAreaKey},
        map::ScrAreaMap,
        primitives::{Adjacency, Rcti},
        vertex::VertPosition,
    },
    types::{CloseAreaOverlayData, JoinHighlightOverlayData, UISettings},
};

impl ScrAreaMap {
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

        // 最小重叠要求 (TODO: calculate_split_areas 当中一致，需要之后合并)
        #[cfg(test)]
        const MIN_AREA_SIZE: u32 = 1; // 测试时使用更小的值便于口算
        #[cfg(not(test))]
        const MIN_AREA_SIZE: u32 = 50; // 生产环境使用正常值

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
                // 当分割出来的区域小于最小区域限制时添加到close当中
                if top.xmin > bottom.xmin && (top.xmin - bottom.xmin) < MIN_AREA_SIZE {
                    let close_width = top.xmin - bottom.xmin + 1;
                    let close_height = bottom.height();
                    close_areas_overlay_data.push(CloseAreaOverlayData {
                        x: bottom.xmin as f32,
                        y: bottom.ymin as f32,
                        width: close_width as f32,
                        height: close_height as f32,
                    });
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
                if top.xmax < bottom.xmax && (bottom.xmax - top.xmax) < MIN_AREA_SIZE {
                    let close_width = bottom.xmax - top.xmax + 1;
                    let close_height = bottom.height();
                    close_areas_overlay_data.push(CloseAreaOverlayData {
                        x: top.xmin as f32,
                        y: bottom.ymin as f32,
                        width: close_width as f32,
                        height: close_height as f32,
                    });
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
                if bottom.xmin > top.xmin && (bottom.xmin - top.xmin) < MIN_AREA_SIZE {
                    let close_width = bottom.xmin - top.xmin + 1;
                    let close_height = top.height();
                    close_areas_overlay_data.push(CloseAreaOverlayData {
                        x: top.xmin as f32,
                        y: top.ymin as f32,
                        width: close_width as f32,
                        height: close_height as f32,
                    });
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
                if bottom.xmax < top.xmax && (top.xmax - bottom.xmax) < MIN_AREA_SIZE {
                    let close_width = top.xmax - bottom.xmax + 1;
                    let close_height = top.height();
                    close_areas_overlay_data.push(CloseAreaOverlayData {
                        x: bottom.xmin as f32,
                        y: top.ymin as f32,
                        width: close_width as f32,
                        height: close_height as f32,
                    });
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
                if left.ymin > right.ymin && (left.ymin - right.ymin) < MIN_AREA_SIZE {
                    let close_width = right.width();
                    let close_height = left.ymin - right.ymin + 1;
                    close_areas_overlay_data.push(CloseAreaOverlayData {
                        x: right.xmin as f32,
                        y: right.ymin as f32,
                        width: close_width as f32,
                        height: close_height as f32,
                    });
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
                if left.ymax < right.ymax && (right.ymax - left.ymax) < MIN_AREA_SIZE {
                    let close_width = right.width();
                    let close_height = right.ymax - left.ymax + 1;
                    close_areas_overlay_data.push(CloseAreaOverlayData {
                        x: right.xmin as f32,
                        y: left.ymax as f32,
                        width: close_width as f32,
                        height: close_height as f32,
                    });
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
                if right.ymin > left.ymin && (right.ymin - left.ymin) < MIN_AREA_SIZE {
                    let close_width = left.width();
                    let close_height = right.ymin - left.ymin + 1;
                    close_areas_overlay_data.push(CloseAreaOverlayData {
                        x: left.xmin as f32,
                        y: left.ymin as f32,
                        width: close_width as f32,
                        height: close_height as f32,
                    });
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
                if right.ymax < left.ymax && (left.ymax - right.ymax) < MIN_AREA_SIZE {
                    let close_width = left.width();
                    let close_height = left.ymax - right.ymax + 1;
                    close_areas_overlay_data.push(CloseAreaOverlayData {
                        x: left.xmin as f32,
                        y: right.ymax as f32,
                        width: close_width as f32,
                        height: close_height as f32,
                    });
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

        // 最小重叠要求 (TODO: calculate_split_areas 当中一致，需要之后合并)
        #[cfg(test)]
        const MIN_AREA_SIZE: u32 = 1; // 测试时使用更小的值便于口算
        #[cfg(not(test))]
        const MIN_AREA_SIZE: u32 = 50; // 生产环境使用正常值

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
                    let temp_close_area_key = merged_area_key.clone();
                    merged_area_key = self.split_area_by_coordinates(
                        merged_area_key,
                        top.xmin,
                        true,
                        ui_settings,
                    )?;
                    if (top.xmin - bottom.xmin) < MIN_AREA_SIZE {
                        new_empty_areas.push(temp_close_area_key)
                    }
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
                    let temp_close_area_key = self.split_area_by_coordinates(
                        merged_area_key,
                        top.xmax,
                        true,
                        ui_settings,
                    )?;
                    if (bottom.xmax - top.xmax) < MIN_AREA_SIZE {
                        new_empty_areas.push(temp_close_area_key)
                    }
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
                    let temp_close_area_key = merged_area_key.clone();
                    merged_area_key = self.split_area_by_coordinates(
                        merged_area_key,
                        bottom.xmin,
                        true,
                        ui_settings,
                    )?;
                    if (bottom.xmin - top.xmin) < MIN_AREA_SIZE {
                        new_empty_areas.push(temp_close_area_key)
                    }
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
                    let temp_close_area_key = self.split_area_by_coordinates(
                        merged_area_key,
                        bottom.xmax,
                        true,
                        ui_settings,
                    )?;
                    if (top.xmax - bottom.xmax) < MIN_AREA_SIZE {
                        new_empty_areas.push(temp_close_area_key)
                    }
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
                    let temp_close_area_key = merged_area_key.clone();
                    merged_area_key = self.split_area_by_coordinates(
                        merged_area_key,
                        left.ymin,
                        false,
                        ui_settings,
                    )?;
                    if (left.ymin - right.ymin) < MIN_AREA_SIZE {
                        new_empty_areas.push(temp_close_area_key)
                    }
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
                    let temp_close_area_key = self.split_area_by_coordinates(
                        merged_area_key,
                        left.ymax,
                        false,
                        ui_settings,
                    )?;
                    if (right.ymax - left.ymax) < MIN_AREA_SIZE {
                        new_empty_areas.push(temp_close_area_key)
                    }
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
                    let temp_close_area_key = merged_area_key.clone();
                    merged_area_key = self.split_area_by_coordinates(
                        merged_area_key,
                        right.ymin,
                        false,
                        ui_settings,
                    )?;
                    if (right.ymin - left.ymin) < MIN_AREA_SIZE {
                        new_empty_areas.push(temp_close_area_key)
                    }
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
                    let temp_close_area_key = self.split_area_by_coordinates(
                        merged_area_key,
                        right.ymax,
                        false,
                        ui_settings,
                    )?;
                    if (left.ymax - right.ymax) < MIN_AREA_SIZE {
                        new_empty_areas.push(temp_close_area_key)
                    }
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
    use crate::screen::{
        area::{AreaType, ScrArea},
        map::ScrAreaMap,
    };
    use crate::types::UISettings;
    use anyhow::Context;

    #[test]
    fn test_join_adjacent_areas_small_to_large() {
        let mut screen_map = ScrAreaMap::new();
        let ui_settings = UISettings::default();

        // 创建顶点：使用10以内的坐标便于口算
        // 大区域：(0,0) 到 (8,6) - 8x6
        // 小区域：(2,6) 到 (6,8) - 4x2，位于大区域下方
        let v1 = screen_map.add_vertex_to_geometry(0, 0); // 大区域左上
        let v2 = screen_map.add_vertex_to_geometry(8, 0); // 大区域右上
        let v3 = screen_map.add_vertex_to_geometry(8, 6); // 大区域右下
        let v4 = screen_map.add_vertex_to_geometry(0, 6); // 大区域左下

        let v5 = screen_map.add_vertex_to_geometry(2, 6); // 小区域左上
        let v6 = screen_map.add_vertex_to_geometry(6, 6); // 小区域右上
        let v7 = screen_map.add_vertex_to_geometry(6, 8); // 小区域右下
        let v8 = screen_map.add_vertex_to_geometry(2, 8); // 小区域左下

        // 创建边
        // 大区域的边
        screen_map.add_edge_to_geometry(v1, v2); // 上边
        screen_map.add_edge_to_geometry(v2, v3); // 右边
        screen_map.add_edge_to_geometry(v3, v4); // 下边
        screen_map.add_edge_to_geometry(v4, v1); // 左边

        // 小区域的边
        screen_map.add_edge_to_geometry(v5, v6); // 上边
        screen_map.add_edge_to_geometry(v6, v7); // 右边
        screen_map.add_edge_to_geometry(v7, v8); // 下边
        screen_map.add_edge_to_geometry(v8, v5); // 左边

        // 创建两个区域
        let mut large_area = ScrArea::new(v1, v2, v3, v4, AreaType::Empty, true, false, false);
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
                    matches!(area.area_type, AreaType::Empty),
                    "左侧区域应该是Empty类型"
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
                    matches!(area.area_type, AreaType::Empty),
                    "右侧区域应该是Empty类型"
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
    fn test_join_non_adjacent_areas_should_fail() {
        let mut screen_map = ScrAreaMap::new();
        let ui_settings = UISettings::default();

        println!("=== 测试不相邻区域合并（应该失败） ===");

        // 创建两个不相邻的区域
        // Area1: (0,0) 到 (3,3) - 3x3
        let area1_tl = screen_map.add_vertex_to_geometry(0, 0);
        let area1_tr = screen_map.add_vertex_to_geometry(3, 0);
        let area1_br = screen_map.add_vertex_to_geometry(3, 3);
        let area1_bl = screen_map.add_vertex_to_geometry(0, 3);

        // Area2: (5,5) 到 (8,8) - 3x3 (与Area1不相邻)
        let area2_tl = screen_map.add_vertex_to_geometry(5, 5);
        let area2_tr = screen_map.add_vertex_to_geometry(8, 5);
        let area2_br = screen_map.add_vertex_to_geometry(8, 8);
        let area2_bl = screen_map.add_vertex_to_geometry(5, 8);

        // 创建边
        screen_map.add_edge_to_geometry(area1_tl, area1_tr);
        screen_map.add_edge_to_geometry(area1_tr, area1_br);
        screen_map.add_edge_to_geometry(area1_br, area1_bl);
        screen_map.add_edge_to_geometry(area1_bl, area1_tl);

        screen_map.add_edge_to_geometry(area2_tl, area2_tr);
        screen_map.add_edge_to_geometry(area2_tr, area2_br);
        screen_map.add_edge_to_geometry(area2_br, area2_bl);
        screen_map.add_edge_to_geometry(area2_bl, area2_tl);

        // 创建区域
        let mut area1 = ScrArea::new(
            area1_tl,
            area1_tr,
            area1_br,
            area1_bl,
            AreaType::Empty,
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
