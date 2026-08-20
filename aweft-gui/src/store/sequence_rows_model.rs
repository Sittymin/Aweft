//! Sequence 数据源模型：实现 `slint::Model`，从权威 project 数据实时投影行。
//!
//! 替代旧的 `VecModel<SlintSubtitleRow>` 行副本缓存。关键区别：
//! - **结构索引**（`anchors`）：扁平化顺序的 item 引用 + 层级上下文，结构变化时重建；
//! - **`row_data(i)` 惰性构造**：从 `anchors[i]` + 权威数据构造 `SlintSubtitleRow`；
//! - **`ModelNotify` 驱动**：值变化 → `notify.row_changed(i)`，结构变化 → `notify.row_added/removed`
//!   或 `reset()`，由 Slint 自动精确重算受影响行，零手工 `set_row_data`。

use std::cell::RefCell;
use std::rc::Rc;

use aweft_model::{
    ids::ItemId,
    sequence::{ItemContent, Sequence},
};
use chrono::TimeDelta;
use slint::{Model, ModelNotify, ModelRc, ModelTracker, SharedString, ToSharedString};

use crate::bridge::{
    SlintSelectionState, SlintSubtitleRow, SlintSubtitleRowType, SlintTimelineSubBlock,
    format_time,
};

/// 一个扁平化行的索引锚点：结构变化时重建，`row_data(i)` 用它 O(1) 定位。
#[derive(Debug, Clone)]
struct RowAnchor {
    item_id: ItemId,
    depth: i32,
    ancestors: Rc<Vec<SharedString>>,
    /// 祖先分组的 ItemId 链（叶子到根，含根）。供过滤判断用（避免 UUID parse）。
    ancestor_ids: Rc<Vec<ItemId>>,
    /// 权威数据的静态行内容（投影时构建，含 sequence 的 event/group 数据）。
    /// 动态 UI 状态（selected/collapsed/preview）由 map 叠加，不入快照。
    content: SlintSubtitleRow,
}

/// 轻量行数据：数据源模型产出的纯数据（不含 Slint UI 结构）。
/// UI 映射（`SlintSubtitleRow` 构造 + 分组聚合）由 `SequenceRowMapper` 负责。
#[derive(Debug, Clone)]
pub struct RowData {
    pub item_id: ItemId,
    pub depth: i32,
    pub ancestors: Rc<Vec<SharedString>>,
    /// 祖先分组的 ItemId 链（叶子到根）。供过滤判断用（避免 UUID parse）。
    pub ancestor_ids: Rc<Vec<ItemId>>,
    /// 静态行内容快照（row_data 时从锚点 clone，map 叠加动态状态）。
    pub content: SlintSubtitleRow,
}

impl RowData {
    /// 从锚点构造轻量行数据（无 UI 结构，零聚合开销）。
    fn from_anchor(anchor: &RowAnchor) -> Self {
        Self {
            item_id: anchor.item_id,
            depth: anchor.depth,
            ancestors: anchor.ancestors.clone(),
            ancestor_ids: anchor.ancestor_ids.clone(),
            content: anchor.content.clone(),
        }
    }
}

/// 扁平化遍历结果：锚点列表 + 分组聚合元数据。
#[derive(Debug, Default)]
struct Flattened {
    anchors: Vec<RowAnchor>,
    /// group item_id → 子树行数（含分组头自身）。供 insert_after 定位组末尾。
    group_row_counts: std::collections::HashMap<ItemId, usize>,
}

/// 数据源模型。
///
/// 权威数据在 model（`Project.sequence`，纯值语义）；本模型持扁平化索引锚点
/// （`flattened`），锚点含投影构建的静态行内容快照（`RowAnchor.content`）。
/// 投影（op_sequence_*）在 tag 后重建受影响行 content + notify，Slint 精确重算。
/// 折叠/选中/预览是运行时 UI 状态（`Rc<RefCell>`），由 map 叠加——不入权威数据。
#[derive(Clone)]
pub struct SequenceRowsModel {
    /// 扁平化索引 + 聚合缓存（含静态行内容快照）。
    flattened: Rc<RefCell<Flattened>>,
    /// 折叠集合共享引用（供 FilterModel 与 toggle 使用）。
    collapsed: Rc<RefCell<std::collections::HashSet<ItemId>>>,
    /// 选中状态集合（三态：Unselected/DirectlySelected/IndirectlySelected）。
    selected: Rc<RefCell<std::collections::HashMap<ItemId, SlintSelectionState>>>,
    /// 时间码拖拽预览覆盖（event_id → (start 覆盖, end 覆盖)）。
    /// 不修改 sequence，仅 UI 显示时覆盖。commit/cancel 时清除。
    preview_overrides:
        Rc<RefCell<std::collections::HashMap<ItemId, (Option<TimeDelta>, Option<TimeDelta>)>>>,
    notify: Rc<ModelNotify>,
}

impl SequenceRowsModel {
    pub fn new() -> Self {
        Self {
            flattened: Rc::new(RefCell::new(Flattened::default())),
            collapsed: Rc::new(RefCell::new(std::collections::HashSet::new())),
            selected: Rc::new(RefCell::new(std::collections::HashMap::new())),
            preview_overrides: Rc::new(RefCell::new(std::collections::HashMap::new())),
            notify: Rc::new(ModelNotify::default()),
        }
    }

    pub fn collapsed_groups(&self) -> Rc<RefCell<std::collections::HashSet<ItemId>>> {
        self.collapsed.clone()
    }

    /// 创建行映射器（共享引用，供 MapModel 映射 SlintSubtitleRow）。
    /// 只持动态 UI 状态（折叠/选中/预览），不持权威数据（content 已在快照）。
    pub fn mapper(&self) -> SequenceRowMapper {
        SequenceRowMapper {
            collapsed: self.collapsed.clone(),
            selected: self.selected.clone(),
            preview_overrides: self.preview_overrides.clone(),
        }
    }

    /// 是否选中某 item。
    /// 是否处于任一选中态（Directly/Indirectly）。
    pub fn is_selected(&self, item_id: ItemId) -> bool {
        self.selected
            .borrow()
            .get(&item_id)
            .map(|s| *s != SlintSelectionState::Unselected)
            .unwrap_or(false)
    }

    /// 设置/清除某 item 的选中态，并通知 Slint 重算该行。
    /// 三态：Unselected（清除）/ DirectlySelected（直接选中）/
    /// IndirectlySelected（范围选中导致的间接选中）。
    pub fn set_selected(&self, item_id: ItemId, state: SlintSelectionState) {
        let mut set = self.selected.borrow_mut();
        if state == SlintSelectionState::Unselected {
            set.remove(&item_id);
        } else {
            set.insert(item_id, state);
        }
        drop(set);
        if let Some(idx) = self.row_index_of(item_id) {
            self.notify.row_changed(idx);
        }
    }

    /// 当前直接选中的项（`DirectlySelected` 状态集合）。
    /// 从三态 `selected` 派生（单一来源），供 reducer 读取当前选中集合。
    pub fn directly_selected_items(&self) -> std::collections::HashSet<ItemId> {
        self.selected
            .borrow()
            .iter()
            .filter(|(_, s)| **s == SlintSelectionState::DirectlySelected)
            .map(|(id, _)| *id)
            .collect()
    }

    /// 全部选中状态（(item_id, state)）。
    /// 供 refresh 遍历"已设组头选中态但子项可能已清空"的项（低频）。
    pub fn selected_states(&self) -> Vec<(ItemId, SlintSelectionState)> {
        self.selected
            .borrow()
            .iter()
            .map(|(id, s)| (*id, *s))
            .collect()
    }

    /// 某 item 是否处于 `DirectlySelected` 状态。
    pub fn is_directly_selected(&self, item_id: ItemId) -> bool {
        self.selected
            .borrow()
            .get(&item_id)
            .map(|s| *s == SlintSelectionState::DirectlySelected)
            .unwrap_or(false)
    }

    /// 设置事件时间码拖拽预览覆盖（start/end 之一），不修改 sequence。
    pub fn set_preview_timecode(&self, event_id: ItemId, is_start_time: bool, time: TimeDelta) {
        let mut overrides = self.preview_overrides.borrow_mut();
        let entry = overrides.entry(event_id).or_insert((None, None));
        if is_start_time {
            entry.0 = Some(time);
        } else {
            entry.1 = Some(time);
        }
    }

    /// 清除事件的时间码预览覆盖（commit 或 cancel 后调用）。
    pub fn clear_preview_timecode(&self, event_id: ItemId) {
        self.preview_overrides.borrow_mut().remove(&event_id);
    }

    /// 清除全部时间码预览覆盖。
    pub fn clear_all_previews(&self) {
        self.preview_overrides.borrow_mut().clear();
    }

    /// 更新权威数据引用并重建扁平化索引（结构变化后调用，触发 reset）。
    /// 初始化行快照（从权威数据构建扁平索引 + 静态行内容）。
    /// 权威数据在 model（纯值），一次性借用构建；此后投影按 tag 精确更新受影响行。
    pub fn set_sequence(&self, sequence: &Sequence) {
        let flattened = flatten_sequence(sequence);
        *self.flattened.borrow_mut() = flattened;
        self.notify.reset();
    }

    /// 按 item_id 找当前行索引。
    pub fn row_index_of(&self, item_id: ItemId) -> Option<usize> {
        self.flattened
            .borrow()
            .anchors
            .iter()
            .position(|a| a.item_id == item_id)
    }

    /// 某 item 的祖先分组链（叶子到根，不含自身）。从锚点读，无权威数据访问。
    pub fn ancestors_of(&self, item_id: ItemId) -> Vec<ItemId> {
        self.flattened
            .borrow()
            .anchors
            .iter()
            .find(|a| a.item_id == item_id)
            .map(|a| a.ancestor_ids.as_ref().clone())
            .unwrap_or_default()
    }

    /// 第 i 行的 item_id（行副本替代：从锚点读，O(1)）。
    pub fn item_id_at(&self, row: usize) -> Option<ItemId> {
        self.flattened.borrow().anchors.get(row).map(|a| a.item_id)
    }

    /// 全部行 item_id（扁平化顺序）。
    pub fn all_item_ids(&self) -> Vec<ItemId> {
        self.flattened
            .borrow()
            .anchors
            .iter()
            .map(|a| a.item_id)
            .collect()
    }

    /// 分组头行数（含分组头自身）。从锚点实时算，不依赖可能过期的缓存。
    pub fn group_row_count(&self, group_id: ItemId) -> usize {
        let flattened = self.flattened.borrow();
        let Some(start) = flattened.anchors.iter().position(|a| a.item_id == group_id) else {
            return 1;
        };
        let start_depth = flattened.anchors[start].depth;
        let mut end = start + 1;
        while end < flattened.anchors.len() {
            if flattened.anchors[end].depth <= start_depth {
                break;
            }
            end += 1;
        }
        end - start
    }

    /// 某 item 是否为分组（从快照 content 的 row_type 判断，无权威数据访问）。
    pub fn is_group_item(&self, item_id: ItemId) -> bool {
        self.flattened
            .borrow()
            .anchors
            .iter()
            .find(|a| a.item_id == item_id)
            .map(|a| a.content.row_type == SlintSubtitleRowType::GroupHeader)
            .unwrap_or(false)
    }

    /// 通知某行值变化（时间码/折叠/选中），Slint 精确重算该行。
    pub fn notify_row_changed(&self, row: usize) {
        self.notify.row_changed(row);
    }

    /// 重建某行的 content 快照（值变化后，从权威数据读新值）。
    /// 投影（op_sequence_*）在权威数据变更后调用，再 `notify_row_changed`。
    pub fn rebuild_row_content(&self, row: usize, sequence: &Sequence) {
        let mut flattened = self.flattened.borrow_mut();
        let Some(anchor) = flattened.anchors.get_mut(row) else {
            return;
        };
        let data = RowData::from_anchor(anchor);
        if let Some(new_content) = build_row(&data, sequence) {
            anchor.content = new_content;
        }
    }

    /// 原子刷新一组行：重建 content（从权威数据）+ 自动通知 Slint。
    ///
    /// 投影 op 对受影响的 item_id 调用此方法——重建 + notify 一体化，
    /// 无需手动记住"先 rebuild 再 notify"，避免遗漏。
    pub fn refresh_ids(&self, sequence: &Sequence, item_ids: &[ItemId]) {
        for &id in item_ids {
            if let Some(idx) = self.row_index_of(id) {
                self.rebuild_row_content(idx, sequence);
                self.notify_row_changed(idx);
            }
        }
    }

    /// 原子刷新某行及其所有祖先分组头（聚合值变化时）。
    ///
    /// 时间码/行数等变化会传播到祖先分组头（聚合），此方法一并重建 + 通知，
    /// 避免调用方分别处理"行自身"与"祖先"而遗漏。
    pub fn refresh_row_with_ancestors(&self, sequence: &Sequence, item_id: ItemId) {
        // 自身
        if let Some(idx) = self.row_index_of(item_id) {
            self.rebuild_row_content(idx, sequence);
            self.notify_row_changed(idx);
        }
        // 祖先分组头（锚点 ancestor_ids 叶子到根）
        let ancestors = self.ancestors_of(item_id);
        for anc in ancestors {
            if let Some(idx) = self.row_index_of(anc) {
                self.rebuild_row_content(idx, sequence);
                self.notify_row_changed(idx);
            }
        }
    }

    /// 移除一个 item 的锚点及其子树（结构变化：删除行）。
    ///
    /// 返回被删除的行数，`notify.row_removed`——Slint 只销毁被删实例，存量复用。
    /// 同时更新所有祖先组的 `group_row_counts`（减去被删行数），保持定位准确。
    pub fn remove_anchor(&self, item_id: ItemId) -> usize {
        let mut flattened = self.flattened.borrow_mut();
        let Some(start) = flattened.anchors.iter().position(|a| a.item_id == item_id) else {
            return 0;
        };
        // 该 item 的子树范围：找到下一个同 depth 或更浅的锚点
        let start_depth = flattened.anchors[start].depth;
        let mut end = start + 1;
        while end < flattened.anchors.len() {
            if flattened.anchors[end].depth <= start_depth {
                break;
            }
            end += 1;
        }
        let count = end - start;
        // 记录被删范围内的 item_id（含子树）
        let removed_ids: Vec<ItemId> = flattened.anchors[start..end]
            .iter()
            .map(|a| a.item_id)
            .collect();
        // 更新祖先组行数：被删子树的每个锚点的祖先组都减去本次删除行数。
        // 去重，避免同一祖先组被同一子树多次减去。
        let mut ancestor_groups: Vec<ItemId> = Vec::new();
        for a in &flattened.anchors[start..end] {
            for anc in a.ancestor_ids.iter() {
                if !ancestor_groups.contains(anc) {
                    ancestor_groups.push(*anc);
                }
            }
        }
        flattened.anchors.drain(start..end);
        flattened
            .group_row_counts
            .retain(|k, _| !removed_ids.contains(k));
        for gid in ancestor_groups {
            if let Some(v) = flattened.group_row_counts.get_mut(&gid) {
                *v = v.saturating_sub(count);
            }
        }
        drop(flattened);
        self.notify.row_removed(start, count);
        count
    }

    /// 在参考 item 之后插入新 item 的锚点（含子树）。
    ///
    /// - 参考行是 event → 插到其后（depth 相同）；
    /// - 参考行是 group → 插到其子树末尾（depth+1，作为组内首项）。
    /// `notify.row_added`——Slint 只新建该行实例，存量复用。
    pub fn insert_after(
        &self,
        sequence: &Sequence,
        item_id: ItemId,
        new_item_id: ItemId,
    ) -> usize {
        let mut flattened = self.flattened.borrow_mut();
        let Some(ref_idx) = flattened.anchors.iter().position(|a| a.item_id == item_id) else {
            return 0;
        };
        let is_group_ref = flattened.group_row_counts.contains_key(&item_id);
        let at = if is_group_ref {
            // group：插到子树末尾（ref_idx + group 行数）
            let cnt = flattened
                .group_row_counts
                .get(&item_id)
                .copied()
                .unwrap_or(1);
            ref_idx + cnt
        } else {
            ref_idx + 1
        };
        let depth = anchor_depth(&flattened, at.saturating_sub(1));
        // 新子项的祖先链：插入到组内时，应是"参考组的祖先 + 参考组自身"。
        // 前一锚点（组头）的 ancestor_ids 只含组的祖先，不含组自身——
        // 需补上。插入到事件后（depth 相同）则继承前一锚点完整祖先链。
        let (ancestors, ancestor_ids) = if is_group_ref {
            let ref_anc = flattened.anchors[ref_idx].ancestors.clone();
            let ref_anc_ids = flattened.anchors[ref_idx].ancestor_ids.clone();
            let mut anc = (*ref_anc).clone();
            anc.push(item_id.to_string().to_shared_string());
            let mut anc_ids = (*ref_anc_ids).clone();
            anc_ids.push(item_id);
            (Rc::new(anc), Rc::new(anc_ids))
        } else {
            (
                anchor_ancestors(&flattened, at.saturating_sub(1)),
                anchor_ancestor_ids(&flattened, at.saturating_sub(1)),
            )
        };
        let mut sub = Flattened::default();
        flatten_items(
            &[new_item_id],
            sequence,
            depth,
            ancestors,
            ancestor_ids,
            &mut sub,
        );
        let count = sub.anchors.len();
        let sub_anchors = std::mem::take(&mut sub.anchors);
        let sub_counts = std::mem::take(&mut sub.group_row_counts);
        flattened.anchors.splice(at..at, sub_anchors);
        for (k, v) in sub_counts {
            flattened.group_row_counts.insert(k, v);
        }
        // 更新参考项祖先组的行数：插入的 count 行加入其祖先组。
        // 用插入点的锚点 ancestor_ids 取祖先链（插入后子项锚点已并入，
        // 但参考 item 仍在原位置，取其祖先即可）。
        let mut ancestor_groups: Vec<ItemId> = Vec::new();
        for a in &flattened.anchors[at..at + count] {
            for anc in a.ancestor_ids.iter() {
                if !ancestor_groups.contains(anc) {
                    ancestor_groups.push(*anc);
                }
            }
        }
        for gid in ancestor_groups {
            if let Some(v) = flattened.group_row_counts.get_mut(&gid) {
                *v = v.saturating_add(count);
            }
        }
        drop(flattened);
        self.notify.row_added(at, count);
        count
    }

    /// 在指定索引批量插入多个子树（粘贴）。
    ///
    /// 一次 `notify.row_added(at, total_count)`——Slint splice 批量插入，存量复用。
    pub fn insert_subtrees(
        &self,
        sequence: &Sequence,
        at: usize,
        items: &[ItemId],
    ) -> usize {
        let mut flattened = self.flattened.borrow_mut();
        let depth = anchor_depth(&flattened, at.saturating_sub(1));
        let ancestors = anchor_ancestors(&flattened, at.saturating_sub(1));
        let ancestor_ids = anchor_ancestor_ids(&flattened, at.saturating_sub(1));
        let mut sub = Flattened::default();
        flatten_items(items, sequence, depth, ancestors, ancestor_ids, &mut sub);
        let count = sub.anchors.len();
        if count == 0 {
            return 0;
        }
        let sub_anchors = std::mem::take(&mut sub.anchors);
        let sub_counts = std::mem::take(&mut sub.group_row_counts);
        flattened.anchors.splice(at..at, sub_anchors);
        for (k, v) in sub_counts {
            flattened.group_row_counts.insert(k, v);
        }
        // 更新插入位置祖先组的行数：插入的 count 行加入其祖先组。
        let mut ancestor_groups: Vec<ItemId> = Vec::new();
        for a in &flattened.anchors[at..at + count] {
            for anc in a.ancestor_ids.iter() {
                if !ancestor_groups.contains(anc) {
                    ancestor_groups.push(*anc);
                }
            }
        }
        for gid in ancestor_groups {
            if let Some(v) = flattened.group_row_counts.get_mut(&gid) {
                *v = v.saturating_add(count);
            }
        }
        drop(flattened);
        self.notify.row_added(at, count);
        count
    }

    /// 解散分组：移除分组头锚点（`row_removed`），子项 depth-1、祖先链去掉本组。
    ///
    /// 返回 `(分组头行索引, 父分组 id)`（供上层用 row_changed 更新父组头聚合）。
    pub fn ungroup_anchor(
        &self,
        sequence: &Sequence,
        group_id: ItemId,
    ) -> Option<(usize, Option<ItemId>)> {
        let mut flattened = self.flattened.borrow_mut();
        let Some(start) = flattened.anchors.iter().position(|a| a.item_id == group_id) else {
            return None;
        };
        // 父组 id：锚点 ancestor_ids 末尾（若有）
        let parent_id = flattened.anchors[start].ancestor_ids.last().copied();
        let start_depth = flattened.anchors[start].depth;
        let group_id_str = group_id.to_string().to_shared_string();
        // 子树范围 [start+1, end)
        let mut end = start + 1;
        while end < flattened.anchors.len() {
            if flattened.anchors[end].depth <= start_depth {
                break;
            }
            end += 1;
        }
        // 移除分组头锚点
        let removed_ids: Vec<ItemId> = flattened.anchors[start..=start]
            .iter()
            .map(|a| a.item_id)
            .collect();
        flattened.anchors.remove(start);
        // 子项（原 [start+1, end)，现索引 [start, end-1)）depth-1、去祖先链末尾本组，
        // 并重建 content（depth/ancestor_groups 变了）
        for a in &mut flattened.anchors[start..end - 1] {
            a.depth -= 1;
            if a.ancestors.last().map(|s| s.as_str()) == Some(group_id_str.as_str()) {
                let mut anc = (*a.ancestors).clone();
                anc.pop();
                a.ancestors = Rc::new(anc);
            }
            if a.ancestor_ids.last() == Some(&group_id) {
                let mut anc = (*a.ancestor_ids).clone();
                anc.pop();
                a.ancestor_ids = Rc::new(anc);
            }
            let data = RowData::from_anchor(a);
            if let Some(new_content) = build_row(&data, sequence) {
                a.content = new_content;
            }
        }
        // 被删组 G 的所有祖先组行数减 1（组头消失，子项仍在）。
        // 用 G 锚点的 ancestor_ids 取祖先链。
        let ancestor_groups: Vec<ItemId> = flattened.anchors[start].ancestor_ids.as_ref().clone();
        flattened
            .group_row_counts
            .retain(|k, _| !removed_ids.contains(k));
        for gid in ancestor_groups {
            if let Some(v) = flattened.group_row_counts.get_mut(&gid) {
                *v = v.saturating_sub(1);
            }
        }
        drop(flattened);
        self.notify.row_removed(start, 1);
        for i in start..end - 1 {
            self.notify.row_changed(i);
        }
        Some((start, parent_id))
    }
}

impl Default for SequenceRowsModel {
    fn default() -> Self {
        Self::new()
    }
}

/// 行映射器：把轻量 `RowData` 映射为 `SlintSubtitleRow`。
///
/// 只持动态 UI 状态（折叠/选中/预览），供 `MapModel` 的 map_function 使用。
/// 权威行内容在 `RowData.content` 快照（投影时构建），map 叠加动态状态。
#[derive(Clone)]
pub struct SequenceRowMapper {
    collapsed: Rc<RefCell<std::collections::HashSet<ItemId>>>,
    selected: Rc<RefCell<std::collections::HashMap<ItemId, SlintSelectionState>>>,
    preview_overrides:
        Rc<RefCell<std::collections::HashMap<ItemId, (Option<TimeDelta>, Option<TimeDelta>)>>>,
}

impl SequenceRowMapper {
    /// 映射一行数据为 Slint UI 行：content 快照 + 叠加动态状态。
    pub fn map(&self, data: &RowData) -> SlintSubtitleRow {
        let collapsed = self.collapsed.borrow();
        let selected = self.selected.borrow();
        let preview = self.preview_overrides.borrow();
        let mut row = data.content.clone();
        // 叠加动态状态：选中态 / 折叠标记 / 拖拽预览覆盖
        row.selected = selected
            .get(&data.item_id)
            .copied()
            .unwrap_or(SlintSelectionState::Unselected);
        if row.row_type == SlintSubtitleRowType::GroupHeader {
            row.collapsed = collapsed.contains(&data.item_id);
        }
        if let Some((p_start, p_end)) = preview.get(&data.item_id) {
            if let Some(start) = p_start {
                row.start_time = format_time(*start);
                row.start_sec = start.num_milliseconds() as f32 / 1000.0;
            }
            if let Some(end) = p_end {
                row.end_time = format_time(*end);
                row.end_sec = end.num_milliseconds() as f32 / 1000.0;
            }
        }
        row
    }
}

/// 构造一个空/兜底行（item 缺失时）。
fn empty_row(
    item_id: &ItemId,
    depth: i32,
    ancestors: Rc<Vec<SharedString>>,
    _ancestor_ids: Rc<Vec<ItemId>>,
) -> SlintSubtitleRow {
    SlintSubtitleRow {
        row_type: SlintSubtitleRowType::Event,
        depth,
        ancestor_groups: ModelRc::from(ancestors.as_slice()),
        start_time: SharedString::new(),
        end_time: SharedString::new(),
        start_sec: 0.0,
        end_sec: 0.0,
        child_blocks: ModelRc::new(Rc::new(slint::VecModel::from(Vec::<
            SlintTimelineSubBlock,
        >::new()))),
        item_id: item_id.to_string().to_shared_string(),
        selected: SlintSelectionState::Unselected,
        total_row_count: 1,
        text: SharedString::new(),
        visable: true,
        group_name: SharedString::new(),
        collapsed: false,
    }
}

/// 构造单行静态内容（从权威数据，投影时构建；不含动态 UI 状态）。
/// 动态状态（selected/collapsed/preview）由 `SequenceRowMapper::map` 叠加。
fn build_row(data: &RowData, sequence: &Sequence) -> Option<SlintSubtitleRow> {
    let item = sequence.get(data.item_id)?;
    let item_id_str = data.item_id.to_string().to_shared_string();

    match &item.content {
        ItemContent::Event(event) => Some(SlintSubtitleRow {
            row_type: SlintSubtitleRowType::Event,
            depth: data.depth,
            ancestor_groups: ModelRc::from(data.ancestors.as_slice()),
            start_time: format_time(event.start_time),
            end_time: format_time(event.end_time),
            start_sec: event.start_time.num_milliseconds() as f32 / 1000.0,
            end_sec: event.end_time.num_milliseconds() as f32 / 1000.0,
            child_blocks: ModelRc::new(Rc::new(slint::VecModel::from(Vec::<
                SlintTimelineSubBlock,
            >::new()))),
            item_id: item_id_str,
            selected: SlintSelectionState::Unselected,
            text: event.text.to_shared_string(),
            visable: item.visible,
            group_name: SharedString::new(),
            collapsed: false,
            total_row_count: 1,
        }),
        ItemContent::Group(group) => {
            // 分组头：聚合子树时间与行数（投影时计算）
            let (bounds, count) = aggregate_group(data.item_id, sequence);
            let (start, end, start_sec, end_sec) = match bounds {
                Some((s, e)) => (
                    format_time(chrono::TimeDelta::milliseconds(s)),
                    format_time(chrono::TimeDelta::milliseconds(e)),
                    s as f32 / 1000.0,
                    e as f32 / 1000.0,
                ),
                None => (SharedString::new(), SharedString::new(), 0.0, 0.0),
            };
            // 收集组内所有字幕行块（递归，时间重叠允许）
            let child_blocks = collect_child_blocks(&group.children, sequence);
            Some(SlintSubtitleRow {
                row_type: SlintSubtitleRowType::GroupHeader,
                depth: data.depth,
                ancestor_groups: ModelRc::from(data.ancestors.as_slice()),
                start_time: start,
                end_time: end,
                start_sec,
                end_sec,
                child_blocks: ModelRc::new(Rc::new(slint::VecModel::from(child_blocks))),
                item_id: item_id_str,
                selected: SlintSelectionState::Unselected,
                text: SharedString::new(),
                visable: item.visible,
                group_name: group.name.to_shared_string(),
                collapsed: false,
                total_row_count: count as i32,
            })
        }
    }
}

/// 聚合分组子树的时间范围与行数（含分组头自身）。
fn aggregate_group(group_id: ItemId, sequence: &Sequence) -> (Option<(i64, i64)>, usize) {
    let Some(group) = sequence.get(group_id) else {
        return (None, 1);
    };
    let ItemContent::Group(group_data) = &group.content else {
        return (None, 1);
    };
    let mut min_start: Option<i64> = None;
    let mut max_end: Option<i64> = None;
    let mut count = 1; // 分组头自身
    for child_id in &group_data.children {
        if let Some(child) = sequence.get(*child_id) {
            count += 1; // 占位（child 若为分组，会展开更多行）
            match &child.content {
                ItemContent::Event(e) => {
                    let s = e.start_time.num_milliseconds();
                    let en = e.end_time.num_milliseconds();
                    min_start = Some(min_start.map_or(s, |x| x.min(s)));
                    max_end = Some(max_end.map_or(en, |x| x.max(en)));
                }
                ItemContent::Group(_) => {
                    let (cb, cc) = aggregate_group(*child_id, sequence);
                    count += cc - 1; // 加上 child 组内的额外行
                    if let Some((s, e)) = cb {
                        min_start = Some(min_start.map_or(s, |x| x.min(s)));
                        max_end = Some(max_end.map_or(e, |x| x.max(e)));
                    }
                }
            }
        }
    }
    let bounds = match (min_start, max_end) {
        (Some(s), Some(e)) => Some((s, e)),
        _ => None,
    };
    (bounds, count)
}

/// 递归收集分组内所有字幕行的时间块（时间线折叠组头时在组头泳道显示）。
/// 嵌套分组递归展开；只收集有有效时间范围的字幕行。
fn collect_child_blocks(
    children: &[ItemId],
    sequence: &Sequence,
) -> Vec<SlintTimelineSubBlock> {
    let mut out: Vec<SlintTimelineSubBlock> = Vec::new();
    for id in children {
        let Some(item) = sequence.get(*id) else {
            continue;
        };
        match &item.content {
            ItemContent::Event(event) => {
                let start = event.start_time.num_milliseconds() as f32 / 1000.0;
                let end = event.end_time.num_milliseconds() as f32 / 1000.0;
                if end > start {
                    out.push(SlintTimelineSubBlock {
                        item_id: id.to_string().to_shared_string(),
                        name: event.text.to_shared_string(),
                        start_sec: start,
                        end_sec: end,
                    });
                }
            }
            ItemContent::Group(group) => {
                out.extend(collect_child_blocks(&group.children, sequence));
            }
        }
    }
    out
}

/// 扁平化：前序遍历，生成锚点列表 + 聚合缓存。
fn flatten_sequence(sequence: &Sequence) -> Flattened {
    let mut out = Flattened::default();
    flatten_items(
        &sequence.root_items,
        sequence,
        0,
        Rc::new(Vec::new()),
        Rc::new(Vec::new()),
        &mut out,
    );
    out
}

fn flatten_items(
    items: &[ItemId],
    sequence: &Sequence,
    depth: i32,
    ancestors: Rc<Vec<SharedString>>,
    ancestor_ids: Rc<Vec<ItemId>>,
    out: &mut Flattened,
) {
    for item_id in items {
        let Some(item) = sequence.get(*item_id) else {
            continue;
        };
        // 轻量行数据（供 build_row 构造静态 content；content 暂为占位）
        let row_data = RowData {
            item_id: *item_id,
            depth,
            ancestors: ancestors.clone(),
            ancestor_ids: ancestor_ids.clone(),
            content: empty_row(&item_id, depth, ancestors.clone(), ancestor_ids.clone()),
        };
        let content = build_row(&row_data, sequence)
            .unwrap_or_else(|| empty_row(&item_id, depth, ancestors.clone(), ancestor_ids.clone()));
        let mut anchor = RowAnchor {
            item_id: *item_id,
            depth,
            ancestors: ancestors.clone(),
            ancestor_ids: ancestor_ids.clone(),
            content,
        };
        match &item.content {
            ItemContent::Event(_) => {
                out.anchors.push(anchor);
            }
            ItemContent::Group(_) => {
                let group_id_str = item_id.to_string().to_shared_string();
                let (_, count) = aggregate_group(*item_id, sequence);
                out.group_row_counts.insert(*item_id, count);
                // 分组头 content 的 total_row_count 需用聚合 count
                anchor.content.total_row_count = count as i32;
                out.anchors.push(anchor);

                let mut new_anc = (*ancestors).clone();
                new_anc.push(group_id_str);
                let mut new_anc_ids = (*ancestor_ids).clone();
                new_anc_ids.push(*item_id);
                let children = match &item.content {
                    ItemContent::Group(g) => g.children.clone(),
                    _ => Vec::new(),
                };
                flatten_items(
                    &children,
                    sequence,
                    depth + 1,
                    Rc::new(new_anc),
                    Rc::new(new_anc_ids),
                    out,
                );
            }
        }
    }
}

// ── 结构操作辅助 ──────────────────────────────────────────────────────────

/// 第 at 个锚点的 depth（越界时取末尾 depth 或 0）。
fn anchor_depth(flattened: &Flattened, at: usize) -> i32 {
    if flattened.anchors.is_empty() {
        return 0;
    }
    if at < flattened.anchors.len() {
        flattened.anchors[at].depth
    } else {
        flattened.anchors.last().map(|a| a.depth).unwrap_or(0)
    }
}

/// 第 at 个锚点的祖先链（越界时取末尾祖先或空）。
fn anchor_ancestors(flattened: &Flattened, at: usize) -> Rc<Vec<SharedString>> {
    if at < flattened.anchors.len() {
        flattened.anchors[at].ancestors.clone()
    } else {
        flattened
            .anchors
            .last()
            .map(|a| a.ancestors.clone())
            .unwrap_or_else(|| Rc::new(Vec::new()))
    }
}

/// 第 at 个锚点的祖先 ItemId 链（越界时取末尾祖先或空）。
fn anchor_ancestor_ids(flattened: &Flattened, at: usize) -> Rc<Vec<ItemId>> {
    if at < flattened.anchors.len() {
        flattened.anchors[at].ancestor_ids.clone()
    } else {
        flattened
            .anchors
            .last()
            .map(|a| a.ancestor_ids.clone())
            .unwrap_or_else(|| Rc::new(Vec::new()))
    }
}

impl Model for SequenceRowsModel {
    type Data = RowData;

    fn row_count(&self) -> usize {
        self.flattened.borrow().anchors.len()
    }

    fn row_data(&self, row: usize) -> Option<Self::Data> {
        let flattened = self.flattened.borrow();
        let anchor = flattened.anchors.get(row)?;
        Some(RowData::from_anchor(anchor))
    }

    fn model_tracker(&self) -> &dyn ModelTracker {
        self.notify.as_ref()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use aweft_model::{
        ids::ItemId,
        sequence::{EventData, GroupData, ItemLocation, Sequence},
    };
    use chrono::TimeDelta;

    fn make_sequence() -> Sequence {
        let mut s = Sequence::new();
        // root event
        let e1 = EventData {
            start_time: TimeDelta::try_milliseconds(0).unwrap(),
            end_time: TimeDelta::try_milliseconds(1000).unwrap(),
            actor: None,
            text: "第一行".into(),
            segment_timing: None,
            style: "Default".into(),
        };
        s.create_event(e1, ItemLocation::Root(0)).unwrap();
        // group with child
        let g = GroupData {
            name: "组".into(),
            collapsed: false,
            color: None,
            children: Vec::new(),
        };
        let gid = s.create_group(g, ItemLocation::Root(1)).unwrap();
        let e2 = EventData {
            start_time: TimeDelta::try_milliseconds(500).unwrap(),
            end_time: TimeDelta::try_milliseconds(900).unwrap(),
            actor: None,
            text: "组内行".into(),
            segment_timing: None,
            style: "Default".into(),
        };
        s.create_event(e2, ItemLocation::InGroup(gid, 0)).unwrap();
        s
    }

    #[test]
    fn row_count_matches_flattened() {
        let model = SequenceRowsModel::new();
        model.set_sequence(&make_sequence());
        assert_eq!(model.row_count(), 3); // event + groupheader + child event
    }

    #[test]
    fn nested_group_total_row_count_matches_flattened() {
        // 验证：分组头的 total_row_count（aggregate_group）与实际锚点行数一致。
        // A 头 + event1 + B 头 + event2 = 4 行
        let mut s = Sequence::new();
        let e1 = EventData {
            start_time: TimeDelta::try_milliseconds(0).unwrap(),
            end_time: TimeDelta::try_milliseconds(100).unwrap(),
            actor: None,
            text: "e1".into(),
            segment_timing: None,
            style: "Default".into(),
        };
        s.create_event(e1, ItemLocation::Root(0)).unwrap();
        let a = s
            .create_group(
                GroupData {
                    name: "A".into(),
                    collapsed: false,
                    color: None,
                    children: Vec::new(),
                },
                ItemLocation::Root(1),
            )
            .unwrap();
        let e2 = EventData {
            start_time: TimeDelta::try_milliseconds(200).unwrap(),
            end_time: TimeDelta::try_milliseconds(300).unwrap(),
            actor: None,
            text: "e2".into(),
            segment_timing: None,
            style: "Default".into(),
        };
        s.create_event(e2, ItemLocation::InGroup(a, 0)).unwrap();
        let b = s
            .create_group(
                GroupData {
                    name: "B".into(),
                    collapsed: false,
                    color: None,
                    children: Vec::new(),
                },
                ItemLocation::InGroup(a, 1),
            )
            .unwrap();
        let e3 = EventData {
            start_time: TimeDelta::try_milliseconds(400).unwrap(),
            end_time: TimeDelta::try_milliseconds(500).unwrap(),
            actor: None,
            text: "e3".into(),
            segment_timing: None,
            style: "Default".into(),
        };
        s.create_event(e3, ItemLocation::InGroup(b, 0)).unwrap();

        let model = SequenceRowsModel::new();
        model.set_sequence(&s);
        assert_eq!(model.row_count(), 5, "e1 + A头 + e2 + B头 + e3 = 5 行");

        let a_idx = model.row_index_of(a).unwrap();
        let a_data = model.row_data(a_idx).unwrap();
        let a_row = model.mapper().map(&a_data);
        assert_eq!(
            a_row.total_row_count, 4,
            "A 头 total_row_count 应等于 A 子树行数（A头+e2+B头+e3）"
        );

        let b_idx = model.row_index_of(b).unwrap();
        let b_data = model.row_data(b_idx).unwrap();
        let b_row = model.mapper().map(&b_data);
        assert_eq!(b_row.total_row_count, 2, "B 头 + e3 = 2 行");
    }

    #[test]
    fn row_data_builds_event() {
        let model = SequenceRowsModel::new();
        model.set_sequence(&make_sequence());
        let data = model.row_data(0).unwrap();
        assert_eq!(data.depth, 0);
        let row = model.mapper().map(&data);
        assert_eq!(row.row_type, SlintSubtitleRowType::Event);
        assert_eq!(row.text.to_string(), "第一行");
        assert_eq!(row.start_time.to_string(), "0:00:00.00");
    }

    #[test]
    fn group_header_aggregates() {
        let model = SequenceRowsModel::new();
        model.set_sequence(&make_sequence());
        let data = model.row_data(1).unwrap();
        let row = model.mapper().map(&data);
        assert_eq!(row.row_type, SlintSubtitleRowType::GroupHeader);
        assert_eq!(row.group_name.to_string(), "组");
        // 子事件 500-900ms → 分组头聚合时间
        assert_eq!(row.start_time.to_string(), "0:00:00.50");
        assert_eq!(row.end_time.to_string(), "0:00:00.90");
        // 行数 = 分组头 + 子事件 = 2
        assert_eq!(row.total_row_count, 2);
    }

    #[test]
    fn row_index_of_locates_item() {
        let sequence = make_sequence();
        let model = SequenceRowsModel::new();
        model.set_sequence(&sequence);
        // 找组内事件的 index（用同一个 sequence 的 id）
        let child_id: ItemId = sequence
            .items
            .iter()
            .find(|(_, it)| matches!(it.content, ItemContent::Event(_)))
            .map(|(id, _)| *id)
            .unwrap();
        assert!(model.row_index_of(child_id).is_some());
    }

    #[test]
    fn insert_after_adds_anchor_and_notifies() {
        // 构造：根事件后追加一个新事件
        let mut sequence = Sequence::new();
        let root_event = sequence
            .create_event(
                EventData {
                    start_time: TimeDelta::try_milliseconds(0).unwrap(),
                    end_time: TimeDelta::try_milliseconds(1000).unwrap(),
                    actor: None,
                    text: "第一行".into(),
                    segment_timing: None,
                    style: "Default".into(),
                },
                ItemLocation::Root(0),
            )
            .unwrap();
        let new_id = sequence
            .create_event(
                EventData {
                    start_time: TimeDelta::try_milliseconds(2000).unwrap(),
                    end_time: TimeDelta::try_milliseconds(3000).unwrap(),
                    actor: None,
                    text: "插入行".into(),
                    segment_timing: None,
                    style: "Default".into(),
                },
                ItemLocation::Root(0),
            )
            .unwrap();

        let model = SequenceRowsModel::new();
        model.set_sequence(&sequence);
        let before = model.row_count();

        model.insert_after(&sequence, root_event, new_id);
        assert_eq!(model.row_count(), before + 1);

        let idx = model.row_index_of(new_id).unwrap();
        let data = model.row_data(idx).unwrap();
        let row = model.mapper().map(&data);
        assert_eq!(row.text.to_string(), "插入行");
    }

    #[test]
    fn remove_anchor_removes_subtree_and_notifies() {
        let sequence = make_sequence();
        let model = SequenceRowsModel::new();
        model.set_sequence(&sequence);

        // 找分组（组 + 子事件 = 2 行）
        let group_id: ItemId = sequence
            .items
            .iter()
            .find(|(_, it)| matches!(it.content, ItemContent::Group(_)))
            .map(|(id, _)| *id)
            .unwrap();
        let before = model.row_count();
        let removed = model.remove_anchor(group_id);
        assert_eq!(removed, 2); // 分组头 + 子事件
        assert_eq!(model.row_count(), before - 2);
    }

    #[test]
    fn remove_multiple_anchors_in_arbitrary_order() {
        // 验证：任意顺序删除多个 item（如 HashSet 迭代顺序），行数仍正确。
        // remove_anchor 用 row_index_of 实时查找，删除后索引平移不应影响
        // 后续删除（每次删都重新定位）。
        let mut sequence = Sequence::new();
        let e1 = sequence
            .create_event(
                EventData {
                    start_time: TimeDelta::try_milliseconds(0).unwrap(),
                    end_time: TimeDelta::try_milliseconds(100).unwrap(),
                    actor: None,
                    text: "e1".into(),
                    segment_timing: None,
                    style: "Default".into(),
                },
                ItemLocation::Root(0),
            )
            .unwrap();
        let e2 = sequence
            .create_event(
                EventData {
                    start_time: TimeDelta::try_milliseconds(200).unwrap(),
                    end_time: TimeDelta::try_milliseconds(300).unwrap(),
                    actor: None,
                    text: "e2".into(),
                    segment_timing: None,
                    style: "Default".into(),
                },
                ItemLocation::Root(1),
            )
            .unwrap();
        let e3 = sequence
            .create_event(
                EventData {
                    start_time: TimeDelta::try_milliseconds(400).unwrap(),
                    end_time: TimeDelta::try_milliseconds(500).unwrap(),
                    actor: None,
                    text: "e3".into(),
                    segment_timing: None,
                    style: "Default".into(),
                },
                ItemLocation::Root(2),
            )
            .unwrap();
        let model = SequenceRowsModel::new();
        model.set_sequence(&sequence);
        assert_eq!(model.row_count(), 3);

        // 任意顺序删除：先删中间，再删末尾
        model.remove_anchor(e2);
        assert_eq!(model.row_count(), 2);
        assert!(model.row_index_of(e1).is_some());
        assert!(model.row_index_of(e3).is_some());

        model.remove_anchor(e1);
        assert_eq!(model.row_count(), 1);
        assert!(model.row_index_of(e3).is_some());
    }

    #[test]
    fn insert_subtrees_batch_adds_multiple_rows() {
        let mut sequence = Sequence::new();
        let anchor = sequence
            .create_event(
                EventData {
                    start_time: TimeDelta::try_milliseconds(0).unwrap(),
                    end_time: TimeDelta::try_milliseconds(1000).unwrap(),
                    actor: None,
                    text: "锚点".into(),
                    segment_timing: None,
                    style: "Default".into(),
                },
                ItemLocation::Root(0),
            )
            .unwrap();
        let e1 = sequence
            .create_event(
                EventData {
                    start_time: TimeDelta::try_milliseconds(1000).unwrap(),
                    end_time: TimeDelta::try_milliseconds(2000).unwrap(),
                    actor: None,
                    text: "粘贴1".into(),
                    segment_timing: None,
                    style: "Default".into(),
                },
                ItemLocation::Root(1),
            )
            .unwrap();
        let e2 = sequence
            .create_event(
                EventData {
                    start_time: TimeDelta::try_milliseconds(2000).unwrap(),
                    end_time: TimeDelta::try_milliseconds(3000).unwrap(),
                    actor: None,
                    text: "粘贴2".into(),
                    segment_timing: None,
                    style: "Default".into(),
                },
                ItemLocation::Root(2),
            )
            .unwrap();

        let model = SequenceRowsModel::new();
        model.set_sequence(&sequence);
        let before = model.row_count();

        let at = model.row_index_of(anchor).unwrap() + 1;
        let added = model.insert_subtrees(&sequence, at, &[e1, e2]);
        assert_eq!(added, 2);
        assert_eq!(model.row_count(), before + 2);

        // 顺序验证：锚点, 粘贴1, 粘贴2
        assert_eq!(model.item_id_at(at), Some(e1));
        assert_eq!(model.item_id_at(at + 1), Some(e2));
    }

    #[test]
    fn ungroup_anchor_removes_header_and_promotes_children() {
        let sequence = make_sequence();
        let model = SequenceRowsModel::new();
        model.set_sequence(&sequence);

        // 找分组（组头 + 子事件），记录子事件行数据
        let group_id: ItemId = sequence
            .items
            .iter()
            .find(|(_, it)| matches!(it.content, ItemContent::Group(_)))
            .map(|(id, _)| *id)
            .unwrap();
        let child_before = model.row_data(2).unwrap(); // group 子事件原本 index 2
        let before = model.row_count();

        let (group_idx, parent) = model.ungroup_anchor(&sequence, group_id).unwrap();
        assert_eq!(parent, None); // 根级分组，无父
        assert_eq!(model.row_count(), before - 1); // 只删分组头
        // 子事件提升为根级（index 移到原来 group 位置）
        let child_after = model.row_data(group_idx).unwrap();
        assert_eq!(child_after.item_id, child_before.item_id);
        assert_eq!(child_after.depth, 0); // 提升到根级
    }

    /// 验证：删除组内事件后，祖先组的 group_row_counts 同步减少，
    /// 后续 insert_after 到参考组末尾仍定位正确（不因陈旧计数错位）。
    #[test]
    fn remove_after_updates_ancestor_group_row_count() {
        use aweft_model::sequence::GroupData;
        // 根组 P 含：子组 G（含 e1,e2,e3）+ 尾部事件 e_tail
        let mut s = Sequence::new();
        let p = s
            .create_group(
                GroupData {
                    name: "P".into(),
                    collapsed: false,
                    color: None,
                    children: Vec::new(),
                },
                ItemLocation::Root(0),
            )
            .unwrap();
        let g = s
            .create_group(
                GroupData {
                    name: "G".into(),
                    collapsed: false,
                    color: None,
                    children: Vec::new(),
                },
                ItemLocation::InGroup(p, 0),
            )
            .unwrap();
        let mut e_ids = Vec::new();
        for i in 0..3 {
            let e = s
                .create_event(
                    EventData {
                        start_time: TimeDelta::try_milliseconds(i as i64 * 100).unwrap(),
                        end_time: TimeDelta::try_milliseconds(i as i64 * 100 + 50).unwrap(),
                        actor: None,
                        text: format!("e{}", i),
                        segment_timing: None,
                        style: "Default".into(),
                    },
                    ItemLocation::InGroup(g, i),
                )
                .unwrap();
            e_ids.push(e);
        }
        let e_tail = s
            .create_event(
                EventData {
                    start_time: TimeDelta::try_milliseconds(400).unwrap(),
                    end_time: TimeDelta::try_milliseconds(500).unwrap(),
                    actor: None,
                    text: "e_tail".into(),
                    segment_timing: None,
                    style: "Default".into(),
                },
                ItemLocation::InGroup(p, 1),
            )
            .unwrap();
        let mut sequence = s;
        let model = SequenceRowsModel::new();
        model.set_sequence(&sequence);
        // 行序：P头(0), G头(1), e0(2), e1(3), e2(4), e_tail(5) = 6 行

        // 删 e2（G 内第三个事件）。祖先组 [P, G] 都应减 1。
        model.remove_anchor(e_ids[2]);
        // 行序：P头, G头, e0, e1, e_tail = 5 行
        assert_eq!(model.row_count(), 5);

        // 插入到 G 末尾：G 实际行数 = 头 + e0 + e1 = 3。
        // 若 group_row_counts[G] 陈旧（仍 4），at = G_idx(1) + 4 = 5，
        // 会插到 e_tail 之后（错）；正确 at = 1 + 3 = 4（e_tail 之前）。
        let new_id = sequence
            .create_event(
                EventData {
                    start_time: TimeDelta::try_milliseconds(600).unwrap(),
                    end_time: TimeDelta::try_milliseconds(700).unwrap(),
                    actor: None,
                    text: "new".into(),
                    segment_timing: None,
                    style: "Default".into(),
                },
                ItemLocation::InGroup(g, 2),
            )
            .unwrap();
        model.insert_after(&sequence, g, new_id);
        // 行序：P头, G头, e0, e1, new, e_tail = 6 行
        assert_eq!(model.row_count(), 6);
        let new_idx = model.row_index_of(new_id).unwrap();
        assert_eq!(
            new_idx, 4,
            "new 应插到 G 末尾（e_tail 之前），实际 {}",
            new_idx
        );
        assert_eq!(model.row_index_of(e_tail), Some(5), "e_tail 应在 new 之后");
    }

    /// 验证：解散子组后，祖先组 group_row_counts 同步减 1（组头消失）。
    #[test]
    fn ungroup_updates_ancestor_group_row_count() {
        use aweft_model::sequence::GroupData;
        let mut s = Sequence::new();
        let p = s
            .create_group(
                GroupData {
                    name: "P".into(),
                    collapsed: false,
                    color: None,
                    children: Vec::new(),
                },
                ItemLocation::Root(0),
            )
            .unwrap();
        let g = s
            .create_group(
                GroupData {
                    name: "G".into(),
                    collapsed: false,
                    color: None,
                    children: Vec::new(),
                },
                ItemLocation::InGroup(p, 0),
            )
            .unwrap();
        let e2 = s
            .create_event(
                EventData {
                    start_time: TimeDelta::try_milliseconds(200).unwrap(),
                    end_time: TimeDelta::try_milliseconds(300).unwrap(),
                    actor: None,
                    text: "e2".into(),
                    segment_timing: None,
                    style: "Default".into(),
                },
                ItemLocation::InGroup(g, 0),
            )
            .unwrap();
        let mut sequence = s;
        let model = SequenceRowsModel::new();
        model.set_sequence(&sequence);
        // 行序：P头, G头, e2 = 3 行

        // 解散 G：删组头 1 行，e2 提升到 P。P 行数应减 1。
        model.ungroup_anchor(&sequence, g);
        // 行序：P头, e2 = 2 行
        assert_eq!(model.row_count(), 2);
        assert_eq!(model.row_index_of(e2), Some(1), "e2 提升到 P 头之后");

        // 插入到 P 末尾：P 现在 2 行，新项应插到末尾（index 2）
        let new_id = sequence
            .create_event(
                EventData {
                    start_time: TimeDelta::try_milliseconds(400).unwrap(),
                    end_time: TimeDelta::try_milliseconds(500).unwrap(),
                    actor: None,
                    text: "new".into(),
                    segment_timing: None,
                    style: "Default".into(),
                },
                ItemLocation::InGroup(p, 1),
            )
            .unwrap();
        model.insert_after(&sequence, p, new_id);
        assert_eq!(model.row_index_of(new_id), Some(2), "new 应插到 P 末尾");
    }

    /// 验证：连续插入到同一组，group_row_counts 同步增长，定位始终正确。
    #[test]
    fn insert_after_updates_ancestor_group_row_count() {
        use aweft_model::sequence::GroupData;
        let mut s = Sequence::new();
        let p = s
            .create_group(
                GroupData {
                    name: "P".into(),
                    collapsed: false,
                    color: None,
                    children: Vec::new(),
                },
                ItemLocation::Root(0),
            )
            .unwrap();
        let mut sequence = s;
        let model = SequenceRowsModel::new();
        model.set_sequence(&sequence);
        // 行序：P头 = 1 行

        // 连续插 3 个事件到 P 末尾
        let mut ids = Vec::new();
        for i in 0..3 {
            let e = sequence
                .create_event(
                    EventData {
                        start_time: TimeDelta::try_milliseconds(i as i64 * 100).unwrap(),
                        end_time: TimeDelta::try_milliseconds(i as i64 * 100 + 50).unwrap(),
                        actor: None,
                        text: format!("e{}", i),
                        segment_timing: None,
                        style: "Default".into(),
                    },
                    ItemLocation::InGroup(p, i),
                )
                .unwrap();
            model.insert_after(&sequence, p, e);
            ids.push(e);
        }
        // 行序：P头, e0, e1, e2 = 4 行
        assert_eq!(model.row_count(), 4);
        for (i, id) in ids.iter().enumerate() {
            assert_eq!(
                model.row_index_of(*id),
                Some(i + 1),
                "e{} 应在 index {}",
                i,
                i + 1
            );
        }

        // 再插一个组 G 到 P 末尾，验证 G 作为分组插入定位正确
        let g = sequence
            .create_group(
                GroupData {
                    name: "G".into(),
                    collapsed: false,
                    color: None,
                    children: Vec::new(),
                },
                ItemLocation::InGroup(p, 3),
            )
            .unwrap();
        model.insert_after(&sequence, p, g);
        assert_eq!(model.row_index_of(g), Some(4), "G 应插到 P 末尾");
    }
}
