// TODO: 实现扁平化传递给 Slint 侧展示，并尝试互相操作数据结构

use std::{
    collections::{HashMap, HashSet},
    mem,
    rc::Rc,
    str::FromStr,
};

use anyhow::Result;
use aweft_ass::{
    ASSFile,
    types::{event::EventKey, time::AssTime},
};
use log::warn;
use slint::{FilterModel, Model, ModelRc, SharedString, ToSharedString, VecModel};
use uuid::Uuid;

use crate::bridge::{
    SlintKey, SlintSubtitleRow, parse_group_id, types::key_to_slint_key,
    update_group_row_count_with_ui,
};

/// 分组唯一标识符
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct EventGroupId(pub Uuid);

impl EventGroupId {
    pub fn new() -> Self {
        Self(Uuid::new_v4())
    }

    pub fn to_string(&self) -> String {
        self.0.to_string()
    }
}

#[derive(Clone, Copy, Debug)]
pub enum EventGroupItem {
    Event(EventKey),
    ChildGroup(EventGroupId),
}

/// 分组结构
#[derive(Debug, Clone)]
pub struct EventGroup {
    pub id: EventGroupId,
    pub name: String,
    pub parent: Option<EventGroupId>,
    pub items: Vec<EventGroupItem>,
    // 用于持久化，显示逻辑由 Slint 独自处理
    pub collapsed: bool,
    pub color: Option<String>,
}

// 用于缓存 Rust 侧数据结构位置，以便快速查找、移动与删除
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum EventGroupItemLocation {
    // 在根级别，索引位置
    Root(usize),
    // 在某个分组内，索引位置
    InGroup(EventGroupId, usize),
}

/// 分组管理器
#[derive(Debug, Clone)]
pub struct EventGroupManager {
    pub groups: HashMap<EventGroupId, EventGroup>,
    pub root_items: Vec<EventGroupItem>,
    pub event_location: HashMap<EventKey, EventGroupItemLocation>,
    pub group_location: HashMap<EventGroupId, EventGroupItemLocation>,
}

impl EventGroupManager {
    pub fn new() -> Self {
        Self {
            groups: HashMap::new(),
            root_items: Vec::new(),
            event_location: HashMap::new(),
            group_location: HashMap::new(),
        }
    }

    fn push_event_to_root(&mut self, event_key: EventKey) {
        let index = self.root_items.len();
        self.root_items.push(EventGroupItem::Event(event_key));
        self.event_location
            .insert(event_key, EventGroupItemLocation::Root(index));
    }

    fn insert_event_to_root(&mut self, event_key: EventKey, position: usize) {
        let pos = position.min(self.root_items.len());
        self.root_items
            .insert(pos, EventGroupItem::Event(event_key));
        self.event_location
            .insert(event_key, EventGroupItemLocation::Root(pos));

        self.update_root_indices_after(pos + 1);
    }

    fn push_event_to_group(&mut self, event_key: EventKey, group_id: EventGroupId) {
        let group = self.groups.get_mut(&group_id).unwrap();
        let index = group.items.len();
        group.items.push(EventGroupItem::Event(event_key));

        self.event_location
            .insert(event_key, EventGroupItemLocation::InGroup(group_id, index));
    }

    fn insert_event_to_group(
        &mut self,
        event_key: EventKey,
        group_id: EventGroupId,
        position: usize,
    ) {
        let group = self.groups.get_mut(&group_id).unwrap();
        let pos = position.min(group.items.len());
        group.items.insert(pos, EventGroupItem::Event(event_key));

        self.update_group_indices_after(group_id, pos + 1);
    }

    fn move_event(&mut self, event_key: EventKey, target: EventGroupItemLocation) {
        self.remove_event(event_key);

        match target {
            EventGroupItemLocation::Root(index) => self.insert_event_to_root(event_key, index),
            EventGroupItemLocation::InGroup(group_id, index) => {
                self.insert_event_to_group(event_key, group_id, index)
            }
        }
    }

    fn push_group_to_root(&mut self, name: String) -> EventGroupId {
        let group_id = EventGroupId::new();
        let group = EventGroup {
            id: group_id,
            name,
            parent: None,
            items: Vec::new(),
            collapsed: false,
            color: None,
        };

        self.groups.insert(group_id, group);

        let index = self.root_items.len();
        self.root_items.push(EventGroupItem::ChildGroup(group_id));
        self.group_location
            .insert(group_id, EventGroupItemLocation::Root(index));

        group_id
    }

    fn insert_group_to_root(&mut self, name: String, position: usize) -> EventGroupId {
        let group_id = EventGroupId::new();
        let group = EventGroup {
            id: group_id,
            name,
            parent: None,
            items: Vec::new(),
            collapsed: false,
            color: None,
        };

        self.groups.insert(group_id, group);

        let pos = position.min(self.root_items.len());
        self.root_items
            .insert(pos, EventGroupItem::ChildGroup(group_id));
        self.group_location
            .insert(group_id, EventGroupItemLocation::Root(pos));

        self.update_root_indices_after(pos + 1);

        group_id
    }

    fn push_group_to_group(&mut self, name: String, parent_id: EventGroupId) -> EventGroupId {
        let group_id = EventGroupId::new();
        let group = EventGroup {
            id: group_id,
            name,
            parent: Some(parent_id),
            items: Vec::new(),
            collapsed: false,
            color: None,
        };

        self.groups.insert(group_id, group);

        let parent = self.groups.get_mut(&parent_id).unwrap();
        let index = parent.items.len();
        parent.items.push(EventGroupItem::ChildGroup(group_id));
        self.group_location
            .insert(group_id, EventGroupItemLocation::InGroup(parent_id, index));

        group_id
    }

    fn insert_group_to_group(
        &mut self,
        name: String,
        parent_id: EventGroupId,
        position: usize,
    ) -> EventGroupId {
        let group_id = EventGroupId::new();
        let group = EventGroup {
            id: group_id,
            name,
            parent: Some(parent_id),
            items: Vec::new(),
            collapsed: false,
            color: None,
        };

        self.groups.insert(group_id, group);

        let parent = self.groups.get_mut(&parent_id).unwrap();
        let pos = position.min(parent.items.len());
        parent
            .items
            .insert(pos, EventGroupItem::ChildGroup(group_id));
        self.group_location
            .insert(group_id, EventGroupItemLocation::InGroup(parent_id, pos));

        self.update_group_indices_after(parent_id, pos + 1);

        group_id
    }

    fn move_group(&mut self, group_id: EventGroupId, target: EventGroupItemLocation) -> Result<()> {
        // 避免父级移到子级
        if let EventGroupItemLocation::InGroup(target_group, _) = target {
            if self.is_ancestor(group_id, target_group) {
                return Err(anyhow::anyhow!("Cannot move group into its descendant"));
            }
        }

        self.remove_group(group_id);

        match target {
            EventGroupItemLocation::Root(index) => {
                let pos = index.min(self.root_items.len());
                self.root_items
                    .insert(pos, EventGroupItem::ChildGroup(group_id));
                self.group_location
                    .insert(group_id, EventGroupItemLocation::Root(pos));
                self.groups.get_mut(&group_id).unwrap().parent = None;

                self.update_root_indices_after(pos + 1);
            }
            EventGroupItemLocation::InGroup(parent_id, index) => {
                let parent = self.groups.get_mut(&parent_id).unwrap();
                let pos = index.min(parent.items.len());
                parent
                    .items
                    .insert(pos, EventGroupItem::ChildGroup(group_id));
                self.group_location
                    .insert(group_id, EventGroupItemLocation::InGroup(parent_id, pos));
                self.groups.get_mut(&group_id).unwrap().parent = Some(parent_id);

                self.update_group_indices_after(parent_id, pos + 1);
            }
        }

        Ok(())
    }

    fn remove_event(&mut self, event_key: EventKey) {
        let location = self.event_location.remove(&event_key).unwrap();
        match location {
            EventGroupItemLocation::Root(index) => {
                self.root_items.remove(index);
                self.update_root_indices_after(index);
            }
            EventGroupItemLocation::InGroup(group_id, index) => {
                let group = self.groups.get_mut(&group_id).unwrap();
                group.items.remove(index);
                self.update_group_indices_after(group_id, index);
            }
        }
    }

    fn ungroup(&mut self, group_id: EventGroupId) -> Result<()> {
        let location = self.group_location.get(&group_id).unwrap();

        let items_to_move = mem::take(&mut self.groups.get_mut(&group_id).unwrap().items);

        // 插入的索引位置
        let insert_location = match location {
            EventGroupItemLocation::Root(index) => EventGroupItemLocation::Root(*index),
            EventGroupItemLocation::InGroup(parent_id, index) => {
                EventGroupItemLocation::InGroup(*parent_id, *index)
            }
        };

        // 从后往前插入，保持顺序
        for item in items_to_move.iter().rev() {
            match item {
                EventGroupItem::Event(event_key) => {
                    self.move_event(*event_key, insert_location);
                }
                EventGroupItem::ChildGroup(child_id) => {
                    self.move_group(*child_id, insert_location)?;
                }
            }
        }

        // 移除空组
        self.remove_group(group_id);
        self.groups.remove(&group_id);

        Ok(())
    }

    fn remove_group(&mut self, group_id: EventGroupId) {
        let location = self.group_location.remove(&group_id).unwrap();

        match location {
            EventGroupItemLocation::Root(index) => {
                self.root_items.remove(index);
                self.update_root_indices_after(index);
            }
            EventGroupItemLocation::InGroup(group_id, index) => {
                let group = self.groups.get_mut(&group_id).unwrap();
                group.items.remove(index);
                self.update_group_indices_after(group_id, index);
            }
        }
    }

    fn update_root_indices_after(&mut self, start: usize) {
        for (i, item) in self.root_items.iter().enumerate().skip(start) {
            match item {
                EventGroupItem::Event(key) => self
                    .event_location
                    .insert(*key, EventGroupItemLocation::Root(i)),
                EventGroupItem::ChildGroup(id) => self
                    .group_location
                    .insert(*id, EventGroupItemLocation::Root(i)),
            };
        }
    }
    fn update_group_indices_after(&mut self, group_id: EventGroupId, start: usize) {
        let group = self.groups.get(&group_id).unwrap();
        for (i, item) in group.items.iter().enumerate().skip(start) {
            match item {
                EventGroupItem::Event(key) => self
                    .event_location
                    .insert(*key, EventGroupItemLocation::InGroup(group_id, i)),
                EventGroupItem::ChildGroup(id) => self
                    .group_location
                    .insert(*id, EventGroupItemLocation::InGroup(group_id, i)),
            };
        }
    }

    // 检查是否是父级
    fn is_ancestor(&self, potential_ancestor: EventGroupId, descendant: EventGroupId) -> bool {
        let mut current = descendant;
        while let Some(group) = self.groups.get(&current) {
            if let Some(parent_id) = group.parent {
                if parent_id == potential_ancestor {
                    return true;
                }
                current = parent_id;
            } else {
                break;
            }
        }
        false
    }
}

type SubtitleFilterModel =
    slint::FilterModel<ModelRc<SlintSubtitleRow>, Box<dyn Fn(&SlintSubtitleRow) -> bool>>;

/// 行编辑器的状态管理
///
/// 使用 FilterModel 实现高效的分组折叠/展开功能：
/// - `all_rows`: 所有字幕行的完整列表（VecModel）
/// - `filtered_rows`: 经过过滤的可见行列表（FilterModel）
/// - `collapsed_groups`: 被折叠的分组ID集合，供FilterModel的过滤闭包使用
pub struct LineEditorState {
    // 分组管理器
    pub group_manager: EventGroupManager,

    // ASS文件数据
    // FIXME: 应当使用样式节点链的高层抽象
    pub ass_file: ASSFile,

    // UI 数据源（扁平化的完整列表）
    // 分组行数与组时间在当中查找
    all_rows: Rc<VecModel<crate::bridge::types::SlintSubtitleRow>>,
    filter_model: Option<Rc<SubtitleFilterModel>>,

    // 运行时折叠状态集合（共享引用，供 FilterModel 的闭包使用。索引到实际数据）
    collapsed_groups: Rc<std::cell::RefCell<std::collections::HashSet<EventGroupId>>>,

    // row_counts 已经失效了的 group
    dirty_row_counts: HashSet<EventGroupId>,

    /// EventKey 到 UI 行索引的映射（用于快速定位）
    event_key_to_ui_index: HashMap<EventKey, usize>,
    group_id_to_ui_index: HashMap<EventGroupId, usize>,
}

impl LineEditorState {
    pub fn new() -> Self {
        Self {
            group_manager: EventGroupManager::new(),
            // FIXME: 应当使用样式节点链的高层抽象
            ass_file: ASSFile::new(),
            all_rows: Rc::new(VecModel::default()),
            filter_model: None,
            collapsed_groups: Rc::new(std::cell::RefCell::new(std::collections::HashSet::new())),
            dirty_row_counts: HashSet::new(),
            event_key_to_ui_index: HashMap::new(),
            group_id_to_ui_index: HashMap::new(),
        }
    }

    /// 创建过滤模型
    ///
    /// 使用 Slint 的 FilterModel 包装数据源，根据折叠状态自动过滤可见行。
    /// 过滤规则：
    /// - 根级别的行（无祖先）始终可见
    /// - 有祖先的行（包括子分组头和事件）只有在所有祖先都未折叠时才可见
    pub fn init_filter_model(&mut self) -> ModelRc<SlintSubtitleRow> {
        use crate::bridge::parse_group_id;

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
                for ancestor_id in row.ancestor_groups.iter() {
                    let Ok(group_id) = parse_group_id(&ancestor_id) else {
                        warn!("row 数据当中存在无效 group_id: {}", ancestor_id);
                        continue;
                    };

                    if collapsed.contains(&group_id) {
                        return false;
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
    pub fn rebuild_all_rows(&mut self) {
        use crate::bridge::types::flatten_subtitle_rows;

        let rows = flatten_subtitle_rows(
            &self.group_manager.root_items,
            &self.group_manager.groups,
            &self.ass_file,
        );

        // 构建索引映射，用于快速定位
        self.event_key_to_ui_index.clear();
        self.group_id_to_ui_index.clear();

        for (i, row) in rows.iter().enumerate() {
            match row.row_type {
                crate::bridge::types::SlintSubtitleRowType::Event => {
                    let event_key = crate::bridge::types::slint_area_key_to_key(&row.event_key);
                    self.event_key_to_ui_index.insert(event_key, i);
                }
                crate::bridge::types::SlintSubtitleRowType::GroupHeader => {
                    if let Ok(group_id) = crate::bridge::parse_group_id(&row.group_id_str) {
                        self.group_id_to_ui_index.insert(group_id, i);
                    }
                }
            }
        }

        // 更新数据源，这会自动触发 FilterModel 重新过滤
        // 仅限初始化使用，否则使用局部更新函数
        self.all_rows.set_vec(rows);

        self.dirty_row_counts.clear();
    }

    /// 触发重新过滤
    ///
    /// 在折叠/展开分组时调用此方法。步骤：
    /// 1. 同步 collapsed_groups 和 group_manager.groups 的折叠状态
    /// 2. 重建所有行以更新 GroupHeader 的 collapsed 字段
    /// 3. VecModel 的变化自动触发 FilterModel 重新过滤
    pub fn toggle_group(&mut self, group_id: EventGroupId) {
        // 同步折叠状态到共享集合
        let is_now_collapsed = {
            let mut collapsed = self.collapsed_groups.borrow_mut();
            if collapsed.contains(&group_id) {
                collapsed.remove(&group_id);
                // 现在是展开状态了
                false
            } else {
                collapsed.insert(group_id);
                // 现在是折叠状态了
                true
            }
        };

        // 通知过滤更新
        if let Some(model) = &self.filter_model {
            model.reset();
        }

        // 更新图标状态
        if let Some(&idx) = self.group_id_to_ui_index.get(&group_id) {
            if let Some(mut row) = self.all_rows.row_data(idx) {
                row.collapsed = is_now_collapsed;
                self.all_rows.set_row_data(idx, row);
            }
        }
    }

    /// 获取过滤后的模型
    pub fn filter_model(&self) -> Option<Rc<SubtitleFilterModel>> {
        self.filter_model.clone()
    }

    pub fn group_time_range(&self, group_id: EventGroupId) -> Option<(AssTime, AssTime)> {
        let idx = self.group_id_to_ui_index.get(&group_id)?;
        let row = self.all_rows.row_data(*idx)?;

        let start = AssTime::from_str(&row.start_time).ok()?;
        let end = AssTime::from_str(&row.end_time).ok()?;
        Some((start, end))
    }

    pub fn group_row_counts(&mut self, group_id: EventGroupId) -> Option<i32> {
        if self.dirty_row_counts.contains(&group_id) {
            let mut cache = HashMap::new();
            return Some(update_group_row_count_with_ui(
                group_id,
                &self.group_manager.groups,
                &self.all_rows,
                &self.group_id_to_ui_index,
                &mut self.dirty_row_counts,
                &mut cache,
            ));
        }

        let idx = self.group_id_to_ui_index.get(&group_id)?;
        let row = self.all_rows.row_data(*idx)?;
        Some(row.total_row_count)
    }

    // ====================================================================
    // TODO: 以下为AI生成，暂未详细审查

    // 获取EventKey的UI索引
    pub fn get_event_ui_index(&self, event_key: EventKey) -> Option<usize> {
        self.event_key_to_ui_index.get(&event_key).copied()
    }

    // 获取GroupId的UI索引
    pub fn get_group_ui_index(&self, group_id: EventGroupId) -> Option<usize> {
        self.group_id_to_ui_index.get(&group_id).copied()
    }

    // === 查询方法 ===

    /// 获取分组的行数（惰性求值）
    pub fn get_group_row_count(&mut self, group_id: EventGroupId) -> Option<i32> {
        self.ensure_group_row_count_valid(group_id);

        let idx = self.group_id_to_ui_index.get(&group_id)?;
        let row = self.all_rows.row_data(*idx)?;
        Some(row.total_row_count)
    }

    // === 辅助方法 ===

    /// 从 UI 读取分组的 ancestor_groups（用于构建子项的祖先列表）
    fn get_group_ancestor_groups(&self, group_id: EventGroupId) -> ModelRc<SharedString> {
        if let Some(&idx) = self.group_id_to_ui_index.get(&group_id) {
            if let Some(row) = self.all_rows.row_data(idx) {
                // 构建新的祖先列表：父的祖先 + 父自己
                let mut ancestors: Vec<SharedString> = row.ancestor_groups.iter().collect();
                ancestors.push(row.group_id_str.clone());
                return ModelRc::from(ancestors.as_slice());
            }
        }
        ModelRc::from(&[] as &[SharedString])
    }

    /// 确保分组的行数有效（惰性更新）
    fn ensure_group_row_count_valid(&mut self, group_id: EventGroupId) {
        if !self.dirty_row_counts.contains(&group_id) {
            return;
        }

        // 从 UI 读取祖先链，只更新 dirty 的分组
        let dirty_chain = self.collect_dirty_ancestors(group_id);

        // 从最深的开始更新（确保子分组数据准确）
        for &gid in dirty_chain.iter().rev() {
            self.update_single_group_row_count(gid);
            self.dirty_row_counts.remove(&gid);
        }
    }

    /// 收集祖先链中的所有 dirty 分组（顺序：根 → 当前）
    fn collect_dirty_ancestors(&self, group_id: EventGroupId) -> Vec<EventGroupId> {
        let mut chain = Vec::new();
        let mut current = Some(group_id);

        while let Some(gid) = current {
            if self.dirty_row_counts.contains(&gid) {
                chain.push(gid);
            }
            current = self.group_manager.groups.get(&gid).and_then(|g| g.parent);
        }

        chain.reverse();
        chain
    }

    /// 更新单个分组的行数（递归确保子分组准确）
    fn update_single_group_row_count(&mut self, group_id: EventGroupId) {
        // 先克隆 items，避免借用冲突
        let items = match self.group_manager.groups.get(&group_id) {
            Some(g) => g.items.clone(),
            None => return,
        };

        let mut count = 1; // 分组头
        for item in &items {
            count += match item {
                EventGroupItem::Event(_) => 1,
                EventGroupItem::ChildGroup(child_id) => {
                    // 如果子分组是 dirty 的，先递归更新它
                    if self.dirty_row_counts.contains(child_id) {
                        self.update_single_group_row_count(*child_id);
                        self.dirty_row_counts.remove(child_id);
                    }

                    // 从 UI 读取子分组的行数
                    self.group_id_to_ui_index.get(child_id)
                        .and_then(|&i| self.all_rows.row_data(i))
                        .map(|r| r.total_row_count)
                        .expect("update_single_group_row_count: child group must exist in UI index and all_rows")
                }
            };
        }

        // 更新 UI
        if let Some(&idx) = self.group_id_to_ui_index.get(&group_id) {
            if let Some(mut row) = self.all_rows.row_data(idx) {
                row.total_row_count = count;
                self.all_rows.set_row_data(idx, row);
            }
        }
    }

    /// 标记分组及其祖先的行数失效
    fn mark_group_row_count_dirty(&mut self, group_id: EventGroupId) {
        self.dirty_row_counts.insert(group_id);

        // 从 UI 读取祖先列表并标记
        if let Some(&idx) = self.group_id_to_ui_index.get(&group_id) {
            if let Some(row) = self.all_rows.row_data(idx) {
                for ancestor_id in row.ancestor_groups.iter() {
                    if let Ok(ancestor_gid) = parse_group_id(&ancestor_id) {
                        self.dirty_row_counts.insert(ancestor_gid);
                    }
                }
            }
        }
    }

    /// 向上传播时间变化到祖先分组
    fn propagate_time_to_ancestors(
        &mut self,
        group_id: EventGroupId,
        start: AssTime,
        end: AssTime,
    ) {
        if let Some(&idx) = self.group_id_to_ui_index.get(&group_id) {
            if let Some(row) = self.all_rows.row_data(idx) {
                for ancestor_id in row.ancestor_groups.iter() {
                    if let Ok(ancestor_gid) = parse_group_id(&ancestor_id) {
                        self.expand_group_time_range(ancestor_gid, start, end);
                    }
                }
            }
        }
    }

    /// 扩大分组的时间范围（只扩大，不缩小）
    fn expand_group_time_range(
        &mut self,
        group_id: EventGroupId,
        new_start: AssTime,
        new_end: AssTime,
    ) {
        let idx = match self.group_id_to_ui_index.get(&group_id) {
            Some(&i) => i,
            None => return,
        };

        let mut row = match self.all_rows.row_data(idx) {
            Some(r) => r,
            None => return,
        };

        let current_start = AssTime::from_str(&row.start_time).unwrap_or(new_start);
        let current_end = AssTime::from_str(&row.end_time).unwrap_or(new_end);

        let updated_start = current_start.min(new_start);
        let updated_end = current_end.max(new_end);

        if updated_start != current_start || updated_end != current_end {
            row.start_time = updated_start.to_shared_string();
            row.end_time = updated_end.to_shared_string();
            self.all_rows.set_row_data(idx, row);
        }
    }

    /// 计算 root_items[index] 之后的 UI 位置
    fn calculate_ui_position_after_root_item(&self, index: usize) -> usize {
        match &self.group_manager.root_items[index] {
            EventGroupItem::Event(key) => {
                // 事件占1行，返回事件位置+1
                self.event_key_to_ui_index
                    .get(key)
                    .map(|&i| i + 1)
                    .expect("calculate_ui_position_after_root_item: event must exist in UI index")
            }
            EventGroupItem::ChildGroup(group_id) => {
                // 分组占 total_row_count 行
                self.group_id_to_ui_index.get(group_id)
                    .and_then(|&i| {
                        self.all_rows.row_data(i).map(|row| i + row.total_row_count as usize)
                    })
                    .expect("calculate_ui_position_after_root_item: group must exist in UI index and all_rows")
            }
        }
    }

    /// 计算分组内 items[index] 之后的 UI 位置
    fn calculate_ui_position_after_group_item(
        &self,
        group_id: EventGroupId,
        index: usize,
    ) -> usize {
        let group = match self.group_manager.groups.get(&group_id) {
            Some(g) => g,
            None => panic!(
                "calculate_ui_position_after_group_item: group {:?} must exist",
                group_id
            ),
        };

        match &group.items[index] {
            EventGroupItem::Event(key) => {
                self.event_key_to_ui_index.get(key)
                    .map(|&i| i + 1)
                    .expect("calculate_ui_position_after_group_item: event must exist in UI index")
            }
            EventGroupItem::ChildGroup(child_id) => {
                self.group_id_to_ui_index.get(child_id)
                    .and_then(|&i| {
                        self.all_rows.row_data(i).map(|row| i + row.total_row_count as usize)
                    })
                    .expect("calculate_ui_position_after_group_item: child group must exist in UI index and all_rows")
            }
        }
    }

    // === 包装EventGroupManager的操作，自动标记失效 ===

    pub fn push_event_to_root(&mut self, event_key: EventKey) {
        if let Some(event) = self.ass_file.events.get(event_key) {
            self.group_manager.push_event_to_root(event_key);

            self.event_key_to_ui_index
                .insert(event_key, self.all_rows.row_count());

            self.all_rows.push(SlintSubtitleRow {
                row_type: crate::bridge::SlintSubtitleRowType::Event,
                depth: 0,
                ancestor_groups: ModelRc::from(&[] as &[SharedString]),
                start_time: event.start().to_shared_string(),
                end_time: event.end().to_shared_string(),
                event_key: key_to_slint_key(event_key),
                text: event.text().to_shared_string(),
                style: event.style().to_shared_string(),
                layer: event.layer(),
                is_comment: matches!(event, aweft_ass::types::event::Event::Comment(_)),
                group_id_str: SharedString::new(),
                group_name: SharedString::new(),
                collapsed: false,
                total_row_count: 0,
            });
        }
    }

    pub fn insert_event_to_root(&mut self, event_key: EventKey, position: usize) {
        // 先提取事件数据，避免借用冲突
        let (start_time, end_time, text, style, layer, is_comment) =
            if let Some(event) = self.ass_file.events.get(event_key) {
                (
                    event.start().to_shared_string(),
                    event.end().to_shared_string(),
                    event.text().to_shared_string(),
                    event.style().to_shared_string(),
                    event.layer(),
                    matches!(event, aweft_ass::types::event::Event::Comment(_)),
                )
            } else {
                return;
            };

        let pos = position.min(self.group_manager.root_items.len());
        self.group_manager.insert_event_to_root(event_key, position);

        // 计算在 all_rows 中的实际插入位置
        let insert_ui_pos = if pos == 0 {
            0
        } else {
            // 找到前一个 root_item 的 UI 位置，然后+1或+total_row_count
            self.calculate_ui_position_after_root_item(pos - 1)
        };

        // 先更新后续索引
        self.shift_indices_after(insert_ui_pos, 1);

        // 插入新行和索引
        self.event_key_to_ui_index.insert(event_key, insert_ui_pos);
        self.all_rows.insert(
            insert_ui_pos,
            SlintSubtitleRow {
                row_type: crate::bridge::SlintSubtitleRowType::Event,
                depth: 0,
                ancestor_groups: ModelRc::from(&[] as &[SharedString]),
                start_time,
                end_time,
                event_key: key_to_slint_key(event_key),
                text,
                style,
                layer,
                is_comment,
                group_id_str: SharedString::new(),
                group_name: SharedString::new(),
                collapsed: false,
                total_row_count: 0,
            },
        );
    }

    pub fn push_event_to_group(&mut self, event_key: EventKey, group_id: EventGroupId) {
        // 先提取事件数据，避免借用冲突
        let (start_time, end_time, text, style, layer, is_comment, event_start, event_end) =
            if let Some(event) = self.ass_file.events.get(event_key) {
                (
                    event.start().to_shared_string(),
                    event.end().to_shared_string(),
                    event.text().to_shared_string(),
                    event.style().to_shared_string(),
                    event.layer(),
                    matches!(event, aweft_ass::types::event::Event::Comment(_)),
                    event.start(),
                    event.end(),
                )
            } else {
                return;
            };

        self.group_manager.push_event_to_group(event_key, group_id);

        // 标记行数失效
        self.mark_group_row_count_dirty(group_id);

        // 获取父分组的 UI 位置和元数据
        let group_ui_idx = match self.group_id_to_ui_index.get(&group_id) {
            Some(&idx) => idx,
            None => {
                warn!(
                    "push_event_to_group: group {:?} not found in UI index, falling back to rebuild",
                    group_id
                );
                self.rebuild_all_rows();
                return;
            }
        };

        // 计算插入位置：分组的最后一个元素之后
        // 使用 total_row_count 直接计算
        let group_row_count = self
            .all_rows
            .row_data(group_ui_idx)
            .map(|r| r.total_row_count as usize)
            .unwrap_or(1);
        let insert_pos = group_ui_idx + group_row_count;

        let depth = self
            .all_rows
            .row_data(group_ui_idx)
            .map(|r| r.depth + 1)
            .unwrap_or(1);
        let ancestor_groups = self.get_group_ancestor_groups(group_id);

        // 先更新后续索引
        self.shift_indices_after(insert_pos, 1);

        // 插入新行和索引
        self.event_key_to_ui_index.insert(event_key, insert_pos);
        self.all_rows.insert(
            insert_pos,
            SlintSubtitleRow {
                row_type: crate::bridge::SlintSubtitleRowType::Event,
                depth,
                ancestor_groups,
                start_time,
                end_time,
                event_key: key_to_slint_key(event_key),
                text,
                style,
                layer,
                is_comment,
                group_id_str: SharedString::new(),
                group_name: SharedString::new(),
                collapsed: false,
                total_row_count: 0,
            },
        );

        // 传播时间变化
        self.expand_group_time_range(group_id, event_start, event_end);
        self.propagate_time_to_ancestors(group_id, event_start, event_end);
    }

    pub fn insert_event_to_group(
        &mut self,
        event_key: EventKey,
        group_id: EventGroupId,
        position: usize,
    ) {
        // 先提取事件数据和分组信息，避免借用冲突
        let (start_time, end_time, text, style, layer, is_comment, event_start, event_end) =
            if let Some(event) = self.ass_file.events.get(event_key) {
                (
                    event.start().to_shared_string(),
                    event.end().to_shared_string(),
                    event.text().to_shared_string(),
                    event.style().to_shared_string(),
                    event.layer(),
                    matches!(event, aweft_ass::types::event::Event::Comment(_)),
                    event.start(),
                    event.end(),
                )
            } else {
                return;
            };

        let items_len = match self.group_manager.groups.get(&group_id) {
            Some(g) => g.items.len(),
            None => return,
        };

        let pos = position.min(items_len);
        self.group_manager
            .insert_event_to_group(event_key, group_id, position);
        self.mark_group_row_count_dirty(group_id);

        // 计算在 all_rows 中的实际插入位置
        let group_ui_idx = match self.group_id_to_ui_index.get(&group_id) {
            Some(&idx) => idx,
            None => {
                // 如果找不到分组UI索引，回退到重建
                self.rebuild_all_rows();
                return;
            }
        };

        let insert_ui_pos = if pos == 0 {
            // 插入在分组头后面
            group_ui_idx + 1
        } else {
            // 插入在第 pos-1 个元素之后
            self.calculate_ui_position_after_group_item(group_id, pos - 1)
        };

        let depth = self
            .all_rows
            .row_data(group_ui_idx)
            .map(|r| r.depth + 1)
            .unwrap_or(1);
        let ancestor_groups = self.get_group_ancestor_groups(group_id);

        // 先更新后续索引
        self.shift_indices_after(insert_ui_pos, 1);

        // 插入新行和索引
        self.event_key_to_ui_index.insert(event_key, insert_ui_pos);
        self.all_rows.insert(
            insert_ui_pos,
            SlintSubtitleRow {
                row_type: crate::bridge::SlintSubtitleRowType::Event,
                depth,
                ancestor_groups,
                start_time,
                end_time,
                event_key: key_to_slint_key(event_key),
                text,
                style,
                layer,
                is_comment,
                group_id_str: SharedString::new(),
                group_name: SharedString::new(),
                collapsed: false,
                total_row_count: 0,
            },
        );

        // 传播时间变化
        self.expand_group_time_range(group_id, event_start, event_end);
        self.propagate_time_to_ancestors(group_id, event_start, event_end);
    }

    pub fn move_event(&mut self, event_key: EventKey, target: EventGroupItemLocation) {
        let old_location = self.group_manager.event_location.get(&event_key).copied();

        // 获取当前UI位置
        let old_ui_pos = match self.event_key_to_ui_index.get(&event_key).copied() {
            Some(pos) => pos,
            None => {
                // 找不到，回退到完整移动
                self.group_manager.move_event(event_key, target);
                self.rebuild_all_rows();
                return;
            }
        };

        // 计算目标UI位置
        let target_ui_pos = match target {
            EventGroupItemLocation::Root(pos) => {
                if pos == 0 {
                    0
                } else {
                    self.calculate_ui_position_after_root_item(pos - 1)
                }
            }
            EventGroupItemLocation::InGroup(group_id, pos) => {
                let group_ui_idx = match self.group_id_to_ui_index.get(&group_id) {
                    Some(&idx) => idx,
                    None => {
                        self.group_manager.move_event(event_key, target);
                        self.rebuild_all_rows();
                        return;
                    }
                };

                if pos == 0 {
                    group_ui_idx + 1
                } else {
                    self.calculate_ui_position_after_group_item(group_id, pos - 1)
                }
            }
        };

        // 获取目标位置的元数据（depth 和 ancestor_groups）
        let (new_depth, new_ancestor_groups) = match target {
            EventGroupItemLocation::Root(_) => (0, ModelRc::from(&[] as &[SharedString])),
            EventGroupItemLocation::InGroup(group_id, _) => {
                let group_idx = self.group_id_to_ui_index.get(&group_id).unwrap();
                let depth = self
                    .all_rows
                    .row_data(*group_idx)
                    .map(|r| r.depth + 1)
                    .unwrap_or(1);
                let ancestor_groups = self.get_group_ancestor_groups(group_id);
                (depth, ancestor_groups)
            }
        };

        // 更新 group_manager
        self.group_manager.move_event(event_key, target);

        // 标记相关分组的 row_count 失效
        if let Some(EventGroupItemLocation::InGroup(old_parent, _)) = old_location {
            self.mark_group_row_count_dirty(old_parent);
        }
        if let EventGroupItemLocation::InGroup(new_parent, _) = target {
            self.mark_group_row_count_dirty(new_parent);
        }

        // 如果位置相同，只需要更新元数据（跨层级移动到同一位置的情况）
        if old_ui_pos == target_ui_pos {
            if let Some(mut row) = self.all_rows.row_data(old_ui_pos) {
                row.depth = new_depth;
                row.ancestor_groups = new_ancestor_groups;
                self.all_rows.set_row_data(old_ui_pos, row);
            }
            return;
        }

        // 移动UI行并更新元数据
        let mut row = self.all_rows.row_data(old_ui_pos).unwrap();
        row.depth = new_depth;
        row.ancestor_groups = new_ancestor_groups;

        self.all_rows.remove(old_ui_pos);

        // 调整插入位置（如果目标在删除位置之后）
        let adjusted_target = if target_ui_pos > old_ui_pos {
            target_ui_pos - 1
        } else {
            target_ui_pos
        };

        self.all_rows.insert(adjusted_target, row);

        // 更新索引：先删除后插入之间的范围
        if old_ui_pos < adjusted_target {
            // 向后移动：[old_ui_pos+1, adjusted_target] 的索引 -1
            for idx in self.event_key_to_ui_index.values_mut() {
                if *idx > old_ui_pos && *idx <= adjusted_target {
                    *idx -= 1;
                }
            }
            for idx in self.group_id_to_ui_index.values_mut() {
                if *idx > old_ui_pos && *idx <= adjusted_target {
                    *idx -= 1;
                }
            }
        } else {
            // 向前移动：[adjusted_target, old_ui_pos-1] 的索引 +1
            for idx in self.event_key_to_ui_index.values_mut() {
                if *idx >= adjusted_target && *idx < old_ui_pos {
                    *idx += 1;
                }
            }
            for idx in self.group_id_to_ui_index.values_mut() {
                if *idx >= adjusted_target && *idx < old_ui_pos {
                    *idx += 1;
                }
            }
        }

        // 更新移动元素的索引
        self.event_key_to_ui_index
            .insert(event_key, adjusted_target);
    }

    pub fn push_group_to_root(&mut self, name: String) -> EventGroupId {
        let id = self.group_manager.push_group_to_root(name.clone());

        self.group_id_to_ui_index
            .insert(id, self.all_rows.row_count());
        self.all_rows.push(SlintSubtitleRow {
            row_type: crate::bridge::SlintSubtitleRowType::GroupHeader,
            depth: 0,
            ancestor_groups: ModelRc::from(&[] as &[SharedString]),
            start_time: SharedString::new(),
            end_time: SharedString::new(),
            event_key: SlintKey {
                key_index: 0,
                key_version: 0,
            },
            text: SharedString::new(),
            style: SharedString::new(),
            layer: -1,
            is_comment: false,
            group_id_str: id.to_string().to_shared_string(),
            group_name: name.to_shared_string(),
            collapsed: false,
            total_row_count: 1,
        });
        id
    }

    pub fn insert_group_to_root(&mut self, name: String, position: usize) -> EventGroupId {
        let pos = position.min(self.group_manager.root_items.len());
        let id = self
            .group_manager
            .insert_group_to_root(name.clone(), position);

        // 计算在 all_rows 中的实际插入位置
        let insert_ui_pos = if pos == 0 {
            0
        } else {
            self.calculate_ui_position_after_root_item(pos - 1)
        };

        // 先更新后续索引
        self.shift_indices_after(insert_ui_pos, 1);

        // 插入新行和索引
        self.group_id_to_ui_index.insert(id, insert_ui_pos);
        self.all_rows.insert(
            insert_ui_pos,
            SlintSubtitleRow {
                row_type: crate::bridge::SlintSubtitleRowType::GroupHeader,
                depth: 0,
                ancestor_groups: ModelRc::from(&[] as &[SharedString]),
                start_time: SharedString::new(),
                end_time: SharedString::new(),
                event_key: SlintKey {
                    key_index: 0,
                    key_version: 0,
                },
                text: SharedString::new(),
                style: SharedString::new(),
                layer: -1,
                is_comment: false,
                group_id_str: id.to_string().to_shared_string(),
                group_name: name.to_shared_string(),
                collapsed: false,
                total_row_count: 1,
            },
        );

        id
    }

    pub fn push_group_to_group(&mut self, name: String, parent_id: EventGroupId) -> EventGroupId {
        let id = self
            .group_manager
            .push_group_to_group(name.clone(), parent_id);
        self.mark_group_row_count_dirty(parent_id);

        // 获取父分组的 UI 位置
        let parent_ui_idx = match self.group_id_to_ui_index.get(&parent_id) {
            Some(&idx) => idx,
            None => {
                warn!(
                    "push_group_to_group: parent group {:?} not found in UI index, falling back to rebuild",
                    parent_id
                );
                self.rebuild_all_rows();
                return id;
            }
        };

        // 计算插入位置：父分组的最后一个元素之后
        let parent_row_count = self
            .all_rows
            .row_data(parent_ui_idx)
            .map(|r| r.total_row_count as usize)
            .unwrap_or(1);
        let insert_pos = parent_ui_idx + parent_row_count;

        let depth = self
            .all_rows
            .row_data(parent_ui_idx)
            .map(|r| r.depth + 1)
            .unwrap_or(1);
        let ancestor_groups = self.get_group_ancestor_groups(parent_id);

        // 先更新后续索引
        self.shift_indices_after(insert_pos, 1);

        // 插入新行和索引
        self.group_id_to_ui_index.insert(id, insert_pos);
        self.all_rows.insert(
            insert_pos,
            SlintSubtitleRow {
                row_type: crate::bridge::SlintSubtitleRowType::GroupHeader,
                depth,
                ancestor_groups,
                start_time: SharedString::new(),
                end_time: SharedString::new(),
                event_key: SlintKey {
                    key_index: 0,
                    key_version: 0,
                },
                text: SharedString::new(),
                style: SharedString::new(),
                layer: -1,
                is_comment: false,
                group_id_str: id.to_string().to_shared_string(),
                group_name: name.to_shared_string(),
                collapsed: false,
                total_row_count: 1,
            },
        );

        id
    }

    pub fn insert_group_to_group(
        &mut self,
        name: String,
        parent_id: EventGroupId,
        position: usize,
    ) -> EventGroupId {
        let items_len = self
            .group_manager
            .groups
            .get(&parent_id)
            .expect("insert_group_to_group: parent group must exist")
            .items
            .len();

        let pos = position.min(items_len);
        let id = self
            .group_manager
            .insert_group_to_group(name.clone(), parent_id, position);
        self.mark_group_row_count_dirty(parent_id);

        // 计算在 all_rows 中的实际插入位置
        let parent_ui_idx = match self.group_id_to_ui_index.get(&parent_id) {
            Some(&idx) => idx,
            None => {
                // 如果找不到父分组UI索引，回退到重建
                self.rebuild_all_rows();
                return id;
            }
        };

        let insert_ui_pos = if pos == 0 {
            // 插入在父分组头后面
            parent_ui_idx + 1
        } else {
            // 插入在第 pos-1 个元素之后
            self.calculate_ui_position_after_group_item(parent_id, pos - 1)
        };

        let depth = self
            .all_rows
            .row_data(parent_ui_idx)
            .map(|r| r.depth + 1)
            .unwrap_or(1);
        let ancestor_groups = self.get_group_ancestor_groups(parent_id);

        // 先更新后续索引
        self.shift_indices_after(insert_ui_pos, 1);

        // 插入新行和索引
        self.group_id_to_ui_index.insert(id, insert_ui_pos);
        self.all_rows.insert(
            insert_ui_pos,
            SlintSubtitleRow {
                row_type: crate::bridge::SlintSubtitleRowType::GroupHeader,
                depth,
                ancestor_groups,
                start_time: SharedString::new(),
                end_time: SharedString::new(),
                event_key: SlintKey {
                    key_index: 0,
                    key_version: 0,
                },
                text: SharedString::new(),
                style: SharedString::new(),
                layer: -1,
                is_comment: false,
                group_id_str: id.to_string().to_shared_string(),
                group_name: name.to_shared_string(),
                collapsed: false,
                total_row_count: 1,
            },
        );

        id
    }

    pub fn move_group(
        &mut self,
        group_id: EventGroupId,
        target: EventGroupItemLocation,
    ) -> Result<()> {
        let old_location = self.group_manager.group_location.get(&group_id).copied();

        // 获取当前UI位置和占用的行数
        let (old_ui_pos, row_count) = match self.group_id_to_ui_index.get(&group_id) {
            Some(&pos) => {
                let count = self
                    .all_rows
                    .row_data(pos)
                    .map(|r| r.total_row_count as usize)
                    .unwrap_or(1);
                (pos, count)
            }
            None => {
                // 找不到，回退到完整移动
                self.group_manager.move_group(group_id, target)?;
                self.rebuild_all_rows();
                return Ok(());
            }
        };

        // 计算目标UI位置
        let target_ui_pos = match target {
            EventGroupItemLocation::Root(pos) => {
                if pos == 0 {
                    0
                } else {
                    self.calculate_ui_position_after_root_item(pos - 1)
                }
            }
            EventGroupItemLocation::InGroup(parent_id, pos) => {
                let group_ui_idx = match self.group_id_to_ui_index.get(&parent_id) {
                    Some(&idx) => idx,
                    None => {
                        self.group_manager.move_group(group_id, target)?;
                        self.rebuild_all_rows();
                        return Ok(());
                    }
                };

                if pos == 0 {
                    group_ui_idx + 1
                } else {
                    self.calculate_ui_position_after_group_item(parent_id, pos - 1)
                }
            }
        };

        // 获取目标位置的元数据（depth 和 ancestor_groups）
        let (new_depth, new_ancestor_groups) = match target {
            EventGroupItemLocation::Root(_) => (0, ModelRc::from(&[] as &[SharedString])),
            EventGroupItemLocation::InGroup(parent_id, _) => {
                let group_idx = self.group_id_to_ui_index.get(&parent_id).unwrap();
                let depth = self
                    .all_rows
                    .row_data(*group_idx)
                    .map(|r| r.depth + 1)
                    .unwrap_or(1);
                let ancestor_groups = self.get_group_ancestor_groups(parent_id);
                (depth, ancestor_groups)
            }
        };

        // 更新 group_manager
        self.group_manager.move_group(group_id, target)?;

        // 标记相关分组的 row_count 失效
        if let Some(EventGroupItemLocation::InGroup(old_parent, _)) = old_location {
            self.mark_group_row_count_dirty(old_parent);
        }
        if let EventGroupItemLocation::InGroup(new_parent, _) = target {
            self.mark_group_row_count_dirty(new_parent);
        }

        // 如果位置相同，只需要更新元数据
        if old_ui_pos == target_ui_pos {
            self.update_subtree_metadata(
                old_ui_pos,
                row_count,
                new_depth,
                &new_ancestor_groups,
                group_id,
            );
            return Ok(());
        }

        // 提取要移动的所有行并更新元数据
        let mut rows_to_move = Vec::with_capacity(row_count);
        for _ in 0..row_count {
            if let Some(row) = self.all_rows.row_data(old_ui_pos) {
                rows_to_move.push(row);
                self.all_rows.remove(old_ui_pos);
            }
        }

        // 调整插入位置（如果目标在删除位置之后）
        let adjusted_target = if target_ui_pos > old_ui_pos {
            target_ui_pos - row_count
        } else {
            target_ui_pos
        };

        // 插入所有行
        for (i, row) in rows_to_move.into_iter().enumerate() {
            self.all_rows.insert(adjusted_target + i, row);
        }

        // 更新子树的元数据
        self.update_subtree_metadata(
            adjusted_target,
            row_count,
            new_depth,
            &new_ancestor_groups,
            group_id,
        );

        // 更新索引映射
        let old_range_end = old_ui_pos + row_count;
        let new_range_end = adjusted_target + row_count;

        // 收集需要更新的索引
        let mut event_updates = Vec::new();
        let mut group_updates = Vec::new();

        for (&key, &idx) in self.event_key_to_ui_index.iter() {
            let new_idx = if idx >= old_ui_pos && idx < old_range_end {
                // 在移动的范围内
                adjusted_target + (idx - old_ui_pos)
            } else if old_ui_pos < adjusted_target {
                // 向后移动：中间的元素向前移
                if idx >= old_range_end && idx < new_range_end {
                    idx - row_count
                } else {
                    idx
                }
            } else {
                // 向前移动：中间的元素向后移
                if idx >= adjusted_target && idx < old_ui_pos {
                    idx + row_count
                } else {
                    idx
                }
            };

            if new_idx != idx {
                event_updates.push((key, new_idx));
            }
        }

        for (&gid, &idx) in self.group_id_to_ui_index.iter() {
            let new_idx = if idx >= old_ui_pos && idx < old_range_end {
                // 在移动的范围内
                adjusted_target + (idx - old_ui_pos)
            } else if old_ui_pos < adjusted_target {
                // 向后移动：中间的元素向前移
                if idx >= old_range_end && idx < new_range_end {
                    idx - row_count
                } else {
                    idx
                }
            } else {
                // 向前移动：中间的元素向后移
                if idx >= adjusted_target && idx < old_ui_pos {
                    idx + row_count
                } else {
                    idx
                }
            };

            if new_idx != idx {
                group_updates.push((gid, new_idx));
            }
        }

        // 应用更新
        for (key, new_idx) in event_updates {
            self.event_key_to_ui_index.insert(key, new_idx);
        }
        for (gid, new_idx) in group_updates {
            self.group_id_to_ui_index.insert(gid, new_idx);
        }

        Ok(())
    }

    /// 更新子树的元数据（depth 和 ancestor_groups）
    fn update_subtree_metadata(
        &mut self,
        start_pos: usize,
        row_count: usize,
        base_depth: i32,
        base_ancestor_groups: &ModelRc<SharedString>,
        moved_group_id: EventGroupId,
    ) {
        // 构建移动分组的新 ancestor_groups（基础祖先 + 自己）
        let mut group_ancestors: Vec<SharedString> = base_ancestor_groups.iter().collect();
        let group_id_str = moved_group_id.to_string().to_shared_string();
        group_ancestors.push(group_id_str.clone());
        let group_ancestor_model = ModelRc::from(group_ancestors.as_slice());

        for i in 0..row_count {
            let idx = start_pos + i;
            if let Some(mut row) = self.all_rows.row_data(idx) {
                if i == 0 {
                    // 分组头：使用基础的 depth 和 ancestor_groups
                    row.depth = base_depth;
                    row.ancestor_groups = base_ancestor_groups.clone();
                } else {
                    // 子元素：深度 = 原深度 - 旧基础深度 + 新基础深度
                    // 这样可以保持子树内部的相对深度
                    let old_base_depth = self.all_rows.row_data(start_pos).unwrap().depth;
                    let depth_offset = row.depth - old_base_depth;
                    row.depth = base_depth + depth_offset;

                    // ancestor_groups：替换前缀
                    // 原祖先列表的第一个元素是移动分组的 ID
                    // 需要替换为新的基础祖先 + 移动分组 ID
                    let old_ancestors: Vec<SharedString> = row.ancestor_groups.iter().collect();
                    if !old_ancestors.is_empty() && old_ancestors[0] == group_id_str {
                        // 保留从第二个开始的祖先（子分组的部分）
                        let mut new_ancestors: Vec<SharedString> =
                            base_ancestor_groups.iter().collect();
                        new_ancestors.push(group_id_str.clone());
                        new_ancestors.extend_from_slice(&old_ancestors[1..]);
                        row.ancestor_groups = ModelRc::from(new_ancestors.as_slice());
                    } else {
                        // 直接子元素（事件或直接子分组）
                        row.ancestor_groups = group_ancestor_model.clone();
                    }
                }

                self.all_rows.set_row_data(idx, row);
            }
        }
    }

    pub fn remove_event(&mut self, event_key: EventKey) {
        let location = self.group_manager.event_location.get(&event_key).copied();

        // 获取 UI 索引
        let ui_index = self.event_key_to_ui_index.get(&event_key).copied();

        // 更新 group_manager
        self.group_manager.remove_event(event_key);

        // 标记分组失效
        if let Some(EventGroupItemLocation::InGroup(group_id, _)) = location {
            self.mark_group_row_count_dirty(group_id);
        }

        // 更新 UI
        if let Some(idx) = ui_index {
            // 先移除索引
            self.event_key_to_ui_index.remove(&event_key);

            // 删除行
            self.all_rows.remove(idx);

            // 更新后续索引
            self.shift_indices_after(idx, -1);
        }
    }

    pub fn ungroup(&mut self, group_id: EventGroupId) -> Result<()> {
        // 获取分组信息
        let group = self
            .group_manager
            .groups
            .get(&group_id)
            .expect("ungroup: group must exist");
        let items = group.items.clone();
        let parent = group.parent;

        // 获取分组的 UI 位置和行数
        let group_ui_idx = match self.group_id_to_ui_index.get(&group_id) {
            Some(&idx) => idx,
            None => {
                warn!(
                    "ungroup: group {:?} not found in UI index, falling back to rebuild",
                    group_id
                );
                self.group_manager.ungroup(group_id)?;
                self.rebuild_all_rows();
                return Ok(());
            }
        };

        let group_row_count = self
            .all_rows
            .row_data(group_ui_idx)
            .map(|r| r.total_row_count as usize)
            .unwrap_or(1);

        // 获取目标层级的元数据
        let (target_depth, target_ancestor_groups) = match parent {
            None => (0, ModelRc::from(&[] as &[SharedString])),
            Some(parent_id) => {
                let parent_idx = self
                    .group_id_to_ui_index
                    .get(&parent_id)
                    .expect("ungroup: parent group must exist in UI index");
                let depth = self
                    .all_rows
                    .row_data(*parent_idx)
                    .map(|r| r.depth + 1)
                    .unwrap_or(1);
                let ancestor_groups = self.get_group_ancestor_groups(parent_id);
                (depth, ancestor_groups)
            }
        };

        // 执行 ungroup
        self.group_manager.ungroup(group_id)?;

        // 标记父分组的 row_count 失效（如果有父分组）
        if let Some(parent_id) = parent {
            self.mark_group_row_count_dirty(parent_id);
        }

        // UI 更新：
        // 1. 删除分组头（第一行）
        self.all_rows.remove(group_ui_idx);
        self.group_id_to_ui_index.remove(&group_id);

        // 2. 更新子元素的元数据（继承上一层的 depth 和 ancestor_groups）
        // 子元素现在从 group_ui_idx 开始（因为分组头已删除）
        // 子元素数量 = group_row_count - 1
        let child_count = group_row_count - 1;

        for i in 0..child_count {
            let idx = group_ui_idx + i;
            if let Some(mut row) = self.all_rows.row_data(idx) {
                // 深度减1（从分组内提升到上一层）
                row.depth = target_depth;
                row.ancestor_groups = target_ancestor_groups.clone();
                self.all_rows.set_row_data(idx, row);
            }
        }

        // 3. 更新后续索引（删除了分组头，后续索引 -1）
        self.shift_indices_after(group_ui_idx + child_count, -1);

        // 4. 更新子元素在索引映射中的位置（它们向前移动了1位）
        for item in &items {
            match item {
                EventGroupItem::Event(event_key) => {
                    if let Some(idx) = self.event_key_to_ui_index.get_mut(event_key) {
                        *idx -= 1;
                    }
                }
                EventGroupItem::ChildGroup(child_id) => {
                    if let Some(idx) = self.group_id_to_ui_index.get_mut(child_id) {
                        *idx -= 1;
                    }
                }
            }
        }

        Ok(())
    }

    pub fn remove_group(&mut self, group_id: EventGroupId) {
        // 获取分组信息
        let group = self.group_manager.groups.get(&group_id);
        let has_children = group.map(|g| !g.items.is_empty()).unwrap_or(false);
        let parent = group.and_then(|g| g.parent);

        // 如果有子元素，需要重建（因为要删除多行）
        if has_children {
            self.group_manager.remove_group(group_id);
            self.rebuild_all_rows();
            return;
        }

        // 空分组，可以局部更新
        let ui_index = self.group_id_to_ui_index.get(&group_id).copied();

        // 更新 group_manager
        self.group_manager.remove_group(group_id);

        // 标记父分组失效
        if let Some(parent_id) = parent {
            self.mark_group_row_count_dirty(parent_id);
        }

        // 更新 UI
        if let Some(idx) = ui_index {
            // 先移除索引
            self.group_id_to_ui_index.remove(&group_id);

            // 删除行（只有分组头1行）
            self.all_rows.remove(idx);

            // 更新后续索引
            self.shift_indices_after(idx, -1);
        }
    }

    /// 索引偏移：在插入/删除行后更新所有后续索引
    /// position: 插入/删除的位置
    /// offset: 偏移量（插入为正，删除为负）
    ///
    /// 注意：当 offset > 0（插入）时，position 处的索引不会被更新
    ///       当 offset < 0（删除）时，position 及之后的索引会被更新
    fn shift_indices_after(&mut self, position: usize, offset: i32) {
        if offset > 0 {
            // 插入：只更新 position 之后的索引（不含 position）
            for idx in self.event_key_to_ui_index.values_mut() {
                if *idx > position {
                    *idx = (*idx as i32 + offset) as usize;
                }
            }
            for idx in self.group_id_to_ui_index.values_mut() {
                if *idx > position {
                    *idx = (*idx as i32 + offset) as usize;
                }
            }
        } else {
            // 删除：更新 position 及之后的索引（含 position）
            for idx in self.event_key_to_ui_index.values_mut() {
                if *idx >= position {
                    *idx = (*idx as i32 + offset) as usize;
                }
            }
            for idx in self.group_id_to_ui_index.values_mut() {
                if *idx >= position {
                    *idx = (*idx as i32 + offset) as usize;
                }
            }
        }
    }
    // TODO: 以上为AI生成，暂未详细审查
    // ===============================================================
}
