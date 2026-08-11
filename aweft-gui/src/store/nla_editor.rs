//! NLA 编辑器 Rust 侧：持有 NLA 轨道视图模型与全部编辑操作。
//!
//! UI 先行阶段：数据为临时测试数据（`window_manager::create_test_animation_data` 填充），
//! 尚未接线 aweft-model 的 `animation::AnimData/NlaTrack/NlaStrip`。
//! 结构对应：`SlintNlaTrack` ↔ `NlaTrack`，`SlintNlaStrip` ↔ `NlaStrip`（时间范围 + 类型）。

use std::collections::HashMap;
use std::rc::Rc;

use slint::{Model, ModelRc, SharedString, VecModel};

use crate::bridge::types::{SlintNlaStrip, SlintNlaTrack, SlintStripType};

/// 片断拖拽状态。`track/strip` 为片段**当前**所在轨道/下标（拖动中会因瞬间跨轨变化），
/// `press_track/press_strip` 为按下时的（供 UI 回传的 released/dragged 匹配）。
#[derive(Debug, Clone, Copy)]
struct StripDrag {
    press_track: usize,
    press_strip: usize,
    track: usize,
    strip: usize,
    orig_start: f32,
    orig_end: f32,
}

/// NLA 编辑器状态
pub struct NlaEditorState {
    /// 权威轨道数据（含全部轨道；`strips` 指向 `strip_models` 中同一 inner 模型）
    pub tracks: Rc<VecModel<SlintNlaTrack>>,
    /// 搜索过滤后的轨道投影（绑定 Slint；画布/列表都显示它，行号 = 过滤视图行，
    /// 经 `orig-index` 映射回权威轨道）
    pub tracks_filter: Rc<VecModel<SlintNlaTrack>>,
    /// 当前时间（播放头，与曲线编辑器共享）
    pub current_time: f64,

    /// 搜索关键字（空 = 全部轨道）
    search: String,
    /// 权威片断数据：权威轨道下标 → 稳定的 inner VecModel
    strip_models: HashMap<usize, Rc<VecModel<SlintNlaStrip>>>,

    drag: Option<StripDrag>,
}

impl Default for NlaEditorState {
    fn default() -> Self {
        Self {
            tracks: Rc::new(VecModel::default()),
            tracks_filter: Rc::new(VecModel::default()),
            current_time: 0.0,
            search: String::new(),
            strip_models: HashMap::new(),
            drag: None,
        }
    }
}

impl NlaEditorState {
    /// 追加一个轨道（含片断）。
    pub fn push_track(&mut self, name: impl Into<SharedString>, strips: Vec<SlintNlaStrip>, is_action: bool) {
        let idx = self.strip_models.len();
        let inner: Rc<VecModel<SlintNlaStrip>> = Rc::new(VecModel::from(strips));
        self.strip_models.insert(idx, Rc::clone(&inner));
        self.tracks.push(SlintNlaTrack {
            name: name.into(),
            enabled: true,
            locked: false,
            strips: ModelRc::from(inner),
            is_action,
            has_data: false,
            orig_index: idx as i32,
        });
        self.refresh_filter();
    }

    /// 同步全局 Action 轨（第一轨 is-action）：名称用当前实际 Action 名，
    /// `has_data` 表示当前 Action 是否有关键帧（无数据时"向下插入"按钮隐藏）。
    /// 无名称（空 Action）时显示"<无动作>"。
    pub fn set_global_action(&mut self, name: impl Into<SharedString>, has_data: bool) {
        let name: SharedString = name.into();
        let display: SharedString = if name.trim().is_empty() {
            "<无动作>".into()
        } else {
            name
        };
        if let Some(mut t) = self.tracks.row_data(0) {
            if t.is_action {
                t.name = display;
                t.has_data = has_data;
                self.tracks.set_row_data(0, t);
                self.refresh_filter();
            }
        }
    }

    /// 按名字搜索轨道并重建过滤投影（空关键字 = 全部）。
    pub fn filter_tracks(&mut self, text: impl Into<SharedString>) {
        self.search = text.into().to_string();
        self.refresh_filter();
    }

    /// 由权威轨道 + 搜索关键字重建过滤投影。
    fn refresh_filter(&mut self) {
        let mut v: Vec<SlintNlaTrack> = Vec::new();
        for i in 0..self.tracks.row_count() {
            if let Some(t) = self.tracks.row_data(i) {
                if self.search.is_empty() || t.name.contains(&self.search) {
                    v.push(t);
                }
            }
        }
        self.tracks_filter.set_vec(v);
    }

    /// 轨道启用开关（`index` 为权威轨道下标；UI 回传 orig-index）。
    pub fn toggle_track_enabled(&mut self, index: usize) {
        if let Some(mut t) = self.tracks.row_data(index) {
            t.enabled = !t.enabled;
            self.tracks.set_row_data(index, t);
            self.refresh_filter();
        }
    }

    /// 轨道锁定开关（`index` 为权威轨道下标；UI 回传 orig-index）。
    pub fn toggle_track_locked(&mut self, index: usize) {
        if let Some(mut t) = self.tracks.row_data(index) {
            t.locked = !t.locked;
            self.tracks.set_row_data(index, t);
            self.refresh_filter();
        }
    }

    /// 全局 Action 轨"向下插入"（左侧列表的 move_to_inbox 按钮）：
    /// 把当前编辑的 Action 作为一条片断（Action 类型）加入 NLA——
    /// 先找一条可编辑轨道（非全局 Action、非锁定）中**能容纳**该时间范围 [start, end]
    ///（不与任何现有片断重叠）的，插入之；若没有则**新建一条轨道**再插入。
    pub fn push_down(&mut self, name: impl Into<SharedString>, start: f32, end: f32) {
        if !start.is_finite() || !end.is_finite() || end <= start {
            return;
        }
        let name: SharedString = name.into();
        let strip = SlintNlaStrip {
            name,
            strip_type: SlintStripType::Action,
            start,
            end,
            muted: false,
            selected: false,
            dragging: false,
        };
        if let Some(track) = self.find_fit_track(start, end) {
            let model = self.strip_models.get(&track).unwrap().clone();
            Self::insert_sorted(&model, strip);
            return;
        }
        // 没有能容纳的已有轨道 → 新建一条
        self.push_track("NLA 片段", vec![], false);
        let track = self.tracks.row_count() - 1;
        let model = self.strip_models.get(&track).unwrap().clone();
        Self::insert_sorted(&model, strip);
    }

    /// 找第一条可编辑且能容纳 [start, end]（不与任何片断重叠）的轨道。
    fn find_fit_track(&self, start: f32, end: f32) -> Option<usize> {
        for i in 0..self.tracks.row_count() {
            let editable = self
                .tracks
                .row_data(i)
                .map(|t| !t.is_action && !t.locked)
                .unwrap_or(false);
            if !editable {
                continue;
            }
            let model = self.strip_models.get(&i).unwrap();
            let mut fits = true;
            for j in 0..model.row_count() {
                if let Some(s) = model.row_data(j) {
                    if start < s.end && end > s.start {
                        fits = false;
                        break;
                    }
                }
            }
            if fits {
                return Some(i);
            }
        }
        None
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
            press_track: track,
            press_strip: strip,
            track,
            strip,
            orig_start: s.start,
            orig_end: s.end,
        });
    }

    /// 片断拖拽。锁定轨道不可编辑。
    ///
    /// `delta_x` 为世界时间位移；`delta_row` 为垂直位移（行数，正 = 向下）。
    /// 垂直：**瞬间跳轨**——片段中心（按下轨道中线 + delta_row）越过轨道边界即把片段
    /// 立即移到目标轨道（源模型 remove → 目标 push），不做平滑跟随；目标无效（越界/
    /// 全局 Action 轨/锁定轨）则保持当前轨道。
    /// 水平：与当前所在轨道其他片断**硬钳制**——片段不能与它们重叠，重叠时按候选
    /// 中点相对片断中点的位置，停到较近的一侧（可穿越到片断另一侧）。
    /// 片断拖拽。锁定轨道不可编辑。
    ///
    /// `delta_x` 为世界时间位移；`delta_row` 为垂直位移（行数，正 = 向下）。
    /// 垂直：**瞬间跳轨**——片段中心（按下轨道中线 + delta_row）越过轨道边界即把片段
    /// 立即移到目标轨道（按 start 排序插入），不做平滑跟随；目标无效（越界/全局
    /// Action 轨/锁定轨）则保持当前轨道。
    /// 水平：与当前所在轨道其他片断**硬钳制**——重叠时按候选中点相对片断中点停到
    /// 较近的一侧（可穿越）；若两片断间空隙放不下，则不塞入（保持片段当前位置）。
    /// 拖动中保持模型按 start 排序（否则穿越后延伸条带/空隙归属错乱）。
    pub fn strip_dragged(&mut self, track: usize, strip: usize, delta_x: f32, delta_row: f32) {
        if !delta_x.is_finite() || !delta_row.is_finite() {
            return;
        }
        let Some(mut d) = self.drag else { return };
        if d.press_track != track || d.press_strip != strip {
            return;
        }
        if self.tracks.row_data(d.track).map(|t| t.locked).unwrap_or(false) {
            return;
        }
        let width = d.orig_end - d.orig_start;

        // 垂直：目标轨道（基于按下轨道 + 绝对位移）；不同且有效 → 立即跨轨（按 start 插入）
        let target = self.drag_target_row(d.press_track, delta_row);
        if target != d.track && self.can_move_to(target) {
            let src = self.strip_models.get(&d.track).unwrap().clone();
            if d.strip < src.row_count() {
                let s = src.remove(d.strip);
                let dst = self.strip_models.get(&target).unwrap().clone();
                d.strip = Self::insert_sorted(&dst, s);
                d.track = target;
            }
        }

        // 水平：钳制到当前所在轨道的空隙（fallback = 片段当前实际位置，空隙不够时不塞入）
        let model = self.strip_models.get(&d.track).unwrap().clone();
        let fallback = model
            .row_data(d.strip)
            .map(|s| s.start)
            .unwrap_or(d.orig_start + delta_x);
        let raw_start = d.orig_start + delta_x;
        let start = self.clamp_no_overlap(d.track, d.strip, raw_start, width, fallback);
        if let Some(mut s) = model.row_data(d.strip) {
            s.start = start;
            s.end = start + width;
            s.dragging = true;
            model.set_row_data(d.strip, s);
        }
        // 保持模型按 start 排序（穿越后延伸/空隙按时间顺序计算）
        d.strip = self.reinsert_sorted(d.track, d.strip);
        self.drag = Some(d);
    }

    /// 垂直位移后片段中心所在的轨道（行号）：中心 = 按下轨道中线 + delta_row，
    /// 片段中心越过轨道边界（|delta_row| > 0.5）即进入相邻轨道；越界钳到两端。
    fn drag_target_row(&self, press_row: usize, delta_row: f32) -> usize {
        let center = press_row as f32 + 0.5 + delta_row;
        let target = center.floor().max(0.0) as usize;
        target.min(self.tracks.row_count().saturating_sub(1))
    }

    /// 把候选 [start, start+width] 钳制到 `track` 轨道的空隙内（不与其他片断重叠）。
    /// 与片断重叠时按**候选中点相对片断中点**停到较近的一侧（可穿越到片断另一侧）；
    /// 但若两片断之间的空隙放不下候选（迭代钳制后仍与某片断重叠），则**不塞入**，
    /// 回退到 `fallback`（片段保持当前位置）。
    fn clamp_no_overlap(
        &self,
        track: usize,
        ignore_strip: usize,
        start: f32,
        width: f32,
        fallback: f32,
    ) -> f32 {
        let Some(model) = self.strip_models.get(&track) else {
            return start;
        };
        let mut s = start;
        for _ in 0..model.row_count() + 1 {
            let mut moved = false;
            for j in 0..model.row_count() {
                if j == ignore_strip {
                    continue;
                }
                if let Some(o) = model.row_data(j) {
                    if s < o.end && s + width > o.start {
                        let cand_mid = s + width / 2.0;
                        let o_mid = (o.start + o.end) / 2.0;
                        if cand_mid < o_mid {
                            s = o.start - width;
                        } else {
                            s = o.end;
                        }
                        moved = true;
                        break;
                    }
                }
            }
            if !moved {
                break;
            }
        }
        // 空隙不够（迭代后仍与片断重叠）→ 不塞入，回退到片段当前位置
        for j in 0..model.row_count() {
            if j == ignore_strip {
                continue;
            }
            if let Some(o) = model.row_data(j) {
                if s < o.end && s + width > o.start {
                    return fallback;
                }
            }
        }
        s
    }

    /// 把 `strip` 按 start 升序插入 `model`，返回插入下标。
    fn insert_sorted(model: &Rc<VecModel<SlintNlaStrip>>, strip: SlintNlaStrip) -> usize {
        let mut idx = model.row_count();
        for i in 0..model.row_count() {
            if let Some(x) = model.row_data(i) {
                if strip.start < x.start {
                    idx = i;
                    break;
                }
            }
        }
        model.insert(idx, strip);
        idx
    }

    /// 把 `track` 轨道的 `old_idx` 片断取出后按 start 重新插入，返回新下标。
    fn reinsert_sorted(&self, track: usize, old_idx: usize) -> usize {
        let model = self.strip_models.get(&track).unwrap().clone();
        let s = model.remove(old_idx);
        Self::insert_sorted(&model, s)
    }

    /// 画布级 TouchArea 左键命中检测：世界时间 x 命中行内某片断则选中并记录拖拽起点。
    /// 返回编码 auth_track*10000+strip（命中）或 -1（未命中）。命中检测用片断的整数毫秒
    /// 时间范围，行由 Slint 按 (y-pan-y)/row-height 换算好传入（过滤视图行号）。
    /// 全局 Action 轨（第一轨）与锁定轨道只能看不能编辑，命中直接返回 -1。
    pub fn strip_pick(&mut self, x: f32, row: i32, mode: i32) -> i32 {
        if row < 0 {
            return -1;
        }
        let Some(t) = self.tracks_filter.row_data(row as usize) else {
            return -1;
        };
        if t.is_action || t.locked {
            return -1;
        }
        let auth = t.orig_index as usize;
        let Some(model) = self.strip_models.get(&auth) else {
            return -1;
        };
        for j in 0..model.row_count() {
            if let Some(s) = model.row_data(j) {
                if x >= s.start && x <= s.end {
                    self.strip_pressed(auth, j, mode != 0);
                    return (auth as i32 * 10000 + j as i32);
                }
            }
        }
        -1
    }

    /// 片断松开：跨轨道已在拖动中瞬间完成，这里只退出"移动中"高亮并清空拖拽状态。
    /// `track/strip` 为按下时传回的编码，按 `press_track/press_strip` 匹配。
    pub fn strip_released(&mut self, track: usize, strip: usize) {
        let Some(d) = self.drag else {
            return;
        };
        if d.press_track != track || d.press_strip != strip {
            return;
        }
        if let Some(model) = self.strip_models.get(&d.track) {
            if let Some(mut s) = model.row_data(d.strip) {
                if s.dragging {
                    s.dragging = false;
                    model.set_row_data(d.strip, s);
                }
            }
        }
        self.drag = None;
    }

    /// 目标轨道是否可作为跨轨道移动的落点：存在、非全局 Action 轨（只读）、非锁定。
    fn can_move_to(&self, target: usize) -> bool {
        if target >= self.tracks.row_count() {
            return false;
        }
        self.tracks
            .row_data(target)
            .map(|t| !t.is_action && !t.locked)
            .unwrap_or(false)
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
            muted: false,
            selected: false,
            dragging: false,
        }
    }

    #[test]
    fn strip_drag_moves_whole_strip() {
        let mut st = NlaEditorState::default();
        st.push_track("t", vec![strip("A", 1.0, 4.0), strip("B", 6.0, 9.0)], false);
        st.tracks.row_data(0).unwrap();

        st.strip_pressed(0, 0, false);
        st.strip_dragged(0, 0, 2.0, 0.0);

        let s = st.strip_models[&0].row_data(0).unwrap();
        assert_eq!(s.start, 3.0);
        assert_eq!(s.end, 6.0);
        assert!(s.selected);
        assert!(s.dragging);

        st.strip_released(0, 0);
        assert!(st.drag.is_none());
        assert!(!st.strip_models[&0].row_data(0).unwrap().dragging);
    }

    #[test]
    fn strip_pick_selects_and_records_drag() {
        let mut st = NlaEditorState::default();
        st.push_track("t", vec![strip("A", 1.0, 4.0), strip("B", 6.0, 9.0)], false);

        let hit = st.strip_pick(2.5, 0, 0);
        assert_eq!(hit, 0);
        let s = st.strip_models[&0].row_data(0).unwrap();
        assert!(s.selected);
        assert!(!st.strip_models[&0].row_data(1).unwrap().selected);

        st.strip_dragged(0, 0, 1.5, 0.0);
        let s = st.strip_models[&0].row_data(0).unwrap();
        assert_eq!(s.start, 2.5);
        assert_eq!(s.end, 5.5);
    }

    #[test]
    fn strip_pick_misses_return_neg_one() {
        let mut st = NlaEditorState::default();
        st.push_track("t", vec![strip("A", 1.0, 4.0)], false);

        assert_eq!(st.strip_pick(7.0, 0, 0), -1);
        assert_eq!(st.strip_pick(2.0, 5, 0), -1);
        assert_eq!(st.strip_pick(2.0, -1, 0), -1);
    }

    #[test]
    fn strip_pick_skips_action_track() {
        let mut st = NlaEditorState::default();
        // 全局 Action 轨：NLA 中只能看不能编辑，即使时间命中也返回 -1
        st.push_track("全局 Action", vec![strip("x", 1.0, 4.0)], true);

        assert_eq!(st.strip_pick(2.5, 0, 0), -1);
    }

    #[test]
    fn strip_pressed_selection_is_single() {
        let mut st = NlaEditorState::default();
        st.push_track("t", vec![strip("A", 1.0, 4.0), strip("B", 6.0, 9.0)], false);

        st.strip_pressed(0, 0, false);
        st.strip_pressed(0, 1, false);

        assert!(st.strip_models[&0].row_data(1).unwrap().selected);
        assert!(!st.strip_models[&0].row_data(0).unwrap().selected);
    }

    #[test]
    fn track_toggles_update_rows() {
        let mut st = NlaEditorState::default();
        st.push_track("t", vec![], false);

        st.toggle_track_enabled(0);
        st.toggle_track_locked(0);
        let t = st.tracks.row_data(0).unwrap();
        assert!(!t.enabled);
        assert!(t.locked);
        assert!(!st.tracks_filter.row_data(0).unwrap().enabled);
        assert!(st.tracks_filter.row_data(0).unwrap().locked);
    }

    #[test]
    fn filter_tracks_by_name() {
        let mut st = NlaEditorState::default();
        st.push_track("Idle", vec![], false);
        st.push_track("Run", vec![strip("R", 0.0, 1.0)], false);
        st.push_track("Idle 2", vec![], false);

        st.filter_tracks("Idle");
        assert_eq!(st.tracks_filter.row_count(), 2);
        assert_eq!(st.tracks_filter.row_data(0).unwrap().orig_index, 0);
        assert_eq!(st.tracks_filter.row_data(1).unwrap().orig_index, 2);

        // 命中经过滤视图行号映射回权威轨道
        st.filter_tracks("");
        let hit = st.strip_pick(0.5, 1, 0);
        assert_eq!(hit, 1 * 10000 + 0);
    }

    #[test]
    fn push_down_uses_existing_track_when_space() {
        let mut st = NlaEditorState::default();
        st.push_track("全局 Action", vec![], true); // 轨道0 只读
        st.push_track("Idle", vec![], false); // 轨道1 空，可容纳
        st.push_track("Run", vec![], false);
        st.toggle_track_locked(2); // 轨道2 锁定

        st.push_down("主 Action", 1.0, 5.0);

        assert_eq!(st.strip_models[&0].row_count(), 0, "全局 Action 轨不接收片断");
        assert_eq!(st.strip_models[&2].row_count(), 0, "锁定轨不接收片断");
        assert_eq!(st.strip_models[&1].row_count(), 1, "空轨道可直接容纳");
        let s = st.strip_models[&1].row_data(0).unwrap();
        assert_eq!(s.name, "主 Action");
        assert_eq!(s.start, 1.0);
        assert_eq!(s.end, 5.0);
        assert!(!s.dragging);
    }

    #[test]
    fn push_down_inserts_sorted_into_fitting_track() {
        let mut st = NlaEditorState::default();
        st.push_track("Idle", vec![strip("A", 3.0, 6.0)], false);

        // [7,10] 与 A[3,6] 不重叠 → 插入轨道0（按 start 排序在 A 后）
        st.push_down("Action", 7.0, 10.0);
        assert_eq!(st.tracks.row_count(), 1, "已有轨道能容纳则新建");
        assert_eq!(st.strip_models[&0].row_count(), 2);
        assert_eq!(st.strip_models[&0].row_data(0).unwrap().name, "A");
        assert_eq!(st.strip_models[&0].row_data(1).unwrap().name, "Action");
    }

    #[test]
    fn push_down_creates_new_track_when_no_space() {
        let mut st = NlaEditorState::default();
        st.push_track("Idle", vec![strip("A", 3.0, 6.0)], false);

        // [1,4] 与 A[3,6] 重叠 → 已有轨道放不下 → 新建轨道
        st.push_down("Action", 1.0, 4.0);
        assert_eq!(st.tracks.row_count(), 2);
        assert_eq!(st.strip_models[&0].row_count(), 1, "原轨道不动");
        assert_eq!(st.strip_models[&1].row_count(), 1);
        let s = st.strip_models[&1].row_data(0).unwrap();
        assert_eq!(s.name, "Action");
        assert_eq!(s.start, 1.0);
        assert_eq!(s.end, 4.0);
    }

    #[test]
    fn push_down_creates_track_even_if_all_readonly() {
        let mut st = NlaEditorState::default();
        st.push_track("全局 Action", vec![], true);
        st.push_down("X", 0.0, 1.0);
        assert_eq!(st.tracks.row_count(), 2, "只读轨放不下则新建");
        assert_eq!(st.strip_models[&1].row_data(0).unwrap().name, "X");
    }

    #[test]
    fn global_action_sync_updates_name_and_has_data() {
        let mut st = NlaEditorState::default();
        st.push_track("全局 Action", vec![], true);
        st.push_track("Idle", vec![], false);

        st.set_global_action("主 Action", true);
        let t = st.tracks.row_data(0).unwrap();
        assert_eq!(t.name, "主 Action");
        assert!(t.has_data);
        assert!(!st.tracks.row_data(1).unwrap().has_data, "普通轨道不受影响");
        assert!(st.tracks_filter.row_data(0).unwrap().has_data, "过滤投影同步");

        // 下插后变空：无数据 → 插入按钮隐藏
        st.set_global_action("主 Action", false);
        assert!(!st.tracks.row_data(0).unwrap().has_data);

        // 空 Action（无名称）→ 显示"<无动作>"
        st.set_global_action("", false);
        assert_eq!(st.tracks.row_data(0).unwrap().name, "<无动作>");
    }

    #[test]
    fn locked_track_is_not_editable() {
        let mut st = NlaEditorState::default();
        st.push_track("t", vec![strip("A", 1.0, 4.0)], false);
        st.toggle_track_locked(0);

        // 锁定轨道：点击命中返回 -1（不能选中/拖拽）
        assert_eq!(st.strip_pick(2.5, 0, 0), -1);
        // 直接拖拽也被忽略
        st.strip_pressed(0, 0, false);
        st.strip_dragged(0, 0, 2.0, 0.0);
        let s = st.strip_models[&0].row_data(0).unwrap();
        assert_eq!(s.start, 1.0);
        assert_eq!(s.end, 4.0);
    }

    #[test]
    fn vertical_drag_moves_strip_to_other_track() {
        let mut st = NlaEditorState::default();
        st.push_track("t0", vec![strip("A", 1.0, 4.0)], false);
        st.push_track("t1", vec![], false);

        // 垂直位移 0.6 行：片段中心越过轨道边界 → 拖动中瞬间跳到轨道 1
        st.strip_pressed(0, 0, false);
        st.strip_dragged(0, 0, 0.0, 0.6);
        assert_eq!(st.strip_models[&0].row_count(), 0, "拖动中即应瞬间跨轨");
        assert_eq!(st.strip_models[&1].row_count(), 1);
        let s = st.strip_models[&1].row_data(0).unwrap();
        assert_eq!(s.name, "A");
        assert_eq!(s.start, 1.0);
        assert!(s.dragging);

        st.strip_released(0, 0);
        assert_eq!(st.strip_models[&1].row_count(), 1);
        assert!(!st.strip_models[&1].row_data(0).unwrap().dragging);
    }

    #[test]
    fn vertical_drag_within_half_row_keeps_track() {
        let mut st = NlaEditorState::default();
        st.push_track("t0", vec![strip("A", 1.0, 4.0)], false);
        st.push_track("t1", vec![], false);

        st.strip_pressed(0, 0, false);
        st.strip_dragged(0, 0, 0.0, 0.4); // 中心未越过边界 → 保持轨道 0
        st.strip_released(0, 0);

        assert_eq!(st.strip_models[&0].row_count(), 1);
        assert_eq!(st.strip_models[&1].row_count(), 0);
    }

    #[test]
    fn horizontal_drag_clamped_to_avoid_overlap() {
        let mut st = NlaEditorState::default();
        st.push_track("t", vec![strip("A", 1.0, 4.0), strip("B", 5.0, 9.0)], false);

        // B 向左拖 3 秒：候选 [2,6] 与 A[1,4] 重叠，候选中点(4)在 A 中点(2.5)右侧 → 停 A 右缘 start=4
        st.strip_pressed(0, 1, false);
        st.strip_dragged(0, 1, -3.0, 0.0);
        let s = st.strip_models[&0].row_data(1).unwrap();
        assert_eq!(s.start, 4.0);
        assert_eq!(s.end, 8.0);

        st.strip_released(0, 1);
        assert_eq!(st.strip_models[&0].row_data(1).unwrap().start, 4.0);
    }

    #[test]
    fn horizontal_drag_keeps_order_on_right_side() {
        let mut st = NlaEditorState::default();
        st.push_track("t", vec![strip("A", 4.0, 7.0), strip("B", 0.0, 2.0)], false);

        // B 原在 A 左侧，向右拖：候选 [4,6] 与 A[4,7] 重叠，候选中点(5)在 A 中点(5.5)左侧 → 停 A 左缘 start=2
        st.strip_pressed(0, 1, false);
        st.strip_dragged(0, 1, 4.0, 0.0);
        // reorder 后 B(start=2) 在 A(start=4) 前 → index 0
        let b = st.strip_models[&0].row_data(0).unwrap();
        assert_eq!(b.name, "B");
        assert_eq!(b.start, 2.0);
        assert_eq!(b.end, 4.0);
    }

    #[test]
    fn narrow_gap_is_not_filled() {
        let mut st = NlaEditorState::default();
        st.push_track(
            "t",
            vec![strip("A", 1.0, 3.0), strip("C", 6.0, 8.0), strip("B", 9.0, 14.0)],
            false,
        );

        // B 宽 5，A-C 之间空隙 (3,6) 宽 3 < 5。B 向左拖进入该空隙 → 放不下，不塞入，
        // 保持片段当前 start=9
        st.strip_pressed(0, 2, false);
        st.strip_dragged(0, 2, -5.0, 0.0); // 候选 [4,9] 在 A-C 之间
        let b = (0..st.strip_models[&0].row_count())
            .find_map(|i| st.strip_models[&0].row_data(i).filter(|s| s.name == "B"))
            .unwrap();
        assert_eq!(b.start, 9.0, "窄空隙放不下，片段应保持原位不被塞入");
        assert_eq!(b.end, 14.0);
    }

    #[test]
    fn clamp_uses_candidate_side_not_original() {
        let mut st = NlaEditorState::default();
        st.push_track("t", vec![strip("A", 10.0, 13.0), strip("B", 0.0, 2.0)], false);

        // B 原在 A 左侧，但拖到 A 右侧附近（候选 [11,13]，中点 12 > A 中点 11.5）
        // → 停到 A **右缘** start=13（按候选当前位置判断，而非按下时的左侧）
        st.strip_pressed(0, 1, false);
        st.strip_dragged(0, 1, 11.0, 0.0);
        let s = st.strip_models[&0].row_data(1).unwrap();
        assert_eq!(s.start, 13.0);
        assert_eq!(s.end, 15.0);
    }

    #[test]
    fn vertical_drag_cannot_move_into_action_or_locked_track() {
        let mut st = NlaEditorState::default();
        st.push_track("全局 Action", vec![strip("x", 0.0, 10.0)], true); // 轨道0 只读
        st.push_track("t1", vec![strip("A", 1.0, 4.0)], false); // 轨道1
        st.push_track("t2", vec![], false);
        st.toggle_track_locked(2); // 轨道2 锁定

        // 从轨道1 向上拖 → 目标轨道0（全局 Action，只读）→ 不移动
        st.strip_pressed(1, 0, false);
        st.strip_dragged(1, 0, 0.0, -0.6);
        st.strip_released(1, 0);
        assert_eq!(st.strip_models[&1].row_count(), 1);
        assert_eq!(st.strip_models[&0].row_count(), 1, "只读轨不能接收片断");

        // 从轨道1 向下拖 → 目标轨道2（锁定）→ 不移动
        st.strip_pressed(1, 0, false);
        st.strip_dragged(1, 0, 0.0, 0.6);
        st.strip_released(1, 0);
        assert_eq!(st.strip_models[&1].row_count(), 1);
        assert_eq!(st.strip_models[&2].row_count(), 0, "锁定轨不能接收片断");
    }

    #[test]
    fn cross_track_move_horizontal_clamped_by_target_gaps() {
        let mut st = NlaEditorState::default();
        st.push_track("t0", vec![strip("A", 1.0, 4.0)], false);
        st.push_track("t1", vec![strip("C", 0.0, 3.0)], false);

        // 从轨道0 向下拖到轨道1：拖动中瞬间跨轨，候选 [1,4] 与 C[0,3] 重叠
        //（候选中点 2.5 > C 中点 1.5）→ 停 C 右缘 start=3
        st.strip_pressed(0, 0, false);
        st.strip_dragged(0, 0, 0.0, 0.6);
        assert_eq!(st.strip_models[&0].row_count(), 0, "拖动中已瞬间跨轨");
        let a = (0..st.strip_models[&1].row_count())
            .find_map(|i| st.strip_models[&1].row_data(i).filter(|s| s.name == "A"))
            .expect("A 应已移到轨道1");
        assert_eq!(a.start, 3.0);
        st.strip_released(0, 0);
        assert_eq!(st.strip_models[&0].row_count(), 0);
    }
}
