// TODO: 实现扁平化传递给 Slint 侧展示，并尝试互相操作数据结构

use std::{
    collections::{BTreeMap, HashMap},
    mem,
};

use anyhow::Result;
use aweft_ass::types::event::EventKey;
use uuid::Uuid;

/// 分组唯一标识符
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct EventGroupId(Uuid);

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
    pub groups: BTreeMap<EventGroupId, EventGroup>,
    pub root_items: Vec<EventGroupItem>,
    pub event_location: HashMap<EventKey, EventGroupItemLocation>,
    pub group_location: HashMap<EventGroupId, EventGroupItemLocation>,
}

impl EventGroupManager {
    pub fn new() -> Self {
        Self {
            groups: BTreeMap::new(),
            root_items: Vec::new(),
            event_location: HashMap::new(),
            group_location: HashMap::new(),
        }
    }

    pub fn push_event_to_root(&mut self, event_key: EventKey) {
        let index = self.root_items.len();
        self.root_items.push(EventGroupItem::Event(event_key));
        self.event_location
            .insert(event_key, EventGroupItemLocation::Root(index));
    }

    pub fn insert_event_to_root(&mut self, event_key: EventKey, position: usize) {
        let pos = position.min(self.root_items.len());
        self.root_items
            .insert(pos, EventGroupItem::Event(event_key));
        self.event_location
            .insert(event_key, EventGroupItemLocation::Root(pos));

        self.update_root_indices_after(pos + 1);
    }

    pub fn push_event_to_group(&mut self, event_key: EventKey, group_id: EventGroupId) {
        let group = self.groups.get_mut(&group_id).unwrap();
        let index = group.items.len();
        group.items.push(EventGroupItem::Event(event_key));

        self.event_location
            .insert(event_key, EventGroupItemLocation::InGroup(group_id, index));
    }

    pub fn insert_event_to_group(
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

    pub fn move_event(&mut self, event_key: EventKey, target: EventGroupItemLocation) {
        self.remove_event(event_key);

        match target {
            EventGroupItemLocation::Root(index) => self.insert_event_to_root(event_key, index),
            EventGroupItemLocation::InGroup(group_id, index) => {
                self.insert_event_to_group(event_key, group_id, index)
            }
        }
    }

    pub fn push_group_to_root(&mut self, name: String) -> EventGroupId {
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

    pub fn insert_group_to_root(&mut self, name: String, position: usize) {
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
    }

    pub fn push_group_to_group(&mut self, name: String, parent_id: EventGroupId) -> EventGroupId {
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

    pub fn insert_group_to_group(
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

    pub fn move_group(
        &mut self,
        group_id: EventGroupId,
        target: EventGroupItemLocation,
    ) -> Result<()> {
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

    pub fn remove_event(&mut self, event_key: EventKey) {
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

    pub fn ungroup(&mut self, group_id: EventGroupId) -> Result<()> {
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

    pub fn remove_group(&mut self, group_id: EventGroupId) {
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
