// TODO: 实现扁平化传递给 Slint 侧展示，并尝试互相操作数据结构

use std::{collections::HashMap, mem};

use anyhow::Result;
use aweft_ass::{ASSFile, types::event::EventKey};
use uuid::Uuid;

use crate::bridge::parse_group_id;

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

pub struct LineEditorState {
    // 数据源
    pub group_manager: EventGroupManager,
    // FIXME: 应当使用样式节点链的高层抽象
    pub ass_file: ASSFile,

    // UI缓存
    ui_rows: Option<Vec<crate::bridge::types::SlintSubtitleRow>>,

    // row_count缓存
    group_row_counts: HashMap<EventGroupId, i32>,
    row_counts_valid: bool,

    // EventKey到UI索引的映射
    event_key_to_ui_index: HashMap<EventKey, usize>,
    group_id_to_ui_index: HashMap<EventGroupId, usize>,
}

impl LineEditorState {
    pub fn new() -> Self {
        Self {
            group_manager: EventGroupManager::new(),
            // FIXME: 应当使用样式节点链的高层抽象
            ass_file: ASSFile::new(),
            ui_rows: None,
            group_row_counts: HashMap::new(),
            row_counts_valid: false,
            event_key_to_ui_index: HashMap::new(),
            group_id_to_ui_index: HashMap::new(),
        }
    }

    // 标记row_count失效（任何修改分组结构的操作）
    pub fn invalidate_row_counts(&mut self) {
        self.row_counts_valid = false;
    }

    // 确保row_count有效
    pub fn ensure_row_counts_valid(&mut self) {
        if !self.row_counts_valid {
            self.recompute_row_counts();
        }
    }

    // 直接设置Group的row_count
    fn set_group_row_count(&mut self, group_id: EventGroupId, count: i32) {
        self.group_row_counts.insert(group_id, count);
    }

    // 重新计算所有row_count
    fn recompute_row_counts(&mut self) {
        use crate::bridge::types::precompute_group_row_counts;

        self.group_row_counts =
            precompute_group_row_counts(&self.group_manager.groups, &self.ass_file);
        self.row_counts_valid = true;
    }

    // 获取Group的row_count
    pub fn get_group_row_count(&mut self, group_id: EventGroupId) -> Option<i32> {
        self.ensure_row_counts_valid();
        self.group_row_counts.get(&group_id).copied()
    }

    // 完整重建 UI 列表
    pub fn build_ui_rows(&mut self) -> Vec<crate::bridge::types::SlintSubtitleRow> {
        use crate::bridge::types::flatten_subtitle_rows;

        let rows = flatten_subtitle_rows(
            &self.group_manager.root_items,
            &self.group_manager.groups,
            &self.ass_file,
        );

        // 构建索引映射
        self.event_key_to_ui_index.clear();
        self.group_id_to_ui_index.clear();

        for (i, row) in rows.iter().enumerate() {
            match row.row_type {
                crate::bridge::types::SlintSubtitleRowType::Event => {
                    let event_key = crate::bridge::types::slint_area_key_to_key(&row.event_key);
                    self.event_key_to_ui_index.insert(event_key, i);
                }
                crate::bridge::types::SlintSubtitleRowType::GroupHeader => {
                    if let Ok(group_id) = parse_group_id(&row.group_id_str) {
                        self.group_id_to_ui_index.insert(group_id, i);
                    }
                }
            }
        }

        // 缓存UI列表
        self.ui_rows = Some(rows.clone());
        self.row_counts_valid = true;

        rows
    }

    // 获取EventKey的UI索引
    pub fn get_event_ui_index(&self, event_key: EventKey) -> Option<usize> {
        self.event_key_to_ui_index.get(&event_key).copied()
    }

    // 获取GroupId的UI索引
    pub fn get_group_ui_index(&self, group_id: EventGroupId) -> Option<usize> {
        self.group_id_to_ui_index.get(&group_id).copied()
    }

    // === 包装EventGroupManager的操作，自动标记失效 ===

    pub fn push_event_to_root(&mut self, event_key: EventKey) {
        self.group_manager.push_event_to_root(event_key);
        self.ui_rows = None;
    }

    pub fn insert_event_to_root(&mut self, event_key: EventKey, position: usize) {
        self.group_manager.insert_event_to_root(event_key, position);
        self.ui_rows = None;
    }

    // NOTE: 可能是子分组，多个分组 count 都将失效
    pub fn push_event_to_group(&mut self, event_key: EventKey, group_id: EventGroupId) {
        self.group_manager.push_event_to_group(event_key, group_id);
        self.invalidate_row_counts();
        self.ui_rows = None;
    }

    // NOTE: 可能是子分组，多个分组 count 都将失效
    pub fn insert_event_to_group(
        &mut self,
        event_key: EventKey,
        group_id: EventGroupId,
        position: usize,
    ) {
        self.group_manager
            .insert_event_to_group(event_key, group_id, position);
        self.invalidate_row_counts();
        self.ui_rows = None;
    }

    // NOTE: 需要保证源位置和目标位置都是 Root 才不需要重建（目前未实现判断逻辑）
    pub fn move_event(&mut self, event_key: EventKey, target: EventGroupItemLocation) {
        self.group_manager.move_event(event_key, target);
        self.invalidate_row_counts();
        self.ui_rows = None;
    }

    // NOTE: Root 级别可以确定 Count 是多少
    pub fn push_group_to_root(&mut self, name: String) -> EventGroupId {
        let id = self.group_manager.push_group_to_root(name);

        self.set_group_row_count(id, 1);
        self.ui_rows = None;
        id
    }
    pub fn insert_group_to_root(&mut self, name: String, position: usize) -> EventGroupId {
        let id = self.group_manager.insert_group_to_root(name, position);

        self.set_group_row_count(id, 1);
        self.ui_rows = None;
        id
    }

    pub fn push_group_to_group(&mut self, name: String, parent_id: EventGroupId) -> EventGroupId {
        let id = self.group_manager.push_group_to_group(name, parent_id);
        self.invalidate_row_counts();
        self.ui_rows = None;
        id
    }
    pub fn insert_group_to_group(
        &mut self,
        name: String,
        parent_id: EventGroupId,
        position: usize,
    ) -> EventGroupId {
        let id = self
            .group_manager
            .insert_group_to_group(name, parent_id, position);
        self.invalidate_row_counts();
        self.ui_rows = None;
        id
    }
    pub fn move_group(
        &mut self,
        group_id: EventGroupId,
        target: EventGroupItemLocation,
    ) -> Result<()> {
        self.group_manager.move_group(group_id, target)?;
        self.invalidate_row_counts();
        self.ui_rows = None;
        Ok(())
    }

    // NOTE: 需要在 Root 级别才不需要标记失效（逻辑未实现）
    pub fn remove_event(&mut self, event_key: EventKey) {
        self.group_manager.remove_event(event_key);
        self.invalidate_row_counts();
        self.ui_rows = None;
    }

    pub fn ungroup(&mut self, group_id: EventGroupId) -> Result<()> {
        self.group_manager.ungroup(group_id)?;
        self.invalidate_row_counts();
        self.ui_rows = None;
        Ok(())
    }

    // NOTE: 需要在 Root 级别才不需要标记失效（逻辑未实现）
    pub fn remove_group(&mut self, group_id: EventGroupId) {
        self.group_manager.remove_group(group_id);
        self.invalidate_row_counts();
        self.ui_rows = None;
    }
}
