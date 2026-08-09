//! NLA 编辑器 Rust 侧：持有 NLA 轨道视图模型与全部编辑操作。
//!
//! UI 先行阶段：数据为临时测试数据（`window_manager::create_test_animation_data` 填充），
//! 尚未接线 aweft-model 的 `animation::AnimData/NlaTrack/NlaStrip`。
//! 结构对应：`SlintNlaTrack` ↔ `NlaTrack`，`SlintNlaStrip` ↔ `NlaStrip`（时间范围 + 类型）。

use std::collections::HashMap;
use std::rc::Rc;

use slint::{Model, ModelRc, SharedString, VecModel};

use crate::bridge::types::{SlintNlaStrip, SlintNlaTrack};

/// 片断拖拽起点（按下时记录）
#[derive(Debug, Clone, Copy)]
struct StripDrag {
    track: usize,
    strip: usize,
    orig_start: f32,
    orig_end: f32,
}

/// NLA 编辑器状态
pub struct NlaEditorState {
    /// 轨道视图模型（绑定 Slint；`strips` 指向 `strip_models` 中同一 inner 模型）
    pub tracks: Rc<VecModel<SlintNlaTrack>>,
    /// 当前时间（播放头，与曲线编辑器共享）
    pub current_time: f64,

    /// 权威片断数据：轨道下标 → 稳定的 inner VecModel
    strip_models: HashMap<usize, Rc<VecModel<SlintNlaStrip>>>,

    drag: Option<StripDrag>,
}

impl Default for NlaEditorState {
    fn default() -> Self {
        Self {
            tracks: Rc::new(VecModel::default()),
            current_time: 0.0,
            strip_models: HashMap::new(),
            drag: None,
        }
    }
}

impl NlaEditorState {
    /// 追加一个轨道（含片断）。
    pub fn push_track(&mut self, name: impl Into<SharedString>, strips: Vec<SlintNlaStrip>) {
        let idx = self.strip_models.len();
        let inner: Rc<VecModel<SlintNlaStrip>> = Rc::new(VecModel::from(strips));
        self.strip_models.insert(idx, Rc::clone(&inner));
        self.tracks.push(SlintNlaTrack {
            name: name.into(),
            muted: false,
            solo: false,
            strips: ModelRc::from(inner),
        });
    }

    /// 清除全部片断选中。
    pub fn deselect_all(&mut self) {
        for model in self.strip_models.values() {
            for i in 0..model.row_count() {
                if let Some(mut s) = model.row_data(i) {
                    if s.selected {
                        s.selected = false;
                        model.set_row_data(i, s);
                    }
                }
            }
        }
    }

    /// 片断按下：选中（additive=Shift/Ctrl 追加）并记录拖拽起点。
    pub fn strip_pressed(&mut self, track: usize, strip: usize, additive: bool) {
        let Some(s) = self.strip_models.get(&track).and_then(|m| m.row_data(strip)) else {
            return;
        };
        if !additive {
            self.deselect_all();
        }
        if let Some(model) = self.strip_models.get(&track) {
            if let Some(mut s) = model.row_data(strip) {
                s.selected = if additive { !s.selected } else { true };
                model.set_row_data(strip, s);
            }
        }
        self.drag = Some(StripDrag {
            track,
            strip,
            orig_start: s.start,
            orig_end: s.end,
        });
    }

    /// 片断横向拖拽：整体平移（新位置 = 按下时位置 + delta）。
    pub fn strip_dragged(&mut self, track: usize, strip: usize, delta_x: f32) {
        if !delta_x.is_finite() {
            return;
        }
        let (orig_start, orig_end) = match self.drag {
            Some(d) if d.track == track && d.strip == strip => (d.orig_start, d.orig_end),
            _ => return,
        };
        let Some(model) = self.strip_models.get(&track) else {
            return;
        };
        if let Some(mut s) = model.row_data(strip) {
            s.start = orig_start + delta_x;
            s.end = orig_end + delta_x;
            model.set_row_data(strip, s);
        }
    }

    /// 片断松开：清空拖拽起点。
    pub fn strip_released(&mut self, track: usize, strip: usize) {
        if let Some(d) = self.drag {
            if d.track == track && d.strip == strip {
                self.drag = None;
            }
        }
    }

    /// 轨道静音开关。
    pub fn toggle_track_muted(&mut self, track: usize) {
        if let Some(mut t) = self.tracks.row_data(track) {
            t.muted = !t.muted;
            self.tracks.set_row_data(track, t);
        }
    }

    /// 轨道独奏开关。
    pub fn toggle_track_solo(&mut self, track: usize) {
        if let Some(mut t) = self.tracks.row_data(track) {
            t.solo = !t.solo;
            self.tracks.set_row_data(track, t);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::bridge::types::{SlintNlaStrip, SlintStripType};
    use slint::Model;

    fn strip(name: &str, start: f32, end: f32) -> SlintNlaStrip {
        SlintNlaStrip {
            name: name.into(),
            strip_type: SlintStripType::Action,
            start,
            end,
            color: slint::Color::from_rgb_u8(99, 199, 99),
            muted: false,
            selected: false,
        }
    }

    #[test]
    fn strip_drag_moves_whole_strip() {
        let mut st = NlaEditorState::default();
        st.push_track("t", vec![strip("A", 1.0, 4.0), strip("B", 6.0, 9.0)]);
        st.tracks.row_data(0).unwrap();

        st.strip_pressed(0, 0, false);
        st.strip_dragged(0, 0, 2.0);

        let s = st.strip_models[&0].row_data(0).unwrap();
        assert_eq!(s.start, 3.0);
        assert_eq!(s.end, 6.0);
        assert!(s.selected);

        st.strip_released(0, 0);
        assert!(st.drag.is_none());
    }

    #[test]
    fn strip_pressed_selection_is_single() {
        let mut st = NlaEditorState::default();
        st.push_track("t", vec![strip("A", 1.0, 4.0), strip("B", 6.0, 9.0)]);

        st.strip_pressed(0, 0, false);
        st.strip_pressed(0, 1, false);

        assert!(st.strip_models[&0].row_data(1).unwrap().selected);
        assert!(!st.strip_models[&0].row_data(0).unwrap().selected);
    }

    #[test]
    fn track_toggles_update_rows() {
        let mut st = NlaEditorState::default();
        st.push_track("t", vec![]);

        st.toggle_track_muted(0);
        st.toggle_track_solo(0);
        let t = st.tracks.row_data(0).unwrap();
        assert!(t.muted);
        assert!(t.solo);
    }
}
