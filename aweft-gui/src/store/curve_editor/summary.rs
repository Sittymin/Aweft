//! 摄影表汇总条带：按整数毫秒时间聚合所有可见通道同一时刻的关键帧。
//!
//! 派生更新（rebuild_summary / summary_update_or_insert / summary_remove_if_empty）
//! 由投影在关键帧数据变化后调用；交互（summary_keyframe_pressed / dragged）
//! 由 reducer 调用（只改关键帧数据，派生更新交给投影）。

use std::collections::BTreeMap;
use std::rc::Rc;

use slint::{Model, ModelRc};

use super::{ms_of, CurveEditorState};
use crate::bridge::types::{
    SlintSelectionModifier, SlintSummaryKeyframe,
};
use crate::depsgraph::Tag;

impl CurveEditorState {
    /// 重建摄影表汇总条带：把全部通道的关键帧按整数毫秒时间聚合，
    /// 同一毫秒的关键帧合并为一个汇总标记（count = 该时刻关键帧总数，
    /// selected = 逐通道选中掩码，按通道在过滤视图中的顺序）。
    /// 只统计摄影表当前可见的通道（搜索过滤 + 眼睛），与下方条带保持一致。
    /// 时间已按整数毫秒对齐，因此"同一时间"即 ms 相等。
    pub(super) fn rebuild_summary(&mut self) {
        let mut groups: BTreeMap<i64, Vec<bool>> = BTreeMap::new();
        for row in 0..self.dope_filter.row_count() {
            let Some(ch) = self.dope_filter.row_data(row) else { continue };
            let curve = ch.curve_index as usize;
            let visible = self.channels_data.get(curve).map(|c| c.visible).unwrap_or(false);
            if !visible {
                continue;
            }
            let Some(kfs) = self.editing_keyframes.get(&curve) else { continue };
            for kf in kfs {
                groups.entry(ms_of(kf.time)).or_default().push(kf.selected);
            }
        }
        let items: Vec<SlintSummaryKeyframe> = groups
            .into_iter()
            .map(|(ms, sel)| SlintSummaryKeyframe {
                time: ms as f32 / 1000.0,
                count: sel.len() as i32,
                all_selected: sel.iter().all(|s| *s),
                selected: ModelRc::from(Rc::new(slint::VecModel::from(sel))),
            })
            .collect();
        self.summary_keyframes.set_vec(items);
    }

    /// 按 ms 查找汇总条带中的条目索引。
    fn summary_find_by_ms(&self, ms: i64) -> Option<usize> {
        for i in 0..self.summary_keyframes.row_count() {
            if let Some(sk) = self.summary_keyframes.row_data(i) {
                if ms_of(sk.time) == ms {
                    return Some(i);
                }
            }
        }
        None
    }

    /// 从数据重建单个 ms 条目（扫描可见通道的关键帧）。
    fn summary_build_entry_for_ms(&self, ms: i64) -> SlintSummaryKeyframe {
        let mut sel = Vec::new();
        for row in 0..self.dope_filter.row_count() {
            let Some(ch) = self.dope_filter.row_data(row) else { continue };
            let curve = ch.curve_index as usize;
            let visible = self.channels_data.get(curve).map(|c| c.visible).unwrap_or(false);
            if !visible {
                continue;
            }
            if let Some(kfs) = self.editing_keyframes.get(&curve) {
                for kf in kfs {
                    if ms_of(kf.time) == ms {
                        sel.push(kf.selected);
                    }
                }
            }
        }
        SlintSummaryKeyframe {
            time: ms as f32 / 1000.0,
            count: sel.len() as i32,
            all_selected: !sel.is_empty() && sel.iter().all(|s| *s),
            selected: ModelRc::from(Rc::new(slint::VecModel::from(sel))),
        }
    }

    /// 更新或插入汇总条带中的一个 ms 条目（增量）。
    /// 若该 ms 条目已存在：原地重建。若不存在：在排序位置插入。
    pub(super) fn summary_update_or_insert(&mut self, ms: i64) {
        let entry = self.summary_build_entry_for_ms(ms);
        if let Some(idx) = self.summary_find_by_ms(ms) {
            self.summary_keyframes.set_row_data(idx, entry);
        } else if entry.count > 0 {
            // 找到插入位置（保持 time 排序）
            let insert_at = (0..self.summary_keyframes.row_count())
                .find(|&i| {
                    self.summary_keyframes
                        .row_data(i)
                        .map(|sk| ms_of(sk.time) > ms)
                        .unwrap_or(false)
                })
                .unwrap_or(self.summary_keyframes.row_count());
            self.summary_keyframes.insert(insert_at, entry);
        }
    }

    /// 移除汇总条带中的一个 ms 条目（若存在）。
    pub(super) fn summary_remove_if_empty(&mut self, ms: i64) {
        if let Some(idx) = self.summary_find_by_ms(ms) {
            if self.summary_keyframes.row_data(idx).map(|sk| sk.count).unwrap_or(0) == 0 {
                self.summary_keyframes.remove(idx);
            }
        }
    }

    // ── 汇总条带交互 ───────────────────────────────────────────────────

    /// 汇总关键帧按下：none=单选，shift=扩展，ctrl=切换该时刻的全部关键帧。
    /// 与普通关键帧不同：汇总帧代表"该时刻的全部"，因此普通点击**总是**清空并
    /// 单选该时刻全部（不会因为其中某个已选中而"保持现状"，否则拖拽时只剩已选中的
    /// 那几个会移动，与用户预期不符）；Shift/Ctrl 才追加到现有选择。
    /// 通道选中用 pending（grab 结束后写回）。
    pub fn summary_keyframe_pressed(&mut self, index: usize, modifier: SlintSelectionModifier) -> Vec<Tag> {
        let Some(sk) = self.summary_keyframes.row_data(index) else { return Vec::new() };
        let target_ms = ms_of(sk.time);
        // 收集该时刻跨通道的全部关键帧（可见且可编辑的通道；与拖拽一致）
        let mut group: Vec<(usize, usize)> = Vec::new();
        for (&c, kfs) in self.editing_keyframes.iter() {
            if !self.channel_editable(c) {
                continue;
            }
            let visible = self.channels_data.get(c).map(|ch| ch.visible).unwrap_or(true);
            if !visible {
                continue;
            }
            for (j, kf) in kfs.iter().enumerate() {
                if ms_of(kf.time) == target_ms {
                    group.push((c, j));
                }
            }
        }
        if group.is_empty() {
            return Vec::new();
        }
        let already_selected = group
            .iter()
            .any(|&(c, j)| {
                self.editing_keyframes
                    .get(&c)
                    .and_then(|kfs| kfs.get(j))
                    .map(|k| k.selected)
                    .unwrap_or(false)
            });
        self.pressed_was_selected = already_selected;
        let mut summary_ms: std::collections::HashSet<i64> = std::collections::HashSet::new();
        if modifier != SlintSelectionModifier::None {
            // 加入：全部选上（不取消），并确保其通道在期望选中集合里
            for &(c, j) in &group {
                let should_toggle_off = modifier == SlintSelectionModifier::Ctrl
                    && self.editing_keyframes.get(&c).and_then(|kfs| kfs.get(j)).map(|keyframe| keyframe.selected).unwrap_or(false);
                if let Some(ms) = self.set_keyframe_selected(c, j, should_toggle_off) {
                    summary_ms.insert(ms);
                }
            }
            let mut sel = self.effective_selected_channels();
            for &(c, _) in &group {
                if !sel.contains(&c) {
                    sel.push(c);
                }
            }
            // 立即写回通道选中（追加，不清空其它）；汇总标记独立于 channels 嵌套循环，
            // 按下期间更新安全——拖动一开始就亮起所有关联轨道，无需等松开二次选中。
            for &c in &sel {
                self.set_channel_selected(c, true);
            }
            self.pending_channel_selection.clear();
        } else {
            // 普通点击：无论该时刻是否已有选中，都清空并单选该时刻全部
            let mut chans: Vec<usize> = group.iter().map(|&(c, _)| c).collect();
            chans.sort_unstable();
            chans.dedup();
            self.selection_anchor = chans[0] as isize;
            for ms in self.deselect_all() {
                summary_ms.insert(ms);
            }
            for &(c, j) in &group {
                if let Some(ms) = self.set_keyframe_selected(c, j, false) {
                    summary_ms.insert(ms);
                }
            }
            // 立即写回通道选中（替换选择）
            self.clear_channel_selection();
            for &c in &chans {
                self.set_channel_selected(c, true);
            }
            self.pending_channel_selection.clear();
        }
        self.current_channel = group[0].0 as isize;
        // 按下立即高亮（视觉反馈，不用等松开重建）：汇总标记独立于 channels 嵌套循环，
        // 按下期间更新本行模型安全（不会破坏 Slint 拖拽抓取）。这是拖拽上下文预览
        // （红框语义），保持 reducer 直写；条带其余更新走投影。
        if let Some(mut sk) = self.summary_keyframes.row_data(index) {
            let n = sk.selected.row_count();
            sk.selected = ModelRc::from(Rc::new(slint::VecModel::from(vec![true; n])));
            sk.all_selected = true;
            self.summary_keyframes.set_row_data(index, sk);
        }
        vec![Tag::AnimationSelectionChanged {
            summary_ms: summary_ms.into_iter().collect(),
        }]
    }

    /// 汇总关键帧拖拽：把增量累加到全部选中关键帧的时间上（跨通道同时移动，
    /// 同一时刻的关键帧保持同步），时间对齐到整数毫秒。
    /// `index` 为被拖的汇总标记下标：就地更新其 time（只改数据不改结构，
    /// 保住 Slint 拖拽抓取）；不整体重建汇总模型（set_vec 会重建元素、打断 grab）。
    pub fn summary_keyframe_dragged(&mut self, index: usize, delta_time: f32) -> Vec<Tag> {
        if !delta_time.is_finite() {
            return Vec::new();
        }
        let mut moved: Vec<usize> = Vec::new();
        let mut updates: Vec<(usize, usize, f32)> = Vec::new(); // (通道, 下标, 新时间)
        for (&c, kfs) in self.editing_keyframes.iter() {
            if !self.channel_editable(c) {
                continue;
            }
            for (j, kf) in kfs.iter().enumerate() {
                if kf.selected {
                    updates.push((c, j, super::snap_ms(kf.time + delta_time)));
                }
            }
        }
        for (c, j, time) in updates {
            if let Some(kfs) = self.editing_keyframes.get_mut(&c)
                && let Some(kf) = kfs.get_mut(j)
            {
                kf.time = time;
                if !moved.contains(&c) {
                    moved.push(c);
                }
            }
        }
        if moved.is_empty() {
            return Vec::new();
        }
        // 曲线段/手柄/连接线随关键帧移动由投影重建；被拖汇总标记就地更新时间（保住 grab）
        vec![Tag::AnimationKeyframesChanged {
            curves: moved,
            summary_ms: vec![],
            dragged_curve: None,
            dragged_summary: Some((index, delta_time)),
        }]
    }
}
