use super::{DragStripSnapshot, NlaEditorState, NlaStripData, StripDrag, StripId};
use crate::depsgraph::{NlaStripMove, NlaStripRef, NlaStripReorder, Tag};
use crate::store::utils::drag::is_valid_delta_pair;
use std::collections::HashSet;

#[derive(Debug, Clone, Copy)]
struct OffsetInterval {
    lo: f32,
    hi: f32,
}

impl OffsetInterval {
    fn new(lo: f32, hi: f32) -> Option<Self> {
        (lo <= hi).then_some(Self { lo, hi })
    }

    fn contains(self, value: f32) -> bool {
        value >= self.lo && value <= self.hi
    }
}

#[derive(Debug, Clone, Copy)]
struct DragLocation {
    snapshot: DragStripSnapshot,
    source_track: usize,
    source_index: usize,
    target_track: usize,
}

#[derive(Debug)]
struct GroupDragPlan {
    offset: f32,
    row_shift: isize,
    locations: Vec<DragLocation>,
}

#[derive(Debug, Default)]
struct DragChanges {
    moved: Vec<NlaStripMove>,
    reordered: Vec<NlaStripReorder>,
    updated: Vec<NlaStripRef>,
}

impl DragChanges {
    fn add_updated(&mut self, id: StripId, track: usize, index: usize) {
        self.updated.push(NlaStripRef { id, track, index });
    }

    fn into_tags(mut self) -> Vec<Tag> {
        self.updated
            .sort_by_key(|reference| (reference.track, reference.index));

        let mut tags = Vec::new();
        if !self.moved.is_empty() || !self.reordered.is_empty() {
            tags.push(Tag::NlaStripStructureChanged {
                moved: self.moved,
                reordered: self.reordered,
                inserted: Vec::new(),
            });
        }
        if !self.updated.is_empty() {
            tags.push(Tag::NlaStripPositionChanged {
                updated: self.updated,
            });
        }
        tags
    }
}

impl StripDrag {
    fn anchor_snapshot(&self) -> Option<DragStripSnapshot> {
        self.selected_snapshot
            .iter()
            .find(|snapshot| snapshot.id == self.anchor_id)
            .copied()
    }

    fn is_group(&self) -> bool {
        self.selected_snapshot.len() > 1
    }
}

impl NlaEditorState {
    /// 片断拖拽入口：只负责校验会话、确认锚点并分派单选/多选路径。
    /// Slint 传入的是相对按下点的累计 delta。
    pub fn strip_dragged(&mut self, delta_x: f32, delta_row: f32) -> Vec<Tag> {
        if !is_valid_delta_pair(delta_x, delta_row) {
            return Vec::new();
        }
        let Some(mut drag) = self.drag.take() else {
            return Vec::new();
        };
        let Some(anchor) = drag.anchor_snapshot() else {
            self.drag = Some(drag);
            return Vec::new();
        };
        let Some((current_track, _)) = self.find_strip_location(anchor.id) else {
            self.drag = Some(drag);
            return Vec::new();
        };
        if !self.can_move_to(current_track) {
            self.drag = Some(drag);
            return Vec::new();
        }

        let tags = if drag.is_group() {
            self.drag_group(&mut drag, delta_x, delta_row)
        } else {
            self.drag_single(anchor, delta_x, delta_row)
        };
        self.drag = Some(drag);
        tags
    }

    /// 单选拖拽：先规划目标轨和可行 offset，成功后才修改数据层结构。
    fn drag_single(&mut self, anchor: DragStripSnapshot, delta_x: f32, delta_row: f32) -> Vec<Tag> {
        let Some((source_track, source_index)) = self.find_strip_location(anchor.id) else {
            return Vec::new();
        };
        let requested_track = self.drag_target_row(anchor.origin_track, delta_row);
        let target_track = self.single_target_track(source_track, requested_track);
        let Some(offset) = self.single_offset(&anchor, target_track, delta_x) else {
            return Vec::new();
        };

        let mut changes = DragChanges::default();
        let Some((track, index)) = self.move_single_strip(
            anchor.id,
            source_track,
            source_index,
            target_track,
            &mut changes,
        ) else {
            return Vec::new();
        };
        if !self.set_strip_geometry(track, index, anchor.origin_start + offset, anchor.width) {
            return Vec::new();
        }

        let (index, reorder) = self.reinsert_sorted(track, index);
        // 跨轨时 moved 投影会按最终 start 插入目标轨；只有同轨移动才需要
        // 额外的 reorder，否则会对已完成跨轨插入的目标下标再次重排。
        if source_track == track {
            if let Some(reorder) = reorder {
                changes.reordered.push(reorder);
            }
        }
        changes.add_updated(anchor.id, track, index);
        changes.into_tags()
    }

    /// 锁定/只读目标轨不能接收片段，单选时保持当前轨道继续水平拖动。
    fn single_target_track(&self, current_track: usize, requested_track: usize) -> usize {
        self.can_move_to(requested_track)
            .then_some(requested_track)
            .unwrap_or(current_track)
    }

    /// 单选目标轨的独立约束：只允许落在目标轨空隙中，不成功则不发生跨轨。
    fn single_offset(
        &self,
        snapshot: &DragStripSnapshot,
        target_track: usize,
        requested: f32,
    ) -> Option<f32> {
        if !snapshot.origin_start.is_finite()
            || !snapshot.width.is_finite()
            || snapshot.width <= 0.0
            || !self.can_move_to(target_track)
        {
            return None;
        }
        let selected_ids = std::iter::once(snapshot.id).collect();
        let allowed = self.strip_allowed_offsets(snapshot, target_track, &selected_ids);
        Self::nearest_offset(&allowed, requested)
    }

    /// 单片段跨轨：只负责数据层 remove + insert，并记录结构变化。
    fn move_single_strip(
        &mut self,
        id: StripId,
        source_track: usize,
        source_index: usize,
        target_track: usize,
        changes: &mut DragChanges,
    ) -> Option<(usize, usize)> {
        if source_track == target_track || !self.can_move_to(target_track) {
            return Some((source_track, source_index));
        }
        let entry = self.remove_strip(source_track, source_index)?;
        let target_index = self.insert_strip(target_track, entry);
        changes.moved.push(NlaStripMove {
            id,
            src_track: source_track,
            src_index: source_index,
            dst_track: target_track,
        });
        Some((target_track, target_index))
    }

    /// 多选整体拖拽：规划统一 offset/row_shift，再一次性搬运全部选中片段。
    fn drag_group(&mut self, drag: &mut StripDrag, delta_x: f32, delta_row: f32) -> Vec<Tag> {
        let Some(plan) = self.plan_group_drag(drag, delta_x, delta_row) else {
            return Vec::new();
        };
        let original_orders = self.capture_track_orders(&plan.locations);
        let Some(moving) = self.remove_group(&plan.locations) else {
            return Vec::new();
        };
        self.insert_group(moving, plan.offset);

        let mut changes = DragChanges::default();
        if self.orders_changed(&original_orders) {
            changes.moved = plan
                .locations
                .iter()
                .map(|location| NlaStripMove {
                    id: location.snapshot.id,
                    src_track: location.source_track,
                    src_index: location.source_index,
                    dst_track: location.target_track,
                })
                .collect();
        }
        self.add_updated_snapshots(&mut changes, &plan.locations);
        drag.last_offset = plan.offset;
        drag.last_row_shift = plan.row_shift;
        changes.into_tags()
    }

    /// 多选规划：选择可行的 row_shift，求所有片段约束的交集，再选最终 offset。
    fn plan_group_drag(
        &self,
        drag: &StripDrag,
        delta_x: f32,
        delta_row: f32,
    ) -> Option<GroupDragPlan> {
        let anchor = drag.anchor_snapshot()?;
        let requested_shift = self.requested_row_shift(anchor.origin_track, delta_row);
        let (row_shift, allowed) = self.resolve_group_row_shift(drag, requested_shift)?;
        let offset = Self::choose_offset(&allowed, delta_x, drag.last_offset)?;
        let locations = self.group_locations(&drag.selected_snapshot, row_shift)?;
        Some(GroupDragPlan {
            offset,
            row_shift,
            locations,
        })
    }

    /// 当请求的目标行不可行时，保留上一次成功的目标行，避免拖拽状态被破坏。
    fn resolve_group_row_shift(
        &self,
        drag: &StripDrag,
        requested_shift: isize,
    ) -> Option<(isize, Vec<OffsetInterval>)> {
        if let Some(allowed) = self.group_allowed_offsets(&drag.selected_snapshot, requested_shift)
        {
            return Some((requested_shift, allowed));
        }
        if requested_shift != drag.last_row_shift {
            if let Some(allowed) =
                self.group_allowed_offsets(&drag.selected_snapshot, drag.last_row_shift)
            {
                return Some((drag.last_row_shift, allowed));
            }
        }
        None
    }

    /// 计算快照片段当前拖拽轮廓的源/目标定位。
    fn group_locations(
        &self,
        snapshots: &[DragStripSnapshot],
        row_shift: isize,
    ) -> Option<Vec<DragLocation>> {
        snapshots
            .iter()
            .map(|snapshot| {
                let (source_track, source_index) = self.find_strip_location(snapshot.id)?;
                let target_track = Self::shifted_track(snapshot.origin_track, row_shift)?;
                Some(DragLocation {
                    snapshot: *snapshot,
                    source_track,
                    source_index,
                    target_track,
                })
            })
            .collect()
    }

    /// 记录所有受影响轨道的片段 ID 顺序，用于判断是否需要结构投影。
    fn capture_track_orders(&self, locations: &[DragLocation]) -> Vec<(usize, Vec<StripId>)> {
        let mut tracks: Vec<usize> = locations
            .iter()
            .flat_map(|location| [location.source_track, location.target_track])
            .collect();
        tracks.sort_unstable();
        tracks.dedup();
        tracks
            .into_iter()
            .map(|track| {
                let order = self
                    .editing_strips
                    .get(track)
                    .into_iter()
                    .flatten()
                    .map(|entry| entry.id)
                    .collect();
                (track, order)
            })
            .collect()
    }

    fn orders_changed(&self, original_orders: &[(usize, Vec<StripId>)]) -> bool {
        original_orders.iter().any(|(track, before)| {
            let after: Vec<StripId> = self
                .editing_strips
                .get(*track)
                .into_iter()
                .flatten()
                .map(|entry| entry.id)
                .collect();
            before != &after
        })
    }

    /// 批量按源下标降序移除，避免同一轨道上的下标互相影响。
    fn remove_group(
        &mut self,
        locations: &[DragLocation],
    ) -> Option<Vec<(DragLocation, NlaStripData)>> {
        let mut remove_order: Vec<usize> = (0..locations.len()).collect();
        remove_order.sort_by_key(|&i| {
            (
                locations[i].source_track,
                std::cmp::Reverse(locations[i].source_index),
            )
        });

        let mut moving = Vec::with_capacity(locations.len());
        for i in remove_order {
            let location = locations[i];
            let entry = self.remove_strip(location.source_track, location.source_index)?;
            moving.push((location, entry));
        }
        moving.sort_by_key(|(location, _)| {
            (
                location.snapshot.origin_track,
                location.snapshot.origin_index,
            )
        });
        Some(moving)
    }

    /// 批量写入统一时间偏移后按目标轨道时间排序插入。
    fn insert_group(&mut self, moving: Vec<(DragLocation, NlaStripData)>, offset: f32) {
        for (location, mut entry) in moving {
            entry.view.start = location.snapshot.origin_start + offset;
            entry.view.end = entry.view.start + location.snapshot.width;
            entry.view.dragging = true;
            self.insert_strip(location.target_track, entry);
        }
    }

    fn add_updated_snapshots(&self, changes: &mut DragChanges, locations: &[DragLocation]) {
        for location in locations {
            if let Some((track, index)) = self.find_strip_location(location.snapshot.id) {
                changes.add_updated(location.snapshot.id, track, index);
            }
        }
    }

    fn requested_row_shift(&self, origin_track: usize, delta_row: f32) -> isize {
        let target = self.drag_target_row(origin_track, delta_row);
        target as isize - origin_track as isize
    }

    fn shifted_track(origin_track: usize, row_shift: isize) -> Option<usize> {
        let target = origin_track as isize + row_shift;
        (target >= 0).then_some(target as usize)
    }

    /// 求多选片段的共同可行 offset 区间：每个片段独立约束，再求交集。
    fn group_allowed_offsets(
        &self,
        snapshots: &[DragStripSnapshot],
        row_shift: isize,
    ) -> Option<Vec<OffsetInterval>> {
        self.validate_group_snapshots(snapshots, row_shift)?;
        let selected_ids: HashSet<StripId> = snapshots.iter().map(|snapshot| snapshot.id).collect();
        let mut allowed = Self::unbounded_offsets();
        for snapshot in snapshots {
            let target_track = Self::shifted_track(snapshot.origin_track, row_shift)?;
            let strip_allowed = self.strip_allowed_offsets(snapshot, target_track, &selected_ids);
            allowed = Self::intersect_intervals(&allowed, &strip_allowed);
            if allowed.is_empty() {
                return None;
            }
        }
        (!self.selected_strips_overlap(snapshots, row_shift)).then_some(allowed)
    }

    fn validate_group_snapshots(
        &self,
        snapshots: &[DragStripSnapshot],
        row_shift: isize,
    ) -> Option<()> {
        if snapshots.is_empty()
            || snapshots.iter().any(|snapshot| {
                !snapshot.origin_start.is_finite()
                    || !snapshot.width.is_finite()
                    || snapshot.width <= 0.0
            })
        {
            return None;
        }
        for snapshot in snapshots {
            let target_track = Self::shifted_track(snapshot.origin_track, row_shift)?;
            if !self.can_move_to(target_track) {
                return None;
            }
        }
        Some(())
    }

    fn unbounded_offsets() -> Vec<OffsetInterval> {
        vec![OffsetInterval {
            lo: f32::NEG_INFINITY,
            hi: f32::INFINITY,
        }]
    }

    fn selected_strips_overlap(&self, snapshots: &[DragStripSnapshot], row_shift: isize) -> bool {
        for (left_index, left) in snapshots.iter().enumerate() {
            let Some(left_track) = Self::shifted_track(left.origin_track, row_shift) else {
                return true;
            };
            for right in snapshots.iter().skip(left_index + 1) {
                let Some(right_track) = Self::shifted_track(right.origin_track, row_shift) else {
                    return true;
                };
                if left_track == right_track
                    && Self::ranges_overlap(
                        left.origin_start,
                        left.origin_start + left.width,
                        right.origin_start,
                        right.origin_start + right.width,
                    )
                {
                    return true;
                }
            }
        }
        false
    }

    // 单条轨道的所有空位
    fn strip_allowed_offsets(
        &self,
        snapshot: &DragStripSnapshot,
        target_track: usize,
        selected_ids: &HashSet<StripId>,
    ) -> Vec<OffsetInterval> {
        let Some(list) = self.editing_strips.get(target_track) else {
            return Vec::new();
        };
        let mut allowed = Self::unbounded_offsets();
        for obstacle in list {
            if selected_ids.contains(&obstacle.id)
                || !obstacle.view.start.is_finite()
                || !obstacle.view.end.is_finite()
                || obstacle.view.end <= obstacle.view.start
            {
                continue;
            }
            // 是相较于 origin_start 的 offset 区间
            let forbidden_lo = obstacle.view.start - snapshot.width - snapshot.origin_start;
            let forbidden_hi = obstacle.view.end - snapshot.origin_start;
            allowed = allowed
                .into_iter()
                .flat_map(|interval| {
                    Self::subtract_open_interval(interval, forbidden_lo, forbidden_hi)
                })
                .collect();
            if allowed.is_empty() {
                break;
            }
        }
        allowed
    }

    // 从已有允许区间中挖去一个禁止带
    fn subtract_open_interval(
        interval: OffsetInterval,
        forbidden_lo: f32,
        forbidden_hi: f32,
    ) -> Vec<OffsetInterval> {
        if forbidden_lo >= forbidden_hi {
            return vec![interval];
        }
        let mut result = Vec::new();
        if interval.lo <= forbidden_lo {
            if let Some(left) = OffsetInterval::new(interval.lo, interval.hi.min(forbidden_lo)) {
                result.push(left);
            }
        }
        if interval.hi >= forbidden_hi {
            if let Some(right) = OffsetInterval::new(interval.lo.max(forbidden_hi), interval.hi) {
                result.push(right);
            }
        }
        result
    }

    // 双区间取交集
    fn intersect_intervals(
        accumulated: &[OffsetInterval],
        current: &[OffsetInterval],
    ) -> Vec<OffsetInterval> {
        accumulated
            .iter()
            .flat_map(|a| {
                current
                    .iter()
                    .filter_map(move |b| OffsetInterval::new(a.lo.max(b.lo), a.hi.min(b.hi)))
            })
            .collect()
    }

    fn choose_offset(intervals: &[OffsetInterval], requested: f32, current: f32) -> Option<f32> {
        if intervals
            .iter()
            .any(|interval| interval.contains(requested))
        {
            return Some(requested);
        }
        // 优先留在当前连续可行区间，避免一次大 delta 跳过中间障碍物。
        // 即到达可以容纳的位置之后才会换边
        if let Some(interval) = intervals.iter().find(|interval| interval.contains(current)) {
            return Some(if requested < interval.lo {
                interval.lo
            } else {
                interval.hi
            });
        }
        Self::nearest_offset(intervals, requested)
    }

    fn nearest_offset(intervals: &[OffsetInterval], requested: f32) -> Option<f32> {
        intervals
            .iter()
            .map(|interval| {
                let value = requested.clamp(interval.lo, interval.hi);
                (value - requested).abs()
            })
            .zip(intervals.iter())
            .min_by(|(left, _), (right, _)| {
                left.partial_cmp(right).unwrap_or(std::cmp::Ordering::Equal)
            })
            .map(|(_, interval)| requested.clamp(interval.lo, interval.hi))
    }

    fn ranges_overlap(left_start: f32, left_end: f32, right_start: f32, right_end: f32) -> bool {
        left_start < right_end && left_end > right_start
    }

    /// 取消拖拽：恢复快照中的原轨道、原顺序、原时间和 dragging 状态。
    pub fn strip_drag_cancel(&mut self) -> Vec<Tag> {
        let Some(drag) = self.drag.take() else {
            return Vec::new();
        };
        let Some(locations) = self.restore_locations(&drag.selected_snapshot) else {
            return Vec::new();
        };
        let original_orders = self.capture_track_orders(&locations);
        let Some(moving) = self.remove_group(&locations) else {
            return Vec::new();
        };
        self.restore_group(moving);

        let mut changes = DragChanges::default();
        if self.orders_changed(&original_orders) {
            changes.moved = locations
                .iter()
                .map(|location| NlaStripMove {
                    id: location.snapshot.id,
                    src_track: location.source_track,
                    src_index: location.source_index,
                    dst_track: location.target_track,
                })
                .collect();
        }
        self.add_updated_snapshots(&mut changes, &locations);
        changes.into_tags()
    }

    fn restore_locations(&self, snapshots: &[DragStripSnapshot]) -> Option<Vec<DragLocation>> {
        snapshots
            .iter()
            .map(|snapshot| {
                let (source_track, source_index) = self.find_strip_location(snapshot.id)?;
                Some(DragLocation {
                    snapshot: *snapshot,
                    source_track,
                    source_index,
                    target_track: snapshot.origin_track,
                })
            })
            .collect()
    }

    /// 按快照原始下标插回，避免取消拖拽时只按 start 排序丢失原顺序。
    fn restore_group(&mut self, mut moving: Vec<(DragLocation, NlaStripData)>) {
        moving.sort_by_key(|(location, _)| {
            (
                location.snapshot.origin_track,
                location.snapshot.origin_index,
            )
        });
        for (location, mut entry) in moving {
            entry.view.start = location.snapshot.origin_start;
            entry.view.end = location.snapshot.origin_start + location.snapshot.width;
            entry.view.dragging = false;
            self.insert_strip_at(
                location.snapshot.origin_track,
                location.snapshot.origin_index,
                entry,
            );
        }
    }

    /// 片断松开：只清除拖拽态；结构和位置已在拖动过程中完成。
    pub fn strip_released(&mut self) -> Vec<Tag> {
        let Some(drag) = self.drag.take() else {
            return Vec::new();
        };
        let updated = self.clear_dragging(&drag.selected_snapshot);
        if updated.is_empty() {
            return Vec::new();
        }
        DragChanges {
            updated,
            ..DragChanges::default()
        }
        .into_tags()
    }

    fn clear_dragging(&mut self, snapshots: &[DragStripSnapshot]) -> Vec<NlaStripRef> {
        snapshots
            .iter()
            .filter_map(|snapshot| {
                let (track, index) = self.find_strip_location(snapshot.id)?;
                self.editing_strips
                    .get_mut(track)
                    .and_then(|list| list.get_mut(index))
                    .map(|entry| {
                        entry.view.dragging = false;
                        NlaStripRef {
                            id: snapshot.id,
                            track,
                            index,
                        }
                    })
            })
            .collect()
    }

    fn remove_strip(&mut self, track: usize, index: usize) -> Option<NlaStripData> {
        self.editing_strips
            .get_mut(track)
            .and_then(|list| (index < list.len()).then(|| list.remove(index)))
    }

    fn insert_strip_at(&mut self, track: usize, index: usize, entry: NlaStripData) {
        if let Some(list) = self.editing_strips.get_mut(track) {
            list.insert(index.min(list.len()), entry);
        }
    }

    fn set_strip_geometry(&mut self, track: usize, index: usize, start: f32, width: f32) -> bool {
        let Some(entry) = self
            .editing_strips
            .get_mut(track)
            .and_then(|list| list.get_mut(index))
        else {
            return false;
        };
        entry.view.start = start;
        entry.view.end = start + width;
        entry.view.dragging = true;
        true
    }

    /// 垂直位移后片段中心所在的轨道；越界钳到两端。
    fn drag_target_row(&self, press_row: usize, delta_row: f32) -> usize {
        let center = press_row as f32 + 0.5 + delta_row;
        let target = center.floor().max(0.0) as usize;
        target.min(self.track_view.len().saturating_sub(1))
    }

    /// 把轨道内片段取出后按 start 重新插入，返回新下标和可选重排信息。
    fn reinsert_sorted(
        &mut self,
        track: usize,
        old_index: usize,
    ) -> (usize, Option<NlaStripReorder>) {
        let list = &mut self.editing_strips[track];
        let entry = list.remove(old_index);
        let start = entry.view.start;
        let new_index = list
            .iter()
            .position(|other| start < other.view.start)
            .unwrap_or(list.len());
        list.insert(new_index, entry);
        let reorder = (old_index != new_index).then_some(NlaStripReorder {
            track,
            old_index,
            new_index,
        });
        (new_index, reorder)
    }
}
