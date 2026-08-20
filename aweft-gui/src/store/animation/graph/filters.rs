//! 过滤视图：曲线编辑器 / 摄影表各自的通道过滤（FilterModel）+ 搜索。

use std::rc::Rc;

use slint::{Model, ModelExt, ModelRc};

use super::GraphEditorState;
use crate::bridge::types::{SlintCurveSegment, SlintFCurveChannel, SlintHandleMarker};
use crate::depsgraph::Tag;

impl GraphEditorState {
    /// 重建曲线编辑器 / 摄影表各自的通道过滤视图，以及曲线编辑器画布的段/手柄过滤视图
    /// （都通过 ModelExt::filter 语法糖构造 FilterModel，自动跟随底层模型变化）。
    pub(super) fn rebuild_filters(&mut self) {
        let g = self.graph_search.clone();
        self.graph_filter = ModelRc::from(Rc::new(self.channels.clone().filter(
            move |c: &SlintFCurveChannel| {
                let needle = g.trim().to_lowercase();
                needle.is_empty() || c.name.to_lowercase().contains(&needle)
            },
        )));
        let d = self.dope_search.clone();
        self.dope_filter = ModelRc::from(Rc::new(self.channels.clone().filter(
            move |c: &SlintFCurveChannel| {
                let needle = d.trim().to_lowercase();
                needle.is_empty() || c.name.to_lowercase().contains(&needle)
            },
        )));

        // 曲线编辑器画布：只显示匹配搜索通道的段/手柄（关键帧走 graph-channels）
        self.graph_matched = (0..self.graph_filter.row_count())
            .filter_map(|i| {
                self.graph_filter
                    .row_data(i)
                    .map(|c| c.curve_index as usize)
            })
            .collect();
        // 摄影表：记录当前匹配的通道（连接线增量更新用）
        self.dope_matched = (0..self.dope_filter.row_count())
            .filter_map(|i| self.dope_filter.row_data(i).map(|c| c.curve_index as usize))
            .collect();
        let seg_matched = self.graph_matched.clone();
        self.graph_segments =
            ModelRc::from(Rc::new(self.segments.clone().filter(
                move |s: &SlintCurveSegment| seg_matched.contains(&(s.curve_index as usize)),
            )));
        let hdl_matched = self.graph_matched.clone();
        self.graph_handles =
            ModelRc::from(Rc::new(self.handles.clone().filter(
                move |h: &SlintHandleMarker| hdl_matched.contains(&(h.curve_index as usize)),
            )));
    }

    /// 曲线编辑器搜索：仅影响曲线编辑器自己的过滤视图（与摄影表互不干扰）。
    /// 只改数据层，过滤视图重建由投影（Structure tag）完成。
    pub fn set_graph_search(&mut self, text: &str) -> Vec<Tag> {
        self.graph_search = text.to_string();
        vec![Tag::GraphStructureChanged]
    }

    /// 摄影表搜索：仅影响摄影表自己的过滤视图。
    /// 连接线增量更新：只重建"进入/离开匹配集合"的通道的连接，其余不动。
    pub fn set_dope_search(&mut self, text: &str) -> Vec<Tag> {
        self.dope_search = text.to_string();
        vec![Tag::GraphStructureChanged]
    }
}
