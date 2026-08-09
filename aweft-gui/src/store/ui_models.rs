use log::warn;
use slint::{Model, ModelRc, VecModel};
use std::{collections::HashMap, rc::Rc};

use crate::bridge::types::{
    ScrAreaData, ScrEdgeTouchAreaData, ScrEdgeTouchAreaDisplayData, SlintSubtitleRow,
    slint_area_key_to_key,
};
use crate::screen::area::ScrAreaKey;
use crate::screen::edge::ScrEdgeKey;
use crate::store::curve_editor::CurveEditorState;
use crate::store::graph_editor::GraphEditorState;
use crate::store::line_editor::LineEditorState;
use crate::store::nla_editor::NlaEditorState;

/// 汇总所有需要在 Rust 侧持久化的 UI 模型。
/// 目前只包含字幕编辑器的行模型，后续可以加入 screen/edge 等模型。
pub struct UiModels {
    pub line_editor_state: LineEditorState,
    pub scrarea_model: Rc<VecModel<ScrAreaData>>,
    pub scredge_touch_display_model: Rc<VecModel<ScrEdgeTouchAreaDisplayData>>,
    pub scredge_touch_model: Rc<VecModel<ScrEdgeTouchAreaData>>,
    pub scrarea_index_map: HashMap<ScrAreaKey, usize>,
    // 显示的 Edge 是合并过后的，所以可能存在 多个 Key 对应一个 Index 的情况
    pub scredge_touch_display_index_map: HashMap<ScrEdgeKey, usize>,
    pub scredge_touch_index_map: HashMap<ScrEdgeKey, usize>,
    // 节点图编辑器状态（样式图 / 应用图）
    pub style_graph: GraphEditorState,
    pub app_graph: GraphEditorState,
    // 曲线编辑器状态（Graph Editor / FCurve）
    pub curve_editor: CurveEditorState,
    // NLA 动画编辑器状态
    pub nla_editor: NlaEditorState,
}

impl UiModels {
    pub fn new() -> Self {
        Self {
            line_editor_state: LineEditorState::new(),
            scrarea_model: Rc::new(VecModel::default()),
            scredge_touch_display_model: Rc::new(VecModel::default()),
            scredge_touch_model: Rc::new(VecModel::default()),
            scrarea_index_map: HashMap::new(),
            scredge_touch_display_index_map: HashMap::new(),
            scredge_touch_index_map: HashMap::new(),
            style_graph: GraphEditorState::default(),
            app_graph: GraphEditorState::default(),
            curve_editor: CurveEditorState::default(),
            nla_editor: NlaEditorState::default(),
        }
    }

    /// 提供 Slint 端需要的行模型（便于初始化或重新绑定）。
    pub fn line_editor_rows(&self) -> Rc<VecModel<SlintSubtitleRow>> {
        self.line_editor_state.all_rows_model()
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
        println!("全量计算scrareas: {:#?}", self.scrarea_index_map);
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
                self.scredge_touch_display_index_map.insert(key, idx);
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
                println!("已更新model");
                self.scrarea_model.set_row_data(index, data);
                return true;
            }
        }
        println!("没更新model");
        warn!("尝试更新不存在的 area key: {:?}", key);
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

    /// 局部更新单个 EdgeDisplay
    pub fn update_scredge_display(
        &mut self,
        key: &ScrEdgeKey,
        new_data: ScrEdgeTouchAreaDisplayData,
    ) -> bool {
        if let Some(&index) = self.scredge_touch_display_index_map.get(key) {
            self.scredge_touch_display_model
                .set_row_data(index, new_data);
            return true;
        }
        warn!("尝试更新不存在的 edge key: {:?}", key);
        false
    }

    /// 局部更新单个 EdgeTouchArea
    pub fn update_scredge_touch(
        &mut self,
        key: &ScrEdgeKey,
        new_data: ScrEdgeTouchAreaData,
    ) -> bool {
        if let Some(&index) = self.scredge_touch_index_map.get(key) {
            self.scredge_touch_model.set_row_data(index, new_data);
            return true;
        }
        warn!("尝试更新不存在的 edge key: {:?}", key);
        false
    }
}
