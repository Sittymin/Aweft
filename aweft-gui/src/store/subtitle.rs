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
use log::warn;
use slint::{FilterModel, Model, ModelRc, SharedString, ToSharedString, VecModel};

use crate::bridge::{SlintSubtitleRow, SlintSubtitleRowType, format_time, parse_time};

type SubtitleFilterModel =
    slint::FilterModel<ModelRc<SlintSubtitleRow>, Box<dyn Fn(&SlintSubtitleRow) -> bool>>;

pub struct LineEditorState {
    // UI 数据源（扁平化的完整列表）
    // 分组行数与组时间在当中查找
    all_rows: Rc<VecModel<SlintSubtitleRow>>,
    filter_model: Option<Rc<SubtitleFilterModel>>,
    // 运行时折叠状态集合（共享引用，供 FilterModel 的闭包使用。索引到实际数据）
    collapsed_groups: Rc<std::cell::RefCell<HashSet<ItemId>>>,

    // row_counts 已经失效了的 group
    dirty_row_counts: HashSet<ItemId>,

    // 到 UI 行索引的映射（用于快速定位）
    item_id_to_ui_index: HashMap<ItemId, usize>,
}

impl LineEditorState {
    pub fn new() -> Self {
        Self {
            all_rows: Rc::new(VecModel::default()),
            filter_model: None,
            collapsed_groups: Rc::new(std::cell::RefCell::new(HashSet::new())),
            dirty_row_counts: HashSet::new(),
            item_id_to_ui_index: HashMap::new(),
        }
    }

    /// 创建过滤模型
    ///
    /// 使用 Slint 的 FilterModel 包装数据源，根据折叠状态自动过滤可见行。
    /// 过滤规则：
    /// - 根级别的行（无祖先）始终可见
    /// - 有祖先的行（包括子分组头和事件）只有在所有祖先都未折叠时才可见
    pub fn init_filter_model(&mut self) -> ModelRc<SlintSubtitleRow> {
        let all_rows_model: ModelRc<crate::bridge::types::SlintSubtitleRow> =
            ModelRc::new(self.all_rows.clone());

        let collapsed_groups = self.collapsed_groups.clone();

        let filter_predicate: Box<dyn Fn(&SlintSubtitleRow) -> bool> =
            Box::new(move |row: &SlintSubtitleRow| {
                // 无祖先的根级别行，始终显示
                if row.ancestor_groups.row_count() == 0 {
                    return true;
                }

                // 需要全部祖先都非折叠才显示
                let collapsed = collapsed_groups.borrow();
                for ancestor_id_str in row.ancestor_groups.iter() {
                    if let Ok(group_id) = uuid::Uuid::parse_str(ancestor_id_str.as_str()) {
                        if collapsed.contains(&ItemId(group_id)) {
                            return false;
                        }
                    } else {
                        warn!("row 数据当中存在无效 group_id: {}", ancestor_id_str);
                    }
                }
                true
            });

        let filter_model: Rc<SubtitleFilterModel> =
            Rc::new(FilterModel::new(all_rows_model, filter_predicate));

        self.filter_model = Some(filter_model.clone());
        ModelRc::from(filter_model)
    }

    /// 重建所有 UI 行数据
    ///
    /// 将分组管理器的树形结构扁平化为线性列表，更新到 VecModel 中。
    /// VecModel 的变化会自动触发 FilterModel 重新过滤。
    pub fn rebuild_all_rows(&mut self, project: &Project) {
        let rows = flatten_project_structure(&project.structure);

        self.item_id_to_ui_index.clear();
        for (i, row) in rows.iter().enumerate() {
            if let Ok(uuid) = uuid::Uuid::parse_str(row.item_id.as_str()) {
                self.item_id_to_ui_index.insert(ItemId(uuid), i);
            }
        }

        // 更新数据源，这会自动触发 FilterModel 重新过滤
        // 仅限初始化使用，否则使用局部更新函数
        self.all_rows.set_vec(rows);
        self.dirty_row_counts.clear();
    }

    pub fn update_row_time(
        &mut self,
        project: &Project,
        event_id: ItemId,
        time: TimeDelta,
        is_start_time: bool,
    ) {
        let Some(&idx) = self.item_id_to_ui_index.get(&event_id) else {
            return;
        };
        let Some(mut row) = self.all_rows.row_data(idx) else {
            return;
        };

        if is_start_time {
            row.start_time = format_time(time);
        } else {
            row.end_time = format_time(time);
        }

        // 克隆父级列表以避免借用冲突
        let ancestor_groups = row.ancestor_groups.clone();
        self.all_rows.set_row_data(idx, row);

        // HACK: 这里走楼梯写法是否可以优化?
        // 遍历并更新所有受影响的父级
        let mut cache: TimeBoundsCache = HashMap::new();
        for ancestor_group_str in ancestor_groups.iter() {
            if let Ok(ancestor_id) = uuid::Uuid::parse_str(&ancestor_group_str).map(ItemId) {
                if let Some(&ancestor_idx) = self.item_id_to_ui_index.get(&ancestor_id) {
                    if let Some(mut ancestor_row) = self.all_rows.row_data(ancestor_idx) {
                        // 决定是否可以进行简单的边界扩展
                        let can_quick_update = if is_start_time {
                            parse_time(&ancestor_row.start_time).map_or(false, |val| time < val)
                        } else {
                            parse_time(&ancestor_row.end_time).map_or(false, |val| time > val)
                        };

                        if can_quick_update {
                            // 路径A: 简单更新，成本低
                            if is_start_time {
                                ancestor_row.start_time = format_time(time);
                            } else {
                                ancestor_row.end_time = format_time(time);
                            }
                        } else {
                            // 路径B: 不是简单扩展，需要递归重新计算，成本高
                            if let Some((min_start, max_end)) =
                                find_item_time_bounds_recursive(ancestor_id, project, &mut cache)
                            {
                                // 保持原始意图：只更新与触发事件相关的那个时间字段
                                if is_start_time {
                                    ancestor_row.start_time = format_time(min_start);
                                } else {
                                    ancestor_row.end_time = format_time(max_end);
                                }
                            }
                        }

                        // 无论通过哪个路径修改了 ancestor_row，都在最后统一提交更新。
                        self.all_rows.set_row_data(ancestor_idx, ancestor_row);
                    }
                }
            }
        }
    }

    pub fn toggle_group(&mut self, group_id_str: String) {
        if let Ok(group_id) = uuid::Uuid::parse_str(&group_id_str).map(ItemId) {
            // 同步折叠状态到共享集合
            let is_now_collapsed = {
                let mut collapsed = self.collapsed_groups.borrow_mut();
                if collapsed.contains(&group_id) {
                    collapsed.remove(&group_id);
                    false
                } else {
                    collapsed.insert(group_id);
                    true
                }
            };

            // 通知过滤更新
            if let Some(model) = &self.filter_model {
                model.reset();
            }

            // 更新图标状态
            if let Some(&idx) = self.item_id_to_ui_index.get(&group_id) {
                if let Some(mut row) = self.all_rows.row_data(idx) {
                    row.collapsed = is_now_collapsed;
                    self.all_rows.set_row_data(idx, row);
                }
            }
        }
    }

    pub fn filter_model(&self) -> Option<Rc<SubtitleFilterModel>> {
        self.filter_model.clone()
    }

    // FIXME: 后面实现操作列表时审查一下代码

    // === 局部更新 API ===
    // 这些方法负责在 ProjectStructure 已经修改后，同步更新 UI (all_rows)
    // 而不需要调用 rebuild_all_rows。

    /// 处理项插入后的 UI 更新
    pub fn handle_item_inserted(
        &mut self,
        project: &Project,
        item_id: ItemId,
        location: ItemLocation,
    ) {
        let insert_idx = self.calculate_insert_ui_index(project, location);

        let mut new_rows = Vec::new();
        let ancestor_groups = self.get_ancestor_groups_for_location(location);

        flatten_items(
            &[item_id],
            &project.structure,
            self.calculate_depth(location),
            &ancestor_groups,
            &mut new_rows,
        );

        let count = new_rows.len();
        self.shift_indices_after(insert_idx, count as i32);

        for (i, row) in new_rows.into_iter().enumerate() {
            let target_idx = insert_idx + i;

            if let Ok(uuid) = uuid::Uuid::parse_str(row.item_id.as_str()) {
                self.item_id_to_ui_index.insert(ItemId(uuid), target_idx);
            }

            self.all_rows.insert(target_idx, row);
        }

        if let ItemLocation::InGroup(parent_id, _) = location {
            self.mark_group_chain_dirty(project, parent_id);
            self.update_all_dirty_groups(project);
        }
    }

    /// 处理项移除后的 UI 更新
    pub fn handle_item_removed(&mut self, project: &Project, item_id: ItemId) {
        // 先获取 parent 以便后续标记 dirty
        // 注意：此时 ProjectStructure 中 item 可能已经被移除了，所以无法查询 parent。
        // 调用者需要在移除前通过 item_id 查到 parent 并保留，或者 handle_item_removed 接口应该接受 parent_id 参数。
        // 但为了接口简单，我们在 LineEditorState 中可能无法完美获取 parent。
        // 变通方法：通过 item_id_to_ui_index 找到行，读取 ancestor_groups 的最后一个元素作为 parent。
        let parent_id = self.find_parent_from_ui(item_id);

        let ui_index = match self.item_id_to_ui_index.get(&item_id) {
            Some(&idx) => idx,
            None => return,
        };

        // 获取要删除的行数
        let count = if let Some(row) = self.all_rows.row_data(ui_index) {
            if row.row_type == SlintSubtitleRowType::GroupHeader {
                row.total_row_count as usize
            } else {
                1
            }
        } else {
            1
        };

        // 先清理索引
        for i in 0..count {
            let idx = ui_index + i;
            if let Some(row) = self.all_rows.row_data(idx) {
                if let Ok(uuid) = uuid::Uuid::parse_str(row.item_id.as_str()) {
                    self.item_id_to_ui_index.remove(&ItemId(uuid));
                }
            }
        }

        for _ in 0..count {
            self.all_rows.remove(ui_index);
        }

        self.shift_indices_after(ui_index, -(count as i32));

        if let Some(pid) = parent_id {
            self.mark_group_chain_dirty(project, pid);
            self.update_all_dirty_groups(project);
        }
    }

    /// 处理项移动后的 UI 更新
    pub fn handle_item_moved(
        &mut self,
        project: &Project,
        item_id: ItemId,
        _old_location: ItemLocation,
        new_location: ItemLocation,
    ) {
        // 先删除（此时 handle_item_removed 会自动处理旧 parent 的更新）
        self.handle_item_removed(project, item_id);
        // 再插入（此时 handle_item_inserted 会自动处理新 parent 的更新）
        self.handle_item_inserted(project, item_id, new_location);
    }

    // === 行数与时间统计更新 ===

    /// 标记分组及其所有祖先为脏（行数/时间失效）
    fn mark_group_chain_dirty(&mut self, project: &Project, start_group_id: ItemId) {
        let mut current = Some(start_group_id);
        while let Some(gid) = current {
            if !self.dirty_row_counts.insert(gid) {
                // 已经标记过，通常意味着祖先也已经标记过，可以提前退出
                // 但为了保险（防止之前的标记不完整），继续向上
            }
            // 向上查找
            current = project.structure.get(gid).and_then(|item| item.parent);
        }
    }

    /// 更新所有脏分组的统计信息（从下往上，或递归）
    fn update_all_dirty_groups(&mut self, project: &Project) {
        // 复制一份 dirty set 以便遍历
        let dirty_ids: Vec<ItemId> = self.dirty_row_counts.iter().cloned().collect();

        for gid in dirty_ids {
            // 只有当它还在 dirty set 中时才处理（递归过程中可能已经处理并移除了子项）
            if self.dirty_row_counts.contains(&gid) {
                self.update_group_stats_recursive(project, gid);
            }
        }
    }

    /// 递归更新单个分组的统计信息（行数和时间）
    fn update_group_stats_recursive(&mut self, project: &Project, group_id: ItemId) {
        // 1. 获取子项列表 (clone 避免借用冲突)
        let children = match project.structure.get(group_id) {
            Some(item) => match &item.content {
                ItemContent::Group(g) => g.children.clone(),
                _ => return, // 不是 Group，无需更新
            },
            None => return,
        };

        let mut count = 1; // 包含 Group Header 自身
        let mut min_start: Option<TimeDelta> = None;
        let mut max_end: Option<TimeDelta> = None;

        for child_id in children {
            // 如果子项是 dirty 的 Group，先递归更新它
            if self.dirty_row_counts.contains(&child_id) {
                self.update_group_stats_recursive(project, child_id);
            }

            // 从 UI 或 Model 读取子项数据
            // 对于 Group，必须读 UI (因为那是聚合结果)
            // 对于 Event，可以直接读 Model (但也需要在 UI 中定位)
            // 统一通过 UI 索引查找
            if let Some(&idx) = self.item_id_to_ui_index.get(&child_id) {
                if let Some(row) = self.all_rows.row_data(idx) {
                    // --- 聚合行数 ---
                    let child_rows = if row.row_type == SlintSubtitleRowType::GroupHeader {
                        row.total_row_count as i32
                    } else {
                        1
                    };
                    count += child_rows;

                    // --- 聚合时间 ---
                    // 尝试获取子项的时间范围
                    let (c_start, c_end) = if row.row_type == SlintSubtitleRowType::Event {
                        // Event: 直接查 Model 更快且准确
                        project
                            .structure
                            .get(child_id)
                            .and_then(|item| match &item.content {
                                ItemContent::Event(e) => Some((e.start_time, e.end_time)),
                                _ => None,
                            })
                            .unwrap_or((TimeDelta::zero(), TimeDelta::zero())) // Fallback
                    } else {
                        // Group: 必须查 UI 的字符串并解析
                        let s = parse_time(&row.start_time);
                        let e = parse_time(&row.end_time);
                        match (s, e) {
                            (Some(s), Some(e)) => (s, e),
                            _ => continue, // 空 Group 或无效时间，跳过
                        }
                    };

                    min_start = Some(min_start.map_or(c_start, |v| v.min(c_start)));
                    max_end = Some(max_end.map_or(c_end, |v| v.max(c_end)));
                }
            }
        }

        // 更新本 Group 的 UI
        if let Some(&idx) = self.item_id_to_ui_index.get(&group_id) {
            if let Some(mut row) = self.all_rows.row_data(idx) {
                row.total_row_count = count;
                if let (Some(s), Some(e)) = (min_start, max_end) {
                    row.start_time = format_time(s).to_shared_string();
                    row.end_time = format_time(e).to_shared_string();
                } else {
                    // 变成空组了，或者子项都没有时间
                    row.start_time = SharedString::new();
                    row.end_time = SharedString::new();
                }
                self.all_rows.set_row_data(idx, row);
            }
        }

        // 标记已处理
        self.dirty_row_counts.remove(&group_id);
    }

    // === 辅助计算 ===

    fn find_parent_from_ui(&self, item_id: ItemId) -> Option<ItemId> {
        let idx = self.item_id_to_ui_index.get(&item_id)?;
        let row = self.all_rows.row_data(*idx)?;
        // ancestor_groups: ["grandparent_id", "parent_id"]
        // 取最后一个
        if row.ancestor_groups.row_count() > 0 {
            let last_ancestor = row
                .ancestor_groups
                .row_data(row.ancestor_groups.row_count() - 1)?;
            uuid::Uuid::parse_str(&last_ancestor).ok().map(ItemId)
        } else {
            None
        }
    }

    fn calculate_insert_ui_index(&self, project: &Project, location: ItemLocation) -> usize {
        match location {
            ItemLocation::Root(idx) => {
                if idx == 0 {
                    0
                } else {
                    let prev_id = project.structure.root_items[idx - 1];
                    self.get_item_ui_end_index(prev_id)
                }
            }
            ItemLocation::InGroup(group_id, idx) => {
                if idx == 0 {
                    self.item_id_to_ui_index
                        .get(&group_id)
                        .map(|&i| i + 1)
                        .unwrap_or(0)
                } else {
                    let group = match project.structure.get(group_id) {
                        Some(item) => match &item.content {
                            ItemContent::Group(g) => g,
                            _ => return 0,
                        },
                        None => return 0,
                    };
                    let prev_id = group.children[idx - 1];
                    self.get_item_ui_end_index(prev_id)
                }
            }
        }
    }

    fn get_item_ui_end_index(&self, item_id: ItemId) -> usize {
        if let Some(&start_idx) = self.item_id_to_ui_index.get(&item_id) {
            if let Some(row) = self.all_rows.row_data(start_idx) {
                let count = if row.row_type == SlintSubtitleRowType::GroupHeader {
                    row.total_row_count as usize
                } else {
                    1
                };
                return start_idx + count;
            }
        }
        0
    }

    fn calculate_depth(&self, location: ItemLocation) -> i32 {
        match location {
            ItemLocation::Root(_) => 0,
            ItemLocation::InGroup(group_id, _) => {
                if let Some(&idx) = self.item_id_to_ui_index.get(&group_id) {
                    if let Some(row) = self.all_rows.row_data(idx) {
                        return row.depth + 1;
                    }
                }
                0
            }
        }
    }

    fn get_ancestor_groups_for_location(&self, location: ItemLocation) -> Vec<SharedString> {
        match location {
            ItemLocation::Root(_) => Vec::new(),
            ItemLocation::InGroup(group_id, _) => {
                if let Some(&idx) = self.item_id_to_ui_index.get(&group_id) {
                    if let Some(row) = self.all_rows.row_data(idx) {
                        let mut ancestors: Vec<SharedString> = row.ancestor_groups.iter().collect();
                        ancestors.push(group_id.to_string().to_shared_string());
                        return ancestors;
                    }
                }
                Vec::new()
            }
        }
    }

    fn shift_indices_after(&mut self, threshold: usize, offset: i32) {
        for idx in self.item_id_to_ui_index.values_mut() {
            if *idx >= threshold {
                *idx = (*idx as i32 + offset) as usize;
            }
        }
    }
}

// Helpers
fn flatten_project_structure(structure: &ProjectStructure) -> Vec<SlintSubtitleRow> {
    let mut output = Vec::new();
    flatten_items(
        &structure.root_items,
        structure,
        0,
        &Vec::new(),
        &mut output,
    );
    output
}

fn flatten_items(
    items: &[ItemId],
    structure: &ProjectStructure,
    depth: i32,
    ancestor_groups: &[SharedString],
    output: &mut Vec<SlintSubtitleRow>,
) -> (Option<(TimeDelta, TimeDelta)>, i32) {
    let mut min_start: Option<TimeDelta> = None;
    let mut max_end: Option<TimeDelta> = None;
    let mut row_count = 0;

    for item_id in items {
        let item = match structure.get(*item_id) {
            Some(i) => i,
            None => continue,
        };

        match &item.content {
            ItemContent::Event(event) => {
                let start = event.start_time;
                let end = event.end_time;
                min_start = Some(min_start.map_or(start, |s| s.min(start)));
                max_end = Some(max_end.map_or(end, |e| e.max(end)));

                output.push(SlintSubtitleRow {
                    row_type: SlintSubtitleRowType::Event,
                    depth,
                    ancestor_groups: ModelRc::from(ancestor_groups),
                    item_id: item_id.to_string().to_shared_string(),
                    start_time: format_time(start).to_shared_string(),
                    end_time: format_time(end).to_shared_string(),
                    text: event.text.to_shared_string(),
                    style: event.style.to_shared_string(),
                    layer: 0,
                    is_comment: false,
                    group_name: SharedString::new(),
                    collapsed: false,
                    total_row_count: 0,
                });
                row_count += 1;
            }
            ItemContent::Group(group) => {
                let group_id_str = item_id.to_string().to_shared_string();
                let group_idx = output.len();

                output.push(SlintSubtitleRow {
                    row_type: SlintSubtitleRowType::GroupHeader,
                    depth,
                    ancestor_groups: ModelRc::from(ancestor_groups),
                    item_id: group_id_str.clone(),
                    start_time: SharedString::new(),
                    end_time: SharedString::new(),
                    text: SharedString::new(),
                    style: SharedString::new(),
                    layer: -1,
                    is_comment: false,
                    group_name: group.name.to_shared_string(),
                    collapsed: group.collapsed,
                    total_row_count: 0,
                });
                row_count += 1;

                let new_ancestors = [ancestor_groups, &[group_id_str].as_slice()].concat();
                let (child_range, child_count) = flatten_items(
                    &group.children,
                    structure,
                    depth + 1,
                    &new_ancestors,
                    output,
                );

                let total_count = 1 + child_count;
                row_count += child_count;

                if let Some((s, e)) = child_range {
                    min_start = Some(min_start.map_or(s, |ms| ms.min(s)));
                    max_end = Some(max_end.map_or(e, |me| me.max(e)));
                    output[group_idx].start_time = format_time(s);
                    output[group_idx].end_time = format_time(e);
                }
                output[group_idx].total_row_count = total_count;
            }
        }
    }

    let range = match (min_start, max_end) {
        (Some(s), Some(e)) => Some((s, e)),
        _ => None,
    };
    (range, row_count)
}

type TimeBoundsCache = HashMap<ItemId, Option<(TimeDelta, TimeDelta)>>;
fn find_item_time_bounds_recursive(
    item_id: ItemId,
    project: &Project,
    cache: &mut TimeBoundsCache,
) -> Option<(TimeDelta, TimeDelta)> {
    // 优先从缓存读取
    if let Some(cached_result) = cache.get(&item_id) {
        return *cached_result;
    }

    let item = match project.structure.get(item_id) {
        Some(i) => i,
        None => return None, // 项目不存在，没有时间范围
    };

    // 根据项目类型进行计算
    let result = match &item.content {
        ItemContent::Event(event) => Some((event.start_time, event.end_time)),

        ItemContent::Group(group) => {
            let mut min_start: Option<TimeDelta> = None;
            let mut max_end: Option<TimeDelta> = None;

            // 遍历分组的所有子项
            for child_id in &group.children {
                if let Some((child_start, child_end)) =
                    find_item_time_bounds_recursive(*child_id, project, cache)
                {
                    // 用子项的时间范围来更新整个分组的“最小开始时间”和“最大结束时间”
                    min_start = Some(min_start.map_or(child_start, |s| s.min(child_start)));
                    max_end = Some(max_end.map_or(child_end, |e| e.max(child_end)));
                }
            }

            // 将最终计算出的 min_start 和 max_end 组合成结果
            match (min_start, max_end) {
                (Some(s), Some(e)) => Some((s, e)),
                _ => None,
            }
        }
    };

    // 将本次计算结果存入缓存，以便下次对同一个 item_id 的请求能立刻返回
    cache.insert(item_id, result);
    result
}
