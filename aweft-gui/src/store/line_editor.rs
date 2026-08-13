use std::{
    collections::{HashMap, HashSet},
    rc::Rc,
};

use anyhow::Result;
use aweft_model::{
    ids::ItemId,
    project::Project,
    structure::{ItemContent, ItemLocation, ProjectStructure},
};
use chrono::TimeDelta;
use slint::{Model, ModelRc};

use crate::bridge::{SlintSelectionState, SlintSubtitleRow};
use crate::store::line_rows_model::{LineRowsModel, RowData};

type SubtitleFilterModel = slint::FilterModel<Rc<LineRowsModel>, Box<dyn Fn(&RowData) -> bool>>;

pub struct LineEditorState {
    // 数据源模型（Slint Model，实时从权威 project 投影行）
    rows_model: LineRowsModel,
    filter_model: Option<Rc<SubtitleFilterModel>>,
    // 运行时折叠状态集合（共享引用，供 FilterModel 的闭包使用。索引到实际数据）
    collapsed_groups: Rc<std::cell::RefCell<HashSet<ItemId>>>,

    // 选中的行（group 和 event 都包含）。直接选中态由 LineRowsModel.selected
    // （三态 HashMap）派生，此处不存（单一来源）。
    // 显示追踪（重置状态用：记录所有 selection_state != Unselected 的行）
    all_highlighted_items: HashSet<ItemId>,
    // 分组内选中的数量（可以与 total_row_count 结合，快速计算是否全选）
    // 仅用于分组（不包含分组头），不可直接使用 total_row_count （因为比较时无法知道嵌套的分组头有多少个）
    selected_event_counts: HashMap<ItemId, usize>,
    // 分组事件数缓存（count_events_recursive 的缓存）
    event_count_cache: HashMap<ItemId, usize>,
    // 最后选择的 ui 索引（用于处理范围选择）
    last_selection_anchor: Option<usize>,
}

impl LineEditorState {
    pub fn new() -> Self {
        let rows_model = LineRowsModel::new();
        Self {
            collapsed_groups: rows_model.collapsed_groups(),
            rows_model,
            filter_model: None,
            all_highlighted_items: HashSet::new(),
            selected_event_counts: HashMap::new(),
            event_count_cache: HashMap::new(),
            last_selection_anchor: None,
        }
    }

    /// 暴露数据源模型（实现 slint::Model）。
    pub fn rows_model(&self) -> &LineRowsModel {
        &self.rows_model
    }

    /// 创建过滤模型
    ///
    /// 顺序：`LineRowsModel(RowData) → FilterModel → MapModel(SlintSubtitleRow)`。
    /// 过滤在轻量 `RowData` 层做（`ancestor_ids` 是 ItemId，零 UUID parse），
    /// 被过滤掉的行完全不构造 `SlintSubtitleRow`。
    /// 过滤规则：
    /// - 根级别的行（无祖先）始终可见
    /// - 有祖先的行（包括子分组头和事件）只有在所有祖先都未折叠时才可见
    pub fn init_filter_model(&mut self) -> ModelRc<SlintSubtitleRow> {
        // 1. 数据源模型：产出轻量 RowData
        let rows_model: Rc<LineRowsModel> = Rc::new(self.rows_model.clone());
        let mapper = rows_model.mapper();

        // 2. 过滤（RowData 层，用 ItemId 判断折叠祖先，零 parse）
        let collapsed_groups = self.collapsed_groups.clone();
        let filter_predicate: Box<dyn Fn(&RowData) -> bool> = Box::new(move |row: &RowData| {
            // 无祖先的根级别行，始终显示
            if row.ancestor_ids.is_empty() {
                return true;
            }
            // 任一祖先已折叠则隐藏
            let collapsed = collapsed_groups.borrow();
            !row.ancestor_ids.iter().any(|id| collapsed.contains(id))
        });
        let filter_model: Rc<slint::FilterModel<Rc<LineRowsModel>, Box<dyn Fn(&RowData) -> bool>>> =
            Rc::new(slint::FilterModel::new(rows_model, filter_predicate));

        // 3. 映射为 SlintSubtitleRow（MapModel 透传 row_changed）
        let mapped =
            slint::MapModel::new(filter_model.clone(), move |data: RowData| mapper.map(&data));

        self.filter_model = Some(filter_model);
        ModelRc::new(mapped)
    }

    pub fn get_selected_events(&self, project: &Project) -> HashSet<ItemId> {
        self.rows_model
            .directly_selected_items()
            .iter()
            .filter_map(|&id| {
                // 检查是否是事件项
                if let Some(item) = project.structure().get(id) {
                    if matches!(item.content, ItemContent::Event(_)) {
                        Some(id)
                    } else {
                        None
                    }
                } else {
                    None
                }
            })
            .collect()
    }

    /// 当前直接选中的项（包含 group 与 event）。
    ///
    /// 主要供 reducer 在处理 `DeleteItems` 等需要"作用于当前选中集合"的 Action 时读取。
    /// 从 LineRowsModel.selected（三态单一来源）派生。
    pub fn directly_selected_items(&self) -> std::collections::HashSet<ItemId> {
        self.rows_model.directly_selected_items()
    }

    /// 初始化 UI 行数据（首次绑定模型）。
    ///
    /// 仅在初始化时调用。结构变化用精确锚点操作，不得走全量重建。
    pub fn initialize_rows(&mut self, project: &Project) {
        self.rows_model.set_structure(project.structure.clone());
        self.invalidate_event_counts();
    }

    // === 时间码更新 ===

    /// 更新事件行的时间码（数据源模型驱动）。
    ///
    /// 权威数据（project）已由 reducer 修改；这里仅通知 Slint 重算该行与祖先分组头。
    /// `row_data` 从权威数据实时构造，因此无需手动改行副本。
    pub fn update_row_time_without_ancestor_groups(
        &mut self,
        event_id: ItemId,
        _is_start_time: bool,
        _time: TimeDelta,
    ) -> Vec<ItemId> {
        let Some(idx) = self.rows_model.row_index_of(event_id) else {
            return vec![];
        };
        self.rows_model.notify_row_changed(idx);
        self.ancestors_of(event_id)
    }

    /// 更新单个事件行的时间码，并通知所有祖先分组头聚合更新。
    pub fn update_row_time(
        &mut self,
        _project: &Project,
        event_id: ItemId,
        is_start_time: bool,
        time: TimeDelta,
    ) {
        let ancestors = self.update_row_time_without_ancestor_groups(event_id, is_start_time, time);
        for ancestor in &ancestors {
            if let Some(idx) = self.rows_model.row_index_of(*ancestor) {
                self.rows_model.notify_row_changed(idx);
            }
        }
    }

    /// 批量更新多个事件行的时间码，并统一通知所有受影响祖先分组头。
    pub fn update_rows_batch_then_ancestors(
        &mut self,
        _project: &Project,
        event_ids: &[ItemId],
        is_start_time: bool,
        times: &[TimeDelta],
    ) {
        let mut dirty_ancestors: HashSet<ItemId> = HashSet::new();
        for (&id, &time) in event_ids.iter().zip(times.iter()) {
            let ancestors = self.update_row_time_without_ancestor_groups(id, is_start_time, time);
            dirty_ancestors.extend(ancestors);
        }
        for ancestor in dirty_ancestors {
            if let Some(idx) = self.rows_model.row_index_of(ancestor) {
                self.rows_model.notify_row_changed(idx);
            }
        }
    }

    /// 计算某个 item 的祖先分组链（叶子到根，不含自身）。从项目结构读取。
    fn ancestors_of(&self, item_id: ItemId) -> Vec<ItemId> {
        let mut chain = Vec::new();
        let mut cur = self.rows_model.item(item_id).and_then(|i| i.parent);
        while let Some(id) = cur {
            chain.push(id);
            let Some(item) = self.rows_model.item(id) else {
                break;
            };
            cur = item.parent;
        }
        chain
    }

    pub fn toggle_group(&mut self, group_id: &str) {
        if let Ok(group_id) = uuid::Uuid::parse_str(&group_id).map(ItemId) {
            // 同步折叠状态到共享集合（row_data 实时读取，Slint 自动重算图标）
            {
                let mut collapsed = self.collapsed_groups.borrow_mut();
                if collapsed.contains(&group_id) {
                    collapsed.remove(&group_id);
                } else {
                    collapsed.insert(group_id);
                }
            }

            // 通知过滤更新（折叠影响可见性，需重过滤）
            if let Some(model) = &self.filter_model {
                model.reset();
            }
        }
    }

    // 选择相关操作
    pub fn select_single(&mut self, project: &Project, item_id: ItemId) {
        for item in &self.all_highlighted_items {
            self.rows_model
                .set_selected(*item, SlintSelectionState::Unselected);
        }
        self.all_highlighted_items.clear();

        let Some(idx) = self.rows_model.row_index_of(item_id) else {
            return;
        };
        self.last_selection_anchor = Some(idx);

        let is_group = {
            let Some(item) = self.rows_model.item(item_id) else {
                return;
            };
            matches!(item.content, ItemContent::Group(_))
        };

        if is_group {
            let ui_total_rows = self.rows_model.group_row_count(item_id);
            let ui_end = idx + ui_total_rows;
            for absolute_idx in idx..ui_end {
                if let Some(row_item_id) = self.rows_model.item_id_at(absolute_idx) {
                    self.rows_model
                        .set_selected(row_item_id, SlintSelectionState::DirectlySelected);
                    self.all_highlighted_items.insert(row_item_id);
                }
            }
        } else {
            self.rows_model
                .set_selected(item_id, SlintSelectionState::DirectlySelected);
            self.all_highlighted_items.insert(item_id);
        }
        // 组头状态从 selected 派生（重算计数 + 刷新三态）
        self.rebuild_selected_event_counts();
        self.refresh_group_selected_by_counts(project);
    }

    pub fn select_toggle(&mut self, project: &Project, item_id: ItemId) {
        let Some(idx) = self.rows_model.row_index_of(item_id) else {
            return;
        };
        self.last_selection_anchor = Some(idx);
        let was_selected = self.rows_model.is_selected(item_id);
        let is_group = self.rows_model.is_group_item(item_id);

        if is_group {
            let ui_total_rows = self.rows_model.group_row_count(item_id);
            let ui_end = idx + ui_total_rows;
            if was_selected {
                for absolute_idx in idx..ui_end {
                    if let Some(id) = self.rows_model.item_id_at(absolute_idx) {
                        self.rows_model
                            .set_selected(id, SlintSelectionState::Unselected);
                        self.all_highlighted_items.remove(&id);
                    }
                }
            } else {
                for absolute_idx in idx..ui_end {
                    if let Some(id) = self.rows_model.item_id_at(absolute_idx) {
                        self.rows_model
                            .set_selected(id, SlintSelectionState::DirectlySelected);
                        self.all_highlighted_items.insert(id);
                    }
                }
            }
        } else {
            if was_selected {
                self.rows_model
                    .set_selected(item_id, SlintSelectionState::Unselected);
                self.all_highlighted_items.remove(&item_id);
            } else {
                self.rows_model
                    .set_selected(item_id, SlintSelectionState::DirectlySelected);
                self.all_highlighted_items.insert(item_id);
            }
        }
        self.rebuild_selected_event_counts();
        self.refresh_group_selected_by_counts(project);
    }

    pub fn select_range(&mut self, project: &Project, item_id: ItemId) {
        let Some(cur_idx) = self.rows_model.row_index_of(item_id) else {
            return;
        };
        let anchor = self.last_selection_anchor.unwrap_or(cur_idx);
        let (ui_start, ui_end) = if anchor <= cur_idx {
            (anchor, cur_idx)
        } else {
            (cur_idx, anchor)
        };
        for item_id in &self.all_highlighted_items {
            self.rows_model
                .set_selected(*item_id, SlintSelectionState::Unselected);
        }
        self.all_highlighted_items.clear();
        let mut i = ui_start;
        while i <= ui_end {
            let Some(id) = self.rows_model.item_id_at(i) else {
                break;
            };
            if self.rows_model.is_group_item(id) {
                // 组头不直接选中：其状态由 refresh 根据内部子项全选派生
                // （子项逐个选中时经 rebuild_selected_event_counts 累加计数）。
                // 子项全选 → IndirectlySelected；未全选 → Unselected。
            } else {
                self.rows_model
                    .set_selected(id, SlintSelectionState::DirectlySelected);
                self.all_highlighted_items.insert(id);
            }
            i += 1;
        }
        self.rebuild_selected_event_counts();
        self.refresh_group_selected_by_counts(project);
    }

    /// 同步数据源模型到当前权威结构（共享同一 Rc，零拷贝）。
    pub fn sync_structure(&mut self, structure: Rc<std::cell::RefCell<ProjectStructure>>) {
        self.rows_model.sync_structure(structure);
    }

    /// 从 `selected`（三态单一来源）重建各分组"选中事件数"。
    ///
    /// 遍历直接选中（DirectlySelected）的事件，累加其祖先组的计数。
    /// 组头的 IndirectlySelected/Unselected 由此派生——不手动维护计数。
    fn rebuild_selected_event_counts(&mut self) {
        self.selected_event_counts.clear();
        for id in self.rows_model.directly_selected_items() {
            if self.rows_model.is_group_item(id) {
                continue;
            }
            for anc in self.ancestors_of(id) {
                *self.selected_event_counts.entry(anc).or_insert(0) += 1;
            }
        }
    }

    /// 带缓存的事件计数（仅事件数，不含任何分组头）。
    ///
    /// 缓存只跨"单次 select 内多次调用"避免重复递归（父组递归时子组
    /// 值入缓存，子组头单独查询时命中）。结构变化时由投影显式
    /// `invalidate_event_counts` 全清，无需标脏到具体组（Blender 式粗粒度）。
    fn event_count_cached(&mut self, project: &Project, item_id: &ItemId) -> usize {
        if let Some(v) = self.event_count_cache.get(item_id) {
            return *v;
        }
        let structure = &*project.structure();
        let count = match structure.get(*item_id) {
            Some(item) => match &item.content {
                ItemContent::Event(_) => 1,
                ItemContent::Group(g) => {
                    let mut sum = 0;
                    for cid in &g.children {
                        sum += self.event_count_cached(project, cid);
                    }
                    sum
                }
            },
            None => 0,
        };
        self.event_count_cache.insert(*item_id, count);
        count
    }

    /// 结构变化后失效事件数缓存（投影 op_line_structure 入口调用）。
    /// 下次 select 懒重建。
    pub fn invalidate_event_counts(&mut self) {
        self.event_count_cache.clear();
    }

    /// 根据选中计数同步各分组头的选中状态（三态）。
    ///
    /// - 子项全选 + 组头被直接点选 → `DirectlySelected`
    /// - 子项全选 + 组头未被直接点选（如范围选中整组）→ `IndirectlySelected`
    /// - 未全选 → `Unselected`
    ///
    /// 遍历两个来源的并集：有选中子项的组（`selected_event_counts`）+
    /// 当前已设组头选中态但子项可能已全部取消的组（`selected` 中非
    /// Unselected 的组头）——否则组头子项清空后残留旧态。
    fn refresh_group_selected_by_counts(&mut self, project: &Project) {
        let mut gids: HashSet<ItemId> = self.selected_event_counts.keys().cloned().collect();
        // 补上已设组头选中态但不在计数里的组（子项可能已全取消）
        for (id, state) in self.rows_model.selected_states() {
            if state != SlintSelectionState::Unselected && self.rows_model.is_group_item(id) {
                gids.insert(id);
            }
        }
        for gid in gids {
            let expect = self.event_count_cached(project, &gid);
            let cnt = *self.selected_event_counts.get(&gid).unwrap_or(&0);
            let is_all_selected = expect > 0 && cnt == expect;

            let state = if is_all_selected {
                if self.rows_model.is_directly_selected(gid) {
                    SlintSelectionState::DirectlySelected
                } else {
                    SlintSelectionState::IndirectlySelected
                }
            } else {
                SlintSelectionState::Unselected
            };
            self.rows_model.set_selected(gid, state);
        }
    }

    /// 处理项插入后的 UI 更新（精确：插入锚点 + row_added）。
    pub fn handle_item_inserted(
        &mut self,
        _project: &Project,
        item_id: ItemId,
        new_item_id: ItemId,
    ) -> Result<()> {
        // 精确插入锚点（structure 经共享 Rc 已是最新）。
        // 事件数缓存失效由投影 op_line_structure 入口统一处理。
        self.rows_model.insert_after(item_id, new_item_id);
        Ok(())
    }

    /// 处理已从 `project.structure` 移除的项的 UI 投影更新（精确：移除锚点 + row_removed）。
    pub fn handle_items_removed(&mut self, _project: &Project, item_ids: &[ItemId]) {
        // 事件数缓存失效由投影 op_line_structure 入口统一处理。
        for &id in item_ids {
            self.rows_model.remove_anchor(id);
            // 清除该行的拖拽预览覆盖（防止残留显示错误值）
            self.rows_model.clear_preview_timecode(id);
        }
        self.cleanup_selection();
    }

    /// 清理已不存在的选中项（基于模型当前锚点）。
    fn cleanup_selection(&mut self) {
        let alive: HashSet<ItemId> = self.rows_model.all_item_ids().into_iter().collect();
        self.all_highlighted_items.retain(|id| alive.contains(id));
        self.selected_event_counts
            .retain(|id, _| alive.contains(id));
    }

    /// 处理解散分组的 UI 投影更新（精确：删分组头 + 子项提升）。
    pub fn handle_group_ungrouped(
        &mut self,
        _project: &Project,
        group_id: ItemId,
        _new_ids: &[ItemId],
    ) -> Result<()> {
        // 解散不改事件归属（事件数不变），但父组头需 notify_row_changed
        // 强制重算 total_row_count（含组头行数，aggregate_group 实时递归）。
        // 事件数缓存失效由投影 op_line_structure 入口统一处理。
        if let Some((idx, parent)) = self.rows_model.ungroup_anchor(group_id) {
            // 更新父组头聚合（若有父组）。父组在 structure 中仍在，可查。
            if let Some(parent) = parent {
                if let Some(pidx) = self.rows_model.row_index_of(parent) {
                    self.rows_model.notify_row_changed(pidx);
                }
            }
            let _ = idx;
        }
        self.cleanup_selection();
        Ok(())
    }

    /// 处理粘贴子树的 UI 投影更新（精确：批量插入锚点 + 一次 row_added）。
    pub fn handle_subtrees_pasted(
        &mut self,
        project: &Project,
        base_location: ItemLocation,
        new_root_ids: &[ItemId],
    ) -> Result<()> {
        let at = self.compute_insert_at(project, &base_location)?;
        self.rows_model.insert_subtrees(at, new_root_ids);
        // 事件数缓存失效由投影 op_line_structure 入口统一处理。
        if let Some(&first) = new_root_ids.first() {
            self.select_single(project, first);
        }
        Ok(())
    }

    /// 从 ItemLocation 计算扁平化插入索引（粘贴锚点）。
    fn compute_insert_at(&self, project: &Project, location: &ItemLocation) -> Result<usize> {
        use aweft_model::structure::ItemContent as IC;
        let structure = &*project.structure();
        match location {
            ItemLocation::Root(k) => {
                // 第 k 个 root item 之后；若 k 超出 root_items 则插末尾
                if let Some(&prev) = structure.root_items.get(*k) {
                    if let Some(idx) = self.rows_model.row_index_of(prev) {
                        let cnt = self.rows_model.group_row_count(prev);
                        return Ok(idx + cnt);
                    }
                }
                // 没有 prev（插在开头或空）——找第 k 个 root 的位置
                // 简化：若 k=0 插最前；否则找前一个 root
                if *k > 0 {
                    if let Some(&prev) = structure.root_items.get(k - 1) {
                        if let Some(idx) = self.rows_model.row_index_of(prev) {
                            let cnt = self.rows_model.group_row_count(prev);
                            return Ok(idx + cnt);
                        }
                    }
                }
                Ok(0)
            }
            ItemLocation::InGroup(pid, k) => {
                // pid 组内第 k 个子项之后（组内）
                let Some(group) = structure.get(*pid) else {
                    return Ok(self.rows_model.row_count());
                };
                let IC::Group(g) = &group.content else {
                    return Ok(self.rows_model.row_count());
                };
                let Some(&child) = g.children.get(*k) else {
                    // k 超出子项范围：插组末尾
                    if let Some(idx) = self.rows_model.row_index_of(*pid) {
                        let cnt = self.rows_model.group_row_count(*pid);
                        return Ok(idx + cnt);
                    }
                    return Ok(self.rows_model.row_count());
                };
                if let Some(idx) = self.rows_model.row_index_of(child) {
                    let cnt = self.rows_model.group_row_count(child);
                    Ok(idx + cnt)
                } else {
                    Ok(self.rows_model.row_count())
                }
            }
        }
    }
}
