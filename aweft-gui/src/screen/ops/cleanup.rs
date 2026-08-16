use crate::screen::{
    area::ScrAreaKey,
    edge::ScrEdgeKey,
    map::ScrAreaMap,
    vertex::{ScrVertKey, VertPosition},
};

impl ScrAreaMap {
    /// 清理重复的边（参考Blender的BKE_screen_remove_double_scredges）
    pub fn remove_duplicate_edges(&mut self) {
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
    pub fn remove_unused_edges(&mut self) {
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
            log::debug!("清理了 {} 条未使用的边", removed_count);
        }
    }

    /// 清理完全重叠的顶点，将它们合并为同一个顶点
    /// 合并位置重叠的顶点。返回拓扑是否实际变化（合并了顶点 → true）。
    pub fn remove_duplicate_vertices(&mut self) -> bool {
        use std::collections::HashMap;

        // 1. 按位置分组顶点
        let mut position_groups: HashMap<(u32, u32), Vec<ScrVertKey>> = HashMap::new();
        for (vert_key, vert) in &self.verts {
            let pos = (vert.vec.x, vert.vec.y);
            position_groups.entry(pos).or_default().push(vert_key);
        }

        // 2. 找出有重叠的位置（超过1个顶点）
        let mut vertex_replacements: HashMap<ScrVertKey, ScrVertKey> = HashMap::new();
        let mut vertices_to_remove: Vec<ScrVertKey> = Vec::new();

        for (_pos, mut vert_keys) in position_groups {
            if vert_keys.len() > 1 {
                // 按键排序，选择第一个作为代表顶点
                vert_keys.sort();
                let representative = vert_keys[0];

                log::debug!(
                    "合并重叠顶点 {:?} 到代表顶点 {:?}",
                    &vert_keys[1..],
                    representative
                );

                // 记录需要替换的映射
                for &duplicate in &vert_keys[1..] {
                    vertex_replacements.insert(duplicate, representative);
                    vertices_to_remove.push(duplicate);
                }
            }
        }

        if vertex_replacements.is_empty() {
            return false; // 没有重叠顶点，直接返回
        }

        // 3. 更新所有边的顶点引用
        for (_edge_key, edge) in &mut self.edges {
            if let Some(&new_v1) = vertex_replacements.get(&edge.v1) {
                log::debug!(
                    "边 {:?}: v1 从 {:?} 替换为 {:?}",
                    _edge_key,
                    edge.v1,
                    new_v1
                );
                edge.v1 = new_v1;
            }
            if let Some(&new_v2) = vertex_replacements.get(&edge.v2) {
                log::debug!(
                    "边 {:?}: v2 从 {:?} 替换为 {:?}",
                    _edge_key,
                    edge.v2,
                    new_v2
                );
                edge.v2 = new_v2;
            }
        }

        // 4. 更新所有区域的顶点引用
        let area_keys: Vec<ScrAreaKey> = self.areas.keys().collect();
        for area_key in area_keys {
            // 收集需要更新的顶点
            let mut updates = Vec::new();
            {
                let area = &self.areas[area_key];

                if let Some(&new_tl) = vertex_replacements.get(&area.v_top_left()) {
                    log::debug!(
                        "区域 {:?}: v_top_left 从 {:?} 替换为 {:?}",
                        area_key,
                        area.v_top_left(),
                        new_tl
                    );
                    updates.push(VertPosition::TopLeft(new_tl));
                }
                if let Some(&new_tr) = vertex_replacements.get(&area.v_top_right()) {
                    log::debug!(
                        "区域 {:?}: v_top_right 从 {:?} 替换为 {:?}",
                        area_key,
                        area.v_top_right(),
                        new_tr
                    );
                    updates.push(VertPosition::TopRight(new_tr));
                }
                if let Some(&new_br) = vertex_replacements.get(&area.v_bottom_right()) {
                    log::debug!(
                        "区域 {:?}: v_bottom_right 从 {:?} 替换为 {:?}",
                        area_key,
                        area.v_bottom_right(),
                        new_br
                    );
                    updates.push(VertPosition::BottomRight(new_br));
                }
                if let Some(&new_bl) = vertex_replacements.get(&area.v_bottom_left()) {
                    log::debug!(
                        "区域 {:?}: v_bottom_left 从 {:?} 替换为 {:?}",
                        area_key,
                        area.v_bottom_left(),
                        new_bl
                    );
                    updates.push(VertPosition::BottomLeft(new_bl));
                }
            }

            // 应用更新
            for update in updates {
                let _ = self.set_area_vert(area_key, update);
            }
        }

        // 5. 删除被合并的顶点
        for vert_key in vertices_to_remove {
            self.verts.remove(vert_key);
            log::debug!("删除重叠顶点 {:?}", vert_key);
        }

        // 6. 清理可能产生的重复边和无效边
        self.remove_duplicate_edges();
        self.remove_unused_edges();

        log::debug!(
            "重叠顶点清理完成，共合并了 {} 个顶点",
            vertex_replacements.len()
        );
        true
    }
}

#[cfg(test)]
mod tests {
    use crate::bridge::types::ThemeSettings;
    use crate::screen::{
        area::{AreaType, ScrArea},
        map::ScrAreaMap,
    };

    #[test]
    fn test_duplicate_edge_removal() {
        let mut screen_map = ScrAreaMap::new();

        // 创建顶点
        let v1 = screen_map.add_vertex_to_geometry(0, 0);
        let v2 = screen_map.add_vertex_to_geometry(100, 0);

        // 由于add_edge_to_geometry会检查重复，我们需要直接插入重复边来测试清理功能
        // 手动创建两条相同的边
        let edge1 = screen_map.edges.insert(crate::screen::edge::ScrEdge {
            v1,
            v2,
            border: false,
            flag: 0,
        });
        let edge2 = screen_map.edges.insert(crate::screen::edge::ScrEdge {
            v1,
            v2,
            border: false,
            flag: 0,
        });

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
        let ui_settings = ThemeSettings::new(1.0, 1.0, 10.0, 5.0, 50);

        // 创建顶点
        let v1 = screen_map.add_vertex_to_geometry(0, 0);
        let v2 = screen_map.add_vertex_to_geometry(100, 0);
        let v3 = screen_map.add_vertex_to_geometry(100, 100);
        let v4 = screen_map.add_vertex_to_geometry(0, 100);

        // 创建区域
        let mut area = ScrArea::new(v1, v2, v3, v4, AreaType::Empty, true, false, false);
        area.calculate_totrct_and_size(&screen_map.verts, &ui_settings);
        let area_key = screen_map.areas.insert(area);

        // 创建区域使用的边
        screen_map.add_edge_to_geometry(v1, v2);
        screen_map.add_edge_to_geometry(v2, v3);
        screen_map.add_edge_to_geometry(v3, v4);
        screen_map.add_edge_to_geometry(v4, v1);

        // 创建一些未使用的边
        let v5 = screen_map.add_vertex_to_geometry(50, 50);
        screen_map.add_edge_to_geometry(v1, v5);
        screen_map.add_edge_to_geometry(v5, v3);

        // 验证初始状态
        assert_eq!(screen_map.edges.len(), 6);

        // 运行清理函数
        screen_map.remove_unused_edges();

        // 验证未使用的边被清理
        assert_eq!(screen_map.edges.len(), 4);
    }
}
