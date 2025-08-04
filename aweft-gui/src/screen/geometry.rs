use anyhow::{Context, Result, anyhow};

use crate::SplitPreviewOverlayData;
use crate::types::{
    AreaType, Rcti, ScrArea, ScrAreaKey, ScrAreaMap, ScrEdge, ScrEdgeKey, ScrVert, ScrVertKey,
    UISettings, Vec2u32, VertPosition,
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

        // 关键：区域创建后立即计算几何属性
        // 这体现了"顶点->几何计算->区域属性"的正确顺序
        area.calculate_totrct_and_size(
            &self.verts,
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
        area1.calculate_totrct_and_size(&self.verts, ui_settings, false, false, false);
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
        area2.calculate_totrct_and_size(&self.verts, ui_settings, false, false, false);
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
        area3.calculate_totrct_and_size(&self.verts, ui_settings, false, false, false);
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

    /// 合并两个相邻的区域
    /// 这个函数实现了Blender中screen_area_join_aligned的逻辑
    pub fn join_adjacent_areas(
        &mut self,
        source_area_key: ScrAreaKey,
        target_area_key: ScrAreaKey,
        ui_settings: &UISettings,
    ) -> Result<()> {
        if source_area_key == target_area_key {
            return Ok(());
        }

        // 最小重叠要求 (TODO: calculate_split_areas 当中一致，需要之后合并)
        const MIN_AREA_SIZE: u32 = 50;

        let source_rect = self
            .areas
            .get(source_area_key)
            .context(format!("源区域不存在: {:?}", source_area_key))? // 到这里得到 &Area
            .get_raw_geometry(&self.verts); // 调用方法，得到 Rect

        let target_rect = self
            .areas
            .get(target_area_key)
            .context(format!("目标区域不存在: {:?}", target_area_key))?
            .get_raw_geometry(&self.verts);

        // 计算重叠点坐标
        let overlap_left = std::cmp::max(source_rect.xmin, target_rect.xmin);
        let overlap_right = std::cmp::min(source_rect.xmax, target_rect.xmax);
        let overlap_top = std::cmp::max(source_rect.ymin, target_rect.ymin);
        let overlap_bottom = std::cmp::min(source_rect.ymax, target_rect.ymax);
        let overlap_x = overlap_right - overlap_left;
        let overlap_y = overlap_bottom - overlap_top;

        // 产生的空白区域
        // let mut remainders = Vec::new();
        let mut merge_area_key: ScrAreaKey = target_area_key;

        // （上下关系）
        // source 在上
        if (source_rect.ymax == target_rect.ymin) && overlap_x >= MIN_AREA_SIZE {
            //   ┏━━━━━━░
            //   ┃source░
            // ┏━┻━━━━━━░
            // ┃        ░
            // ┃  target░
            // ┃        ░
            // ┗━━━━━━━━░
            if source_rect.xmin > target_rect.xmin {
                // 右方的新建Area
                // 此时 target_area 已经被修改
                merge_area_key = self.split_area_by_coordinates(
                    target_area_key,
                    source_rect.xmin,
                    true,
                    ui_settings,
                )?;

            // ┏━━━━━━━━░
            // ┃        ░
            // ┃  source░
            // ┃        ░
            // ┗━┳━━━━━━░
            //   ┃target░
            //   ┗━━━━━━░
            } else if source_rect.xmin < target_rect.xmin {
                unimplemented!("source 在上的大合小的实现(左侧)")
            }

            // ░━━━━━━┓
            // ░source┃
            // ░━━━━━━┻━┓
            // ░        ┃
            // ░target  ┃
            // ░        ┃
            // ░━━━━━━━━┛
            // 说明右边目标区域更大(即目标区域需要split, 可能已被上方代码分割过)
            if source_rect.xmax < target_rect.xmax {
                self.split_area_by_coordinates(
                    merge_area_key,
                    source_rect.xmin,
                    true,
                    ui_settings,
                )?;
            // ░━━━━━━━━┓
            // ░        ┃
            // ░source  ┃
            // ░        ┃
            // ░━━━━━━┳━┛
            // ░target┃
            // ░━━━━━━┛
            } else if source_rect.xmax > target_rect.xmax {
                unimplemented!("source 在上的大合小的实现(右侧)")
            }

            // 开始合并对齐的面板

            // WARN: TODO: 需要想一个可以在可变与不可变当中设置顶点的方法来统一一下
            self.set_area_vert(
                source_area_key,
                VertPosition::BottomLeft(
                    self.areas
                        .get(merge_area_key)
                        .context(format!("合并区域不存在: {:?}", merge_area_key))?
                        .v_bottom_left,
                ),
            );
            self.set_area_vert(
                source_area_key,
                VertPosition::BottomRight(
                    self.areas
                        .get(merge_area_key)
                        .context(format!("合并区域不存在: {:?}", merge_area_key))?
                        .v_bottom_right,
                ),
            );
            self.add_edge_to_geometry(
                self.areas
                    .get(source_area_key)
                    .context("不存在的source_area_key")?
                    .v_top_left,
                self.areas
                    .get(source_area_key)
                    .context("不存在的source_area_key")?
                    .v_bottom_left,
            );
            self.add_edge_to_geometry(
                self.areas
                    .get(source_area_key)
                    .context("不存在的source_area_key")?
                    .v_top_right,
                self.areas
                    .get(source_area_key)
                    .context("不存在的source_area_key")?
                    .v_bottom_right,
            );
        } else if (source_rect.ymin == target_rect.ymax) && overlap_x >= MIN_AREA_SIZE {
            unimplemented!("source 在下的合并实现")
        } else {
            panic!("上下 Area 坐标不一致")
        }

        // WARN: 如果前面触发,这里可以清理然后返回不执行后面了

        // （左右关系）
        // source 在左
        if (source_rect.xmax == target_rect.xmin) && overlap_y >= MIN_AREA_SIZE {
            //        ┏━━━━━━━━━━┓
            // ┏━━━━━━┫          ┃
            // ┃source┃  target  ┃
            // ░░░░░░░░░░░░░░░░░░░
            if source_rect.ymin > target_rect.ymin {
                let merge_area_key = self.split_area_by_coordinates(
                    target_area_key,
                    source_rect.ymin,
                    false,
                    ui_settings,
                )?;
            // ┏━━━━━━━━━━┓
            // ┃          ┣━━━━━━┓
            // ┃  source  ┃target┃
            // ░░░░░░░░░░░░░░░░░░░
            } else if source_rect.ymin < target_rect.ymin {
                unimplemented!("source 在左的大合小的实现(上侧)")
            }

            // ░░░░░░░░░░░░░░░░░░░
            // ┃source┃  target  ┃
            // ┗━━━━━━┫          ┃
            //        ┗━━━━━━━━━━┛
            if source_rect.ymax < target_rect.ymax {
                self.split_area_by_coordinates(
                    merge_area_key,
                    source_rect.ymax,
                    false,
                    ui_settings,
                )?;
            // ░░░░░░░░░░░░░░░░░░░
            // ┃  source  ┃target┃
            // ┃          ┣━━━━━━┛
            // ┗━━━━━━━━━━┛
            } else if source_rect.ymax > target_rect.ymax {
                unimplemented!("source 在左的大合小的实现(下侧)")
            }
            // 开始合并对齐的面板
            self.set_area_vert(
                source_area_key,
                VertPosition::TopRight(
                    self.areas
                        .get(merge_area_key)
                        .context("不存在合并area")?
                        .v_top_right,
                ),
            );
            self.set_area_vert(
                source_area_key,
                VertPosition::BottomRight(
                    self.areas
                        .get(merge_area_key)
                        .context("不存在合并area")?
                        .v_bottom_right,
                ),
            );
            self.add_edge_to_geometry(
                self.areas
                    .get(source_area_key)
                    .context(format!("源区域不存在: {:?}", source_area_key))?
                    .v_top_left,
                self.areas
                    .get(source_area_key)
                    .context(format!("源区域不存在: {:?}", source_area_key))?
                    .v_top_right,
            );
            self.add_edge_to_geometry(
                self.areas
                    .get(source_area_key)
                    .context(format!("源区域不存在: {:?}", source_area_key))?
                    .v_bottom_left,
                self.areas
                    .get(source_area_key)
                    .context(format!("源区域不存在: {:?}", source_area_key))?
                    .v_bottom_right,
            );
        } else if (source_rect.xmin == target_rect.xmax) && overlap_y >= MIN_AREA_SIZE {
            unimplemented!("source 在右的合并实现")
        }

        // 删除目标区域
        self.areas.remove(target_area_key);

        // 清理重复的边和顶点
        self.remove_duplicate_edges();
        self.remove_unused_edges();

        Ok(())
    }

    pub fn split_area_by_coordinates(
        &mut self,
        area_key: ScrAreaKey,
        split_coord: u32,
        is_horizontal_split: bool,
        ui_settings: &UISettings,
    ) -> Result<ScrAreaKey> {
        let (vert_rect, v1, v2, v3, v4, area_type) = {
            let area = self
                .areas
                .get(area_key)
                .ok_or_else(|| anyhow::anyhow!("源区域 {:?} 不存在", area_key))?;
            // 使用原始几何信息（边距调整之前的）
            let vert_rect = area.get_raw_geometry(&self.verts);
            (
                vert_rect,
                area.v_top_left,
                area.v_top_right,
                area.v_bottom_right,
                area.v_bottom_left,
                area.area_type.clone(),
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

            let new_area = ScrArea::new(new_vert_top, v2, v3, new_vert_bottom, area_type);
            let new_area_key = self.areas.insert(new_area);

            // 修改原始区域
            let source_area = self.areas.get_mut(area_key).unwrap();
            source_area.v_top_right = new_vert_top;
            source_area.v_bottom_right = new_vert_bottom;

            // 重新计算两个区域的几何属性
            source_area.calculate_totrct_and_size(&self.verts, ui_settings, false, false, false);
            let new_area_mut = self.areas.get_mut(new_area_key).unwrap();
            new_area_mut.calculate_totrct_and_size(&self.verts, ui_settings, false, false, false);

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

            let new_area = ScrArea::new(new_vert_left, new_vert_right, v3, v4, area_type);
            let new_area_key = self.areas.insert(new_area);

            // 修改原始区域
            let source_area = self.areas.get_mut(area_key).unwrap();
            source_area.v_bottom_left = new_vert_left;
            source_area.v_bottom_right = new_vert_right;

            // 重新计算两个区域的几何属性
            source_area.calculate_totrct_and_size(&self.verts, ui_settings, false, false, false);
            let new_area_mut = self.areas.get_mut(new_area_key).unwrap();
            new_area_mut.calculate_totrct_and_size(&self.verts, ui_settings, false, false, false);

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
            println!("清理了 {} 条重复边", removed_count);
        }
    }

    /// 清理未使用的边（参考Blender的BKE_screen_remove_unused_scredges）
    fn remove_unused_edges(&mut self) {
        // 标记所有被区域使用的边
        let mut used_edges = std::collections::HashSet::new();

        for area in self.areas.values() {
            // 检查区域的四条边
            let area_edges = [
                (area.v_top_left, area.v_top_right),
                (area.v_top_right, area.v_bottom_right),
                (area.v_bottom_right, area.v_bottom_left),
                (area.v_bottom_left, area.v_top_left),
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
        let area = ScrArea::new(v1, v2, v3, v4, AreaType::Empty);
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
}
