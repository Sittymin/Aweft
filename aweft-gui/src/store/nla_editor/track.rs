use super::{NlaEditorState, NlaStripData, StripDrag};
use crate::bridge::types::{SlintNlaStrip, SlintNlaTrack};
use crate::depsgraph::Tag;
use aweft_model::{animation::strip::NlaStrip, ids::ActionId};
use slint::{ModelRc, SharedString};

impl NlaEditorState {
    /// 追加一个轨道（含片断）。装载/初始化用：改数据层并同步派生层（结构变化全量重建）。
    pub fn push_track(
        &mut self,
        name: impl Into<SharedString>,
        strips: Vec<SlintNlaStrip>,
        is_action: bool,
    ) {
        let idx = self.track_view.len();
        self.track_view.push(SlintNlaTrack {
            name: name.into(),
            enabled: true,
            locked: false,
            strips: ModelRc::default(),
            is_action,
            has_data: false,
            orig_index: idx as i32,
        });
        self.editing_strips.push(
            strips
                .into_iter()
                .map(|view| {
                    let model = NlaStrip::new_action(view.name.to_string(), ActionId::default());
                    NlaStripData { view, model }
                })
                .collect(),
        );
        self.apply_structure();
    }

    /// 找第一条可编辑且能容纳 [start, end]（不与任何片断重叠）的轨道。
    /// 排除全局 Action 轨（is_action，只读）与锁定轨。
    pub fn find_fit_track(&self, start: f32, end: f32) -> Option<usize> {
        for i in 0..self.track_view.len() {
            let editable = !self.track_view[i].is_action && !self.track_view[i].locked;
            if !editable {
                continue;
            }
            let fits = self.editing_strips[i]
                .iter()
                .all(|s| !(start < s.view.end && end > s.view.start));
            if fits {
                return Some(i);
            }
        }
        None
    }

    /// 把一条片断按 start 升序插入数据层（`track` 轨道），返回插入下标。
    pub fn insert_strip(&mut self, track: usize, entry: NlaStripData) -> usize {
        let list = &mut self.editing_strips[track];
        let start = entry.view.start;
        let idx = list
            .iter()
            .position(|s| start < s.view.start)
            .unwrap_or(list.len());
        list.insert(idx, entry);
        idx
    }

    /// 当前轨道数（含第 0 轨全局 Action 轨）。
    pub fn track_count(&self) -> usize {
        self.track_view.len()
    }

    /// 轨道启用开关（`index` 为权威轨道下标；UI 回传 orig-index）。
    /// model 的 `NlaTrack.muted` 同步由 reducer 完成（enabled = !muted）。
    pub fn toggle_track_enabled(&mut self, index: usize) -> Vec<Tag> {
        if let Some(t) = self.track_view.get_mut(index) {
            if !t.is_action {
                t.enabled = !t.enabled;
                return vec![Tag::NlaTrackChanged { track: index }];
            }
        }
        Vec::new()
    }

    /// 轨道锁定开关（`index` 为权威轨道下标；UI 回传 orig-index）。锁定纯 UI 态。
    pub fn toggle_track_locked(&mut self, index: usize) -> Vec<Tag> {
        if let Some(t) = self.track_view.get_mut(index) {
            if !t.is_action {
                t.locked = !t.locked;
                return vec![Tag::NlaTrackChanged { track: index }];
            }
        }
        Vec::new()
    }

    /// 目标轨道是否可作为跨轨道移动的落点：存在、非全局 Action 轨（只读）、非锁定。
    pub(crate) fn can_move_to(&self, target: usize) -> bool {
        self.track_view
            .get(target)
            .map(|t| !t.is_action && !t.locked)
            .unwrap_or(false)
    }
}
