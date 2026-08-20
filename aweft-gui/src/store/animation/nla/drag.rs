use super::{NlaEditorState, StripDrag};
use crate::depsgraph::{NlaStripMove, NlaStripRef, NlaStripReorder, Tag};
use crate::store::utils::drag::is_valid_delta_pair;

impl NlaEditorState {
    /// 片断拖拽。支持多选同移：选中集一起以相同 `delta_x` 增量移动，各自独立硬钳制
    /// （与曲线编辑器多关键帧同移一致）。偏移由 Slint 侧每次传来累计 `delta_x`，
    /// 增量 = `delta - prev`，无需快照。
    pub fn strip_dragged(
        &mut self,
        track: usize,
        strip: usize,
        delta_x: f32,
        delta_row: f32,
    ) -> Vec<Tag> {
        if !is_valid_delta_pair(delta_x, delta_row) {
            return Vec::new();
        }
        let Some(mut d) = self.drag.clone() else {
            return Vec::new();
        };
        if d.press_track != track || d.press_strip != strip {
            return Vec::new();
        }
        if self
            .track_view
            .get(d.track)
            .map(|t| t.locked)
            .unwrap_or(false)
        {
            return Vec::new();
        }

        // 判定多选：选中数 >1 且按下的是已选中
        let selected_count = self
            .editing_strips
            .iter()
            .flat_map(|l| l.iter())
            .filter(|e| e.view.selected)
            .count();
        let is_multi = selected_count > 1
            && self
                .editing_strips
                .get(d.press_track)
                .and_then(|l| l.get(d.press_strip))
                .map(|e| e.view.selected)
                .unwrap_or(false);

        if is_multi {
            return self.strip_dragged_multi(&mut d, delta_x);
        }

        // —— 单选：保留原有垂直跨轨 + 水平钳制 + 重排 ——
        let width = d.orig_end - d.orig_start;
        let mut updated = Vec::new();
        let mut moved = Vec::new();
        let mut reordered = Vec::new();

        let target = self.drag_target_row(d.press_track, delta_row);
        if target != d.track && self.can_move_to(target) {
            let src_idx = d.strip;
            // 卫语句：越界
            if d.strip >= self.editing_strips[d.track].len() {
                self.drag = Some(d);
                return Vec::new();
            }
            let s = self.editing_strips[d.track].remove(d.strip);
            let dst_list = &mut self.editing_strips[target];
            let start = s.view.start;
            let idx = dst_list
                .iter()
                .position(|x| start < x.view.start)
                .unwrap_or(dst_list.len());
            dst_list.insert(idx, s);
            d.strip = idx;
            moved.push(NlaStripMove {
                src_track: d.track,
                src_index: src_idx,
                dst_track: target,
            });
            d.track = target;
        }

        let fallback = self
            .editing_strips
            .get(d.track)
            .and_then(|l| l.get(d.strip))
            .map(|e| e.view.start)
            .unwrap_or(d.orig_start);
        let raw_start = d.orig_start + delta_x;
        let start = self.clamp_no_overlap(d.track, d.strip, raw_start, width, fallback);
        let Some(e) = self
            .editing_strips
            .get_mut(d.track)
            .and_then(|l| l.get_mut(d.strip))
        else {
            self.drag = Some(d);
            return Vec::new();
        };
        e.view.start = start;
        e.view.end = start + width;
        e.view.dragging = true;

        let old_idx = d.strip;
        d.strip = self.reinsert_sorted(d.track, d.strip);
        if d.strip != old_idx {
            reordered.push(NlaStripReorder {
                track: d.track,
                old_index: old_idx,
                new_index: d.strip,
            });
        }
        updated.push(NlaStripRef {
            track: d.track,
            index: d.strip,
        });
        self.drag = Some(d);
        let mut tags = Vec::new();
        if !moved.is_empty() || !reordered.is_empty() {
            tags.push(Tag::NlaStripStructureChanged {
                moved,
                reordered,
                inserted: Vec::new(),
            });
        }
        if !updated.is_empty() {
            tags.push(Tag::NlaStripPositionChanged { updated });
        }
        tags
    }

    /// 多选同移：选中集以相同增量水平移动，各自独立钳制（与 Graph 多关键帧一致）。
    /// Slint 侧 `delta` 为累计，增量 = `delta - prev`。
    fn strip_dragged_multi(&mut self, drag: &mut StripDrag, delta_x: f32) -> Vec<Tag> {
        let inc_x = delta_x - drag.prev_delta_x;
        if inc_x == 0.0 {
            return Vec::new();
        }
        if !inc_x.is_finite() {
            return Vec::new();
        }
        // 收集每轨选中下标（不可变借用，卫语句提前返回）
        let mut per_track_selected: Vec<std::collections::HashSet<usize>> =
            vec![std::collections::HashSet::new(); self.editing_strips.len()];
        for (ti, list) in self.editing_strips.iter().enumerate() {
            for (idx, e) in list.iter().enumerate() {
                if !e.view.selected {
                    continue;
                }
                per_track_selected[ti].insert(idx);
            }
        }
        let mut updated = Vec::new();
        // 逐轨增量移动（先收集未选中区间，再可变更新，避免借用冲突）
        for ti in 0..self.editing_strips.len() {
            if self
                .track_view
                .get(ti)
                .map(|t| t.is_action || t.locked)
                .unwrap_or(true)
            {
                continue;
            }
            let selected_set = &per_track_selected[ti];
            if selected_set.is_empty() {
                continue;
            }
            // 收集未选中区间（用于独立钳制）
            let unselected: Vec<(f32, f32)> = {
                let list = &self.editing_strips[ti];
                let mut v = Vec::new();
                for (idx, e) in list.iter().enumerate() {
                    if selected_set.contains(&idx) {
                        continue;
                    }
                    v.push((e.view.start, e.view.end));
                }
                v
            };
            // 收集待更新的选中条目快照
            let to_update: Vec<(usize, f32, f32)> = {
                let list = &self.editing_strips[ti];
                let mut v = Vec::new();
                for &idx in selected_set {
                    let Some(e) = list.get(idx) else { continue };
                    v.push((idx, e.view.start, e.view.end - e.view.start));
                }
                v
            };
            // 可变更新
            let list = &mut self.editing_strips[ti];
            for (idx, cur_start, width) in to_update {
                let raw = cur_start + inc_x;
                let clamped = Self::clamp_with_unselected(&unselected, raw, width, cur_start);
                let Some(e) = list.get_mut(idx) else { continue };
                e.view.start = clamped;
                e.view.end = clamped + width;
                e.view.dragging = true;
                updated.push(NlaStripRef { track: ti, index: idx });
            }
        }
        drag.prev_delta_x = delta_x;
        if let Some(d) = self.drag.as_mut() {
            d.prev_delta_x = delta_x;
        }
        if updated.is_empty() {
            return Vec::new();
        }
        vec![Tag::NlaStripPositionChanged { updated }]
    }

    /// 右键取消当前拖动：轻量回退（仅选中条目，不克隆全量）
    pub fn strip_drag_cancel(&mut self) -> Vec<Tag> {
        let Some(d) = self.drag.take() else {
            return Vec::new();
        };
        for list in &mut self.editing_strips {
            for e in list.iter_mut() {
                if e.view.selected {
                    e.view.dragging = false;
                }
            }
        }
        if d.selected_snapshot.len() == 1 {
            let width = d.orig_end - d.orig_start;
            if let Some(list) = self.editing_strips.get_mut(d.track) {
                if let Some(e) = list.get_mut(d.strip) {
                    if e.view.selected {
                        e.view.start = d.orig_start;
                        e.view.end = d.orig_start + width;
                    }
                }
            }
        } else {
            for (track, idx, orig_start, width) in d.selected_snapshot {
                if let Some(list) = self.editing_strips.get_mut(track) {
                    if let Some(e) = list.get_mut(idx) {
                        if e.view.selected {
                            e.view.start = orig_start;
                            e.view.end = orig_start + width;
                        }
                    }
                }
            }
        }
        let mut updated = Vec::new();
        for (ti, list) in self.editing_strips.iter().enumerate() {
            for (idx, e) in list.iter().enumerate() {
                if e.view.selected {
                    updated.push(NlaStripRef { track: ti, index: idx });
                }
            }
        }
        if updated.is_empty() {
            return Vec::new();
        }
        vec![Tag::NlaStripPositionChanged { updated }]
    }

    /// 垂直位移后片段中心所在的轨道（行号）：中心 = 按下轨道中线 + delta_row，
    /// 片段中心越过轨道边界（|delta_row| > 0.5）即进入相邻轨道；越界钳到两端。
    fn drag_target_row(&self, press_row: usize, delta_row: f32) -> usize {
        let center = press_row as f32 + 0.5 + delta_row;
        let target = center.floor().max(0.0) as usize;
        target.min(self.track_view.len().saturating_sub(1))
    }

    /// 单条钳制：把候选 [start, start+width] 钳制到 `track` 轨道的空隙内
    fn clamp_no_overlap(
        &self,
        track: usize,
        ignore_strip: usize,
        start: f32,
        width: f32,
        fallback: f32,
    ) -> f32 {
        let Some(list) = self.editing_strips.get(track) else {
            return start;
        };
        let mut s = start;
        for _ in 0..list.len() + 1 {
            let mut moved = false;
            for (j, o) in list.iter().enumerate() {
                if j == ignore_strip {
                    continue;
                }
                let o = &o.view;
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
            if !moved {
                break;
            }
        }
        for (j, o) in list.iter().enumerate() {
            if j == ignore_strip {
                continue;
            }
            let o = &o.view;
            if s < o.end && s + width > o.start {
                return fallback;
            }
        }
        s
    }

    /// 多选钳制：仅与未选中区间重叠时钳制（与单条钳制同逻辑，但忽略选中集）
    fn clamp_with_unselected(
        unselected: &[(f32, f32)],
        start: f32,
        width: f32,
        fallback: f32,
    ) -> f32 {
        let mut s = start;
        for _ in 0..unselected.len() + 1 {
            let mut moved = false;
            for (o_start, o_end) in unselected {
                if s < *o_end && s + width > *o_start {
                    let cand_mid = s + width / 2.0;
                    let o_mid = (o_start + o_end) / 2.0;
                    if cand_mid < o_mid {
                        s = o_start - width;
                    } else {
                        s = *o_end;
                    }
                    moved = true;
                    break;
                }
            }
            if !moved {
                break;
            }
        }
        for (o_start, o_end) in unselected {
            if s < *o_end && s + width > *o_start {
                return fallback;
            }
        }
        s
    }

    /// 把 `track` 轨道的 `old_idx` 片断取出后按 start 重新插入（数据层），返回新下标。
    fn reinsert_sorted(&mut self, track: usize, old_idx: usize) -> usize {
        let list = &mut self.editing_strips[track];
        let s = list.remove(old_idx);
        let start = s.view.start;
        let idx = list
            .iter()
            .position(|x| start < x.view.start)
            .unwrap_or(list.len());
        list.insert(idx, s);
        idx
    }

    /// 片断松开：跨轨道已在拖动中瞬间完成，这里只退出"移动中"高亮并清空拖拽状态。
    pub fn strip_released(&mut self, track: usize, strip: usize) -> Vec<Tag> {
        let Some(d) = self.drag.clone() else {
            return Vec::new();
        };
        if d.press_track != track || d.press_strip != strip {
            return Vec::new();
        }
        // 清理所有选中条的 dragging
        for list in &mut self.editing_strips {
            for e in list.iter_mut() {
                if e.view.selected {
                    e.view.dragging = false;
                }
            }
        }
        self.drag = None;
        let mut updated = Vec::new();
        for (ti, list) in self.editing_strips.iter().enumerate() {
            for (idx, e) in list.iter().enumerate() {
                if e.view.selected {
                    updated.push(NlaStripRef { track: ti, index: idx });
                }
            }
        }
        if updated.is_empty() {
            updated.push(NlaStripRef {
                track: d.track,
                index: d.strip,
            });
        }
        vec![Tag::NlaStripPositionChanged { updated }]
    }
}
