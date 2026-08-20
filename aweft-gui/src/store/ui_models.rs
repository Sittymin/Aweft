use log::warn;
use slint::{Model, VecModel};
use std::{collections::HashMap, rc::Rc};

use crate::bridge::types::{
    ScrAreaData, ScrEdgeTouchAreaData, ScrEdgeTouchAreaDisplayData, slint_area_key_to_key,
};
use crate::screen::area::ScrAreaKey;
use crate::screen::edge::ScrEdgeKey;
use crate::store::animation::AnimationState;
use crate::store::sequence::SequenceState;
use crate::store::node_graph::NodeGraphState;

/// 汇总所有需要在 Rust 侧持久化的 UI 模型。
pub struct UiModels {
    pub sequence_state: SequenceState,
    /// 样式图编辑器状态（节点/连线/链/角色）
    pub style_graph: NodeGraphState,
    /// 应用图编辑器状态
    pub app_graph: NodeGraphState,
    /// 动画域聚合状态：曲线/摄影表（graph）+ NLA（nla）共享 DataId::Animation，
    /// 因 Area 表现形式不同保留两组派生状态，TagKind 隔离。
    pub animation: AnimationState,
    pub scrarea_model: Rc<VecModel<ScrAreaData>>,
    pub scredge_touch_display_model: Rc<VecModel<ScrEdgeTouchAreaDisplayData>>,
    pub scredge_touch_model: Rc<VecModel<ScrEdgeTouchAreaData>>,
    pub scrarea_index_map: HashMap<ScrAreaKey, usize>,
    // 显示的 Edge 是合并过后的，所以可能存在 多个 Key 对应一个 Index 的情况
    pub scredge_touch_display_index_map: HashMap<ScrEdgeKey, usize>,
    pub scredge_touch_index_map: HashMap<ScrEdgeKey, usize>,
}

impl UiModels {
    pub fn new() -> Self {
        Self {
            sequence_state: SequenceState::new(),
            style_graph: NodeGraphState::default(),
            app_graph: NodeGraphState::default(),
            animation: AnimationState::default(),
            scrarea_model: Rc::new(VecModel::default()),
            scredge_touch_display_model: Rc::new(VecModel::default()),
            scredge_touch_model: Rc::new(VecModel::default()),
            scrarea_index_map: HashMap::new(),
            scredge_touch_display_index_map: HashMap::new(),
            scredge_touch_index_map: HashMap::new(),
        }
    }

    /// 提供 ScreenArea 的模型。
    pub fn scrareas(&self) -> Rc<VecModel<ScrAreaData>> {
        self.scrarea_model.clone()
    }

    /// 提供 EdgeDisplay 模型。
    pub fn scredge_displays(&self) -> Rc<VecModel<ScrEdgeTouchAreaDisplayData>> {
        self.scredge_touch_display_model.clone()
    }

    /// 提供 EdgeTouchArea 模型。
    pub fn scredge_touch_areas(&self) -> Rc<VecModel<ScrEdgeTouchAreaData>> {
        self.scredge_touch_model.clone()
    }

    /// 以全量向量刷新 ScreenArea 模型，同时维护 key->index 映射。
    pub fn set_scrareas(&mut self, areas: Vec<ScrAreaData>) {
        self.scrarea_index_map.clear();
        for (idx, area) in areas.iter().enumerate() {
            let key: ScrAreaKey = slint_area_key_to_key(&area.area_key);
            self.scrarea_index_map.insert(key, idx);
        }
        self.scrarea_model.set_vec(areas);
    }

    /// 全量刷新 EdgeDisplay 模型。
    pub fn set_scredge_displays(&mut self, edges: Vec<ScrEdgeTouchAreaDisplayData>) {
        self.scredge_touch_display_index_map.clear();
        for (idx, edge) in edges.iter().enumerate() {
            for (_, edge_key) in edge.edge_keys.iter().enumerate() {
                let key: ScrEdgeKey = slint_area_key_to_key(&edge_key);
                self.scredge_touch_display_index_map.insert(key, idx);
            }
        }
        self.scredge_touch_display_model.set_vec(edges);
    }

    /// 全量刷新 EdgeTouchArea 模型。
    pub fn set_scredge_touch_areas(&mut self, edges: Vec<ScrEdgeTouchAreaData>) {
        self.scredge_touch_index_map.clear();
        for (idx, edge) in edges.iter().enumerate() {
            for (_, edge_key) in edge.edge_keys.iter().enumerate() {
                let key: ScrEdgeKey = slint_area_key_to_key(&edge_key);
                self.scredge_touch_index_map.insert(key, idx);
            }
        }
        self.scredge_touch_model.set_vec(edges);
    }

    // 局部更新单个 ScreenArea 的类型
    pub fn update_area_type(
        &mut self,
        key: &ScrAreaKey,
        new_area_type: crate::bridge::types::SlintAreaType,
    ) -> bool {
        if let Some(&index) = self.scrarea_index_map.get(key) {
            if let Some(mut data) = self.scrarea_model.row_data(index) {
                data.area_type = new_area_type;
                log::debug!("已更新model");
                self.scrarea_model.set_row_data(index, data);
                return true;
            }
        }
        log::warn!("尝试更新不存在的 area key: {:?}", key);
        false
    }

    /// 局部更新单个 ScreenArea 的几何数据（位置/尺寸等）
    pub fn update_scrarea_geometry(&mut self, key: &ScrAreaKey, new_data: ScrAreaData) -> bool {
        if let Some(&index) = self.scrarea_index_map.get(key) {
            self.scrarea_model.set_row_data(index, new_data);
            return true;
        }
        warn!("尝试更新不存在的 area key: {:?}", key);
        false
    }

    /// 按已知索引更新 EdgeDisplay 行（resize/缩放局部几何更新）。
    pub fn set_scredge_display_at(&mut self, index: usize, data: ScrEdgeTouchAreaDisplayData) {
        self.scredge_touch_display_model.set_row_data(index, data);
    }

    /// 按已知索引更新 EdgeTouchArea 行（resize/缩放局部几何更新）。
    pub fn set_scredge_touch_at(&mut self, index: usize, data: ScrEdgeTouchAreaData) {
        self.scredge_touch_model.set_row_data(index, data);
    }

    /// 按 key 更新 ScreenArea 行（返回该行索引，供 edges 同步更新）。
    pub fn update_scrarea(&mut self, key: &ScrAreaKey, data: ScrAreaData) -> Option<usize> {
        let index = *self.scrarea_index_map.get(key)?;
        self.scrarea_model.set_row_data(index, data);
        Some(index)
    }

    /// 在指定索引插入一个 ScreenArea 行（`row_added`，存量组件复用），
    /// 并平移 index map 中 index 及其后的项。
    pub fn insert_scrarea_at(&mut self, index: usize, data: ScrAreaData) {
        self.scrarea_model.insert(index, data.clone());
        let key: ScrAreaKey = slint_area_key_to_key(&data.area_key);
        for (&k, &v) in self.scrarea_index_map.clone().iter() {
            if v >= index {
                self.scrarea_index_map.insert(k, v + 1);
            }
        }
        self.scrarea_index_map.insert(key, index);
    }

    /// 移除指定索引的 ScreenArea 行（`row_removed`，存量组件复用），
    /// 并平移 index map 中 index 之后的项。
    pub fn remove_scrarea_at(&mut self, index: usize) {
        let removed_key: Option<ScrAreaKey> = self
            .scrarea_model
            .row_data(index)
            .map(|d| slint_area_key_to_key(&d.area_key));
        self.scrarea_model.remove(index);
        if let Some(key) = removed_key {
            self.scrarea_index_map.remove(&key);
        }
        for (&k, &v) in self.scrarea_index_map.clone().iter() {
            if v > index {
                self.scrarea_index_map.insert(k, v - 1);
            }
        }
    }

    /// 当前 ScreenArea 行数。
    pub fn scrarea_count(&self) -> usize {
        self.scrarea_model.row_count()
    }

    /// 按 key 查找 ScreenArea 行索引。
    pub fn scrarea_index_of(&self, key: &ScrAreaKey) -> Option<usize> {
        self.scrarea_index_map.get(key).copied()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::bridge::types::{SlintAreaType, key_to_slint_key};

    fn make_area(key: ScrAreaKey) -> ScrAreaData {
        ScrAreaData {
            area_key: key_to_slint_key(&key),
            area_type: SlintAreaType::LineEditor,
            x: 0.0,
            y: 0.0,
            width: 100.0,
            height: 100.0,
            actionzone_side_length: 10.0,
        }
    }

    /// 构造唯一 ScrAreaKey（用 KeyData::from_ffi）。
    fn unique_key(n: u64) -> ScrAreaKey {
        slotmap::KeyData::from_ffi(n).into()
    }

    #[test]
    fn insert_shift_index_map() {
        let mut ui = UiModels::new();
        let a: ScrAreaKey = unique_key(1);
        let b: ScrAreaKey = unique_key(2);
        let c: ScrAreaKey = unique_key(3);

        ui.set_scrareas(vec![make_area(a), make_area(b)]);
        assert_eq!(ui.scrarea_index_of(&a), Some(0));
        assert_eq!(ui.scrarea_index_of(&b), Some(1));

        // 在 index 1 插入 c → b 平移到 2
        ui.insert_scrarea_at(1, make_area(c));
        assert_eq!(ui.scrarea_count(), 3);
        assert_eq!(ui.scrarea_index_of(&a), Some(0));
        assert_eq!(ui.scrarea_index_of(&c), Some(1));
        assert_eq!(ui.scrarea_index_of(&b), Some(2));
        // 模型顺序一致
        assert_eq!(
            ui.scrarea_model
                .row_data(0)
                .map(|d| slint_area_key_to_key(&d.area_key)),
            Some(a)
        );
        assert_eq!(
            ui.scrarea_model
                .row_data(2)
                .map(|d| slint_area_key_to_key(&d.area_key)),
            Some(b)
        );
    }

    #[test]
    fn remove_shift_index_map() {
        let mut ui = UiModels::new();
        let a: ScrAreaKey = unique_key(1);
        let b: ScrAreaKey = unique_key(2);
        let c: ScrAreaKey = unique_key(3);

        ui.set_scrareas(vec![make_area(a), make_area(b), make_area(c)]);
        assert_eq!(ui.scrarea_count(), 3);

        // 移除 index 1 (b) → c 平移到 1
        ui.remove_scrarea_at(1);
        assert_eq!(ui.scrarea_count(), 2);
        assert_eq!(ui.scrarea_index_of(&a), Some(0));
        assert_eq!(ui.scrarea_index_of(&b), None, "被移除 key 应从 map 删除");
        assert_eq!(ui.scrarea_index_of(&c), Some(1));
        assert_eq!(
            ui.scrarea_model
                .row_data(1)
                .map(|d| slint_area_key_to_key(&d.area_key)),
            Some(c)
        );
    }
}
