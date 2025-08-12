use crate::{
    screen::{
        edge::ScrEdgeKey,
        map::ScrAreaMap,
        primitives::Rcti,
        vertex::{ScrVert, ScrVertKey},
    },
    types::UISettings,
};

impl ScrAreaMap {
    /// 计算单条边对移动的约束
    /// 返回: (向正方向限制, 向负方向限制)
    fn calculate_edge_constraint(
        &self,
        dir_axis_horizontal: bool,
        origval: u32,
        fixed_vertex: &ScrVert,
        min_area_size: u32,
    ) -> (u32, u32) {
        if dir_axis_horizontal {
            // 水平轴（改变Y坐标）
            let distance = if fixed_vertex.vec.y > origval {
                // 固定顶点在移动轴下方，限制向下移动
                let max_down = fixed_vertex
                    .vec
                    .y
                    .saturating_sub(origval)
                    .saturating_sub(min_area_size);
                (max_down, u32::MAX)
            } else if fixed_vertex.vec.y < origval {
                // 固定顶点在移动轴上方，限制向上移动
                let max_up = origval
                    .saturating_sub(fixed_vertex.vec.y)
                    .saturating_sub(min_area_size) as u32;
                (u32::MAX, max_up)
            } else {
                // 在同一水平线上，无约束
                (u32::MAX, u32::MAX)
            };
            distance
        } else {
            // 垂直轴（改变X坐标）
            let distance = if fixed_vertex.vec.x > origval {
                // 固定顶点在移动轴右侧，限制向右移动
                let max_right = fixed_vertex
                    .vec
                    .x
                    .saturating_sub(origval)
                    .saturating_sub(min_area_size);
                (max_right, u32::MAX)
            } else if fixed_vertex.vec.x < origval {
                // 固定顶点在移动轴左侧，限制向左移动
                let max_left = origval
                    .saturating_sub(fixed_vertex.vec.x)
                    .saturating_sub(min_area_size);
                (u32::MAX, max_left)
            } else {
                // 在同一垂直线上，无约束
                (u32::MAX, u32::MAX)
            };
            distance
        }
    }

    /// 应用边移动，更新顶点坐标并刷新区域几何
    /// 返回实际应用的最终坐标
    pub fn apply_edge_move(
        &mut self,
        dir_axis_horizontal: bool,
        moving_vert_keys: &[ScrVertKey],
        origval: u32,
        delta: i32,
        bigger: u32,
        smaller: u32,
    ) -> u32 {
        let final_loc = if delta > 0 {
            let move_amount = delta as u32;
            let clamped_amount = move_amount.min(bigger);
            origval.saturating_add(clamped_amount)
        } else if delta < 0 {
            let move_amount = delta.unsigned_abs();
            let clamped_amount = move_amount.min(smaller);
            origval.saturating_sub(clamped_amount)
        } else {
            origval
        };

        for &vert_key in moving_vert_keys {
            if let Some(vert) = self.verts.get_mut(vert_key) {
                if dir_axis_horizontal {
                    vert.vec.y = final_loc;
                } else {
                    vert.vec.x = final_loc;
                }
            }
        }

        final_loc
    }

    /// 收集所有"可交互"的内部边（排除窗口边缘边）。
    pub fn collect_internal_edges(&self) -> Vec<ScrEdgeKey> {
        let mut out = Vec::new();
        for (ekey, e) in &self.edges {
            if !e.border {
                out.push(ekey);
            }
        }
        out
    }

    /// 根据窗口边界标记边的 `border` 字段：靠近四周范围内的标记为 true，其余为 false。
    pub fn mark_window_border_edges(&mut self, window_rect: &Rcti, ui_settings: &UISettings) {
        let d2 = ui_settings.get_edge_filter_boundary().round() as u32;
        for (_, e) in &mut self.edges {
            let Some(v1) = self.verts.get(e.v1) else {
                panic!("存在无效的索引: {:#?}", e.v1);
            };
            let Some(v2) = self.verts.get(e.v2) else {
                panic!("存在无效的索引: {:#?}", e.v2);
            };
            let is_border = match (v1.vec.y == v2.vec.y, v1.vec.x == v2.vec.x) {
                (true, false) => {
                    let y = v1.vec.y;
                    y <= window_rect.ymin.saturating_add(d2)
                        || y >= window_rect.ymax.saturating_sub(d2)
                }
                (false, true) => {
                    let x = v1.vec.x;
                    x <= window_rect.xmin.saturating_add(d2)
                        || x >= window_rect.xmax.saturating_sub(d2)
                }
                _ => false,
            };
            e.border = is_border;
        }
    }
    /// 从移动顶点集合中分析移动方向和原始坐标
    /// 返回: (dir_axis_horizontal, origval)
    /// dir_axis_horizontal: true=水平移动(改变Y坐标)，false=垂直移动(改变X坐标)
    pub fn analyze_move_direction(&self, moving_vert_keys: &[ScrVertKey]) -> Option<(bool, u32)> {
        if moving_vert_keys.len() < 2 {
            return None;
        }

        // 取前两个顶点来判断移动方向
        let v1 = self.verts.get(moving_vert_keys[0])?;
        let v2 = self.verts.get(moving_vert_keys[1])?;

        // 判断移动方向
        let dir_axis_horizontal = if v1.vec.y == v2.vec.y {
            true // 水平线，移动时改变Y坐标
        } else if v1.vec.x == v2.vec.x {
            false // 垂直线，移动时改变X坐标
        } else {
            return None; // 非轴对齐，不支持移动
        };

        // 获取原始轴坐标值
        let origval = if dir_axis_horizontal {
            v1.vec.y
        } else {
            v1.vec.x
        };

        Some((dir_axis_horizontal, origval))
    }

    /// 计算边移动的范围限制，确保所有区域不小于最小尺寸
    /// 返回: (bigger, smaller) - 向正方向和负方向的最大位移
    pub fn compute_move_limits(
        &self,
        dir_axis_horizontal: bool,
        moving_vert_keys: &[ScrVertKey],
        origval: u32,
        min_area_size: u32,
    ) -> (u32, u32) {
        let mut bigger = u32::MAX;
        let mut smaller = u32::MAX;

        // 创建移动顶点的集合，便于快速查找
        let moving_set: std::collections::HashSet<ScrVertKey> =
            moving_vert_keys.iter().cloned().collect();

        // 遍历所有边，寻找连接到移动顶点的垂直边
        for (_, edge) in &self.edges {
            let v1 = match self.verts.get(edge.v1) {
                Some(v) => v,
                None => continue,
            };
            let v2 = match self.verts.get(edge.v2) {
                Some(v) => v,
                None => continue,
            };

            // 检查这条边是否有一个顶点在移动轴上，另一个不在
            let v1_on_move_axis = moving_set.contains(&edge.v1);
            let v2_on_move_axis = moving_set.contains(&edge.v2);

            match (v1_on_move_axis, v2_on_move_axis) {
                (true, false) => {
                    // v1在移动轴上，v2不在 - 这是一条垂直于移动轴的边
                    let constraint = self.calculate_edge_constraint(
                        dir_axis_horizontal,
                        origval,
                        v2,
                        min_area_size,
                    );
                    bigger = bigger.min(constraint.0);
                    smaller = smaller.min(constraint.1);
                }
                (false, true) => {
                    // v2在移动轴上，v1不在 - 这是一条垂直于移动轴的边
                    let constraint = self.calculate_edge_constraint(
                        dir_axis_horizontal,
                        origval,
                        v1,
                        min_area_size,
                    );
                    bigger = bigger.min(constraint.0);
                    smaller = smaller.min(constraint.1);
                }
                _ => {
                    // 与移动轴不相关或自身是移动轴
                    ()
                }
            }
        }

        (bigger, smaller)
    }
}
