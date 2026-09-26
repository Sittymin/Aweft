use crate::ids::{ActorId, ItemId};
use anyhow::{Context, Result, anyhow};
use chrono::TimeDelta;
use nom::{
    Finish, IResult, Parser,
    branch::alt,
    character::complete::{alpha1, char, digit1, multispace0},
    combinator::{all_consuming, map_res, verify},
    number::complete::double,
    sequence::separated_pair,
};
use serde::{Deserialize, Serialize};
use std::collections::{HashMap, HashSet};

// === 数据结构定义 ===

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Sequence {
    // 根节点列表：定义了最顶层的顺序
    pub root_items: Vec<ItemId>,

    // 所有项的扁平存储池 (无论是 Event 还是 Group 都在这里)
    // 替代了原来的 groups: HashMap 和 event_location
    pub items: HashMap<ItemId, StructureItem>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct StructureItem {
    pub id: ItemId,
    // 反向索引：方便向上查找
    pub parent: Option<ItemId>,
    // 锁定状态 (业务逻辑)
    pub locked: bool,
    // 可见性 (业务逻辑)
    pub visible: bool,
    // 实际内容
    pub content: ItemContent,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum ItemContent {
    // 分组
    Group(GroupData),
    // 实际的字幕事件行
    Event(EventData),
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct GroupData {
    pub name: String,
    pub collapsed: bool, // 持久化的折叠状态
    pub color: Option<String>,
    // 子项 ID 列表：定义了组内的顺序
    pub children: Vec<ItemId>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct EventData {
    // 基础时序
    pub start_time: TimeDelta,
    pub end_time: TimeDelta,
    // 或许叫 Tag 标签更好？
    pub actor: Option<ActorId>,

    // 内容与样式
    pub text: String,
    // 各个音节字符长度与持续时间（卡拉OK）
    pub segment_timing: Option<SegmentTiming>,

    // FIXME: 暂时用 String 占位，之后会被替换为 NodeGraph 的引用或定义
    pub style: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SegmentTiming {
    pub segments: Vec<Segment>,
}

impl Default for SegmentTiming {
    fn default() -> Self {
        SegmentTiming {
            segments: Vec::new(),
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Segment {
    pub len: usize,
    pub duration: TimeDelta,
}

// 辅助枚举：用于定位插入位置
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ItemLocation {
    Root(usize),
    InGroup(ItemId, usize),
}

// === 业务逻辑实现 ===

impl Sequence {
    pub fn new() -> Self {
        Self {
            root_items: Vec::new(),
            items: HashMap::new(),
        }
    }

    // === 查询 ===

    pub fn get(&self, id: ItemId) -> Option<&StructureItem> {
        self.items.get(&id)
    }

    pub fn get_mut(&mut self, id: ItemId) -> Option<&mut StructureItem> {
        self.items.get_mut(&id)
    }

    pub fn is_ancestor(&self, potential_ancestor: ItemId, descendant: ItemId) -> bool {
        let mut current = descendant;
        while let Some(item) = self.items.get(&current) {
            if let Some(parent_id) = item.parent {
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

    /// 检查在 potential_ancestor_set 当中是否有 descendant 的父级
    pub fn has_ancestor_in_set(
        &self,
        descendant: &ItemId,
        potential_ancestor_set: &HashSet<ItemId>,
    ) -> bool {
        let mut current = self.items.get(descendant).and_then(|i| i.parent);
        while let Some(pid) = current {
            if potential_ancestor_set.contains(&pid) {
                return true;
            }
            current = self.items.get(&pid).and_then(|i| i.parent);
        }
        false
    }

    pub fn event_time(&self, id: ItemId, is_start_time: bool) -> Result<TimeDelta> {
        if let Some(item) = self.items.get(&id) {
            if let ItemContent::Event(event_data) = &item.content {
                if is_start_time {
                    Ok(event_data.start_time)
                } else {
                    Ok(event_data.end_time)
                }
            } else {
                return Err(anyhow!("Item 不是 Event"));
            }
        } else {
            return Err(anyhow!("Item 没找到"));
        }
    }

    // === 修改数据 ===

    pub fn set_event_time_fuzzy(
        &mut self,
        id: ItemId,
        is_start_time: bool,
        text: &str,
    ) -> Result<TimeDelta> {
        let input = text.trim();

        // +/- 调整
        if let Some(sign) = input.chars().next() {
            if sign == '+' || sign == '-' {
                let delta = parse_fuzzy_timecode(&input[1..])?;
                let delta = if sign == '-' { -delta } else { delta };
                return self.adjust_event_time(id, is_start_time, delta);
            }
        }

        let target_time = parse_fuzzy_timecode(input)?;

        if let Some(item) = self.items.get_mut(&id) {
            if let ItemContent::Event(event_data) = &mut item.content {
                if is_start_time && target_time <= event_data.end_time {
                    event_data.start_time = target_time;
                } else if !is_start_time && target_time >= event_data.start_time {
                    event_data.end_time = target_time;
                } else {
                    return Err(anyhow!("时间无法形成范围"));
                }
            } else {
                return Err(anyhow!("Item 不是 Event"));
            }
        } else {
            return Err(anyhow!("Item 没找到"));
        }
        Ok(target_time)
    }

    pub fn adjust_event_time(
        &mut self,
        id: ItemId,
        is_start_time: bool,
        delta: TimeDelta,
    ) -> Result<TimeDelta> {
        if let Some(item) = self.items.get_mut(&id) {
            if let ItemContent::Event(event_data) = &mut item.content {
                if is_start_time {
                    let new_start_time = event_data.start_time + delta;
                    // 避免越界
                    event_data.start_time = new_start_time.min(event_data.end_time);
                    Ok(event_data.start_time)
                } else {
                    // is_end_time
                    let new_end_time = event_data.end_time + delta;
                    event_data.end_time = new_end_time.max(event_data.start_time);
                    Ok(event_data.end_time)
                }
            } else {
                Err(anyhow!("Item 不是 Event"))
            }
        } else {
            Err(anyhow!("Item 没找到"))
        }
    }
    // 不修改实际数据的版本（纯只读预览计算）
    pub fn preview_adjust_event_time(
        &self,
        id: ItemId,
        is_start_time: bool,
        delta: TimeDelta,
    ) -> Result<TimeDelta> {
        if let Some(item) = self.items.get(&id) {
            if let ItemContent::Event(event_data) = &item.content {
                if is_start_time {
                    let new_start_time = event_data.start_time + delta;
                    // 避免越界
                    Ok(new_start_time.min(event_data.end_time))
                } else {
                    // is_end_time
                    let new_end_time = event_data.end_time + delta;
                    Ok(new_end_time.max(event_data.start_time))
                }
            } else {
                Err(anyhow!("Item 不是 Event"))
            }
        } else {
            Err(anyhow!("Item 没找到"))
        }
    }

    // === 创建与插入 ===

    /// 创建一个新的事件行并插入
    pub fn create_event(&mut self, data: EventData, location: ItemLocation) -> Result<ItemId> {
        let id = ItemId::new();
        let parent = match location {
            ItemLocation::Root(_) => None,
            ItemLocation::InGroup(pid, _) => Some(pid),
        };

        let item = StructureItem {
            id,
            parent,
            locked: false,
            visible: true,
            content: ItemContent::Event(data),
        };

        self.insert_item_to_sequence(item, location)?;
        Ok(id)
    }

    /// 创建一个新的分组并插入
    pub fn create_group(&mut self, data: GroupData, location: ItemLocation) -> Result<ItemId> {
        let id = ItemId::new();
        let parent = match location {
            ItemLocation::Root(_) => None,
            ItemLocation::InGroup(pid, _) => Some(pid),
        };

        let item = StructureItem {
            id,
            parent,
            locked: false,
            visible: true,
            content: ItemContent::Group(data),
        };

        self.insert_item_to_sequence(item, location)?;
        Ok(id)
    }

    pub fn insert_event_after(
        &mut self,
        item_id: ItemId,
        end_time: TimeDelta,
        actor: Option<ActorId>,
        text: String,
        segment_timing: Option<SegmentTiming>,
        // FIXME: 之后需要更改为节点链
        style: Option<String>,
    ) -> Result<ItemId> {
        let location = self.find_insert_location(item_id)?;
        // 延续参考行的时长：新行 start = 参考行 end，end = 参考行 end + 原时长。
        // 这样新行保持与参考行相同的时长，向后平移（而非零长度起始）。
        let duration = self
            .items
            .get(&item_id)
            .and_then(|it| match &it.content {
                ItemContent::Event(e) => Some(e.end_time - e.start_time),
                ItemContent::Group(_) => None,
            })
            .unwrap_or_default();
        let new_item = EventData {
            start_time: end_time,
            end_time: end_time + duration,
            actor,
            text,
            segment_timing,
            style: style.unwrap_or_default(),
        };
        let new_item_id = self.create_event(new_item, location)?;
        Ok(new_item_id)
    }

    pub fn insert_group_after(&mut self, item_id: ItemId, text: String) -> Result<ItemId> {
        let location = self.find_insert_location(item_id)?;
        let new_item = GroupData {
            name: text,
            collapsed: false,
            color: None,
            children: Vec::new(),
        };
        let new_item_id = self.create_group(new_item, location)?;
        Ok(new_item_id)
    }

    /// 计算"在 `item_id` 之后/之内"的逻辑插入位置。
    ///
    /// - 若 `item_id` 是 Group：返回 `InGroup(item_id, len(children))`（插入到组末尾）。
    /// - 若是事件/分组：返回 `Root(idx+1)` 或 `InGroup(parent, idx+1)`，即"紧跟其后"。
    pub fn find_insert_location(&self, item_id: ItemId) -> Result<ItemLocation> {
        let item = self.items.get(&item_id).context("不存在的 ItemId")?;

        // 是分组时则是分组内最后一个
        if let ItemContent::Group(g) = &item.content {
            return Ok(ItemLocation::InGroup(item_id, g.children.len()));
        }

        match item.parent {
            None => {
                let idx = self
                    .root_items
                    .iter()
                    .position(|&x| x == item_id)
                    .context("root_items 未找到 item")?;
                Ok(ItemLocation::Root(idx + 1))
            }
            Some(pid) => {
                let parent_item = self.items.get(&pid).context("parent 不存在")?;
                if let ItemContent::Group(g) = &parent_item.content {
                    let idx = g
                        .children
                        .iter()
                        .position(|&x| x == item_id)
                        .context("children 未找到 item")?;
                    Ok(ItemLocation::InGroup(pid, idx + 1))
                } else {
                    Err(anyhow!("parent 不是 group"))
                }
            }
        }
    }

    // 内部 helper：将 item 放入 map 并链接到 children 列表
    fn insert_item_to_sequence(
        &mut self,
        item: StructureItem,
        location: ItemLocation,
    ) -> Result<()> {
        let id = item.id;
        self.items.insert(id, item);
        self.attach_item(id, location)
    }

    // === 移动操作 ===

    pub fn move_item(&mut self, item_id: ItemId, target: ItemLocation) -> Result<()> {
        // 1. 检查合法性：不能移到自己里面
        if let ItemLocation::InGroup(target_group_id, _) = target {
            if item_id == target_group_id || self.is_ancestor(item_id, target_group_id) {
                return Err(anyhow!("Cannot move item into its own descendant"));
            }
        }

        self.attach_item(item_id, target)
    }

    // === 删除操作 ===

    /// 移除一个项。如果是分组，这会递归删除所有子项！
    pub fn remove_item(&mut self, item_id: ItemId) -> Result<()> {
        // 1. 先从树结构中摘除引用
        self.detach_item(item_id)?;

        // 2. 递归清理 Map 中的数据
        self.remove_item_recursive(item_id);

        Ok(())
    }

    // 内部递归清理
    fn remove_item_recursive(&mut self, item_id: ItemId) {
        // 如果存在且是分组，先删孩子
        if let Some(item) = self.items.remove(&item_id) {
            if let ItemContent::Group(g) = item.content {
                for child_id in g.children {
                    self.remove_item_recursive(child_id);
                }
            }
        }
    }

    /// 解散分组 (Ungroup)：移除分组壳子，将子项提升到父级
    ///
    /// 返回被提升的子项 ID 列表（保持原文档顺序），便于 UI 层快速更新。
    pub fn ungroup(&mut self, group_id: ItemId) -> Result<Vec<ItemId>> {
        let (parent_id, children, original_idx) = {
            let group = self
                .items
                .get(&group_id)
                .ok_or_else(|| anyhow!("Group not found"))?;

            let parent = group.parent;

            // 获取当前在父级中的索引，以便把子项插到正确位置。
            // 此处若 "找不到" / "类型不符" 说明 parent 指针与父级
            // children/root_items 已不一致——数据结构不变量被破坏，属编程
            // 错误，panic 暴露而非静默兜底。
            let idx = if let Some(pid) = parent {
                let p_item = self.items.get(&pid).unwrap_or_else(|| {
                    unreachable!("ungroup: parent {:?} 不存在（数据不一致）", pid)
                });
                if let ItemContent::Group(pg) = &p_item.content {
                    pg.children
                        .iter()
                        .position(|&x| x == group_id)
                        .unwrap_or_else(|| {
                            unreachable!(
                                "ungroup: {} 不在父级 {} 的 children 中（数据不一致）",
                                group_id, pid
                            )
                        })
                } else {
                    unreachable!("ungroup: 父级 {} 不是 Group（数据不一致）", pid);
                }
            } else {
                self.root_items
                    .iter()
                    .position(|&x| x == group_id)
                    .unwrap_or_else(|| {
                        unreachable!("ungroup: {} 不在 root_items 中（数据不一致）", group_id)
                    })
            };

            match &group.content {
                ItemContent::Group(g) => (parent, g.children.clone(), idx),
                _ => return Err(anyhow!("Target is not a group")),
            }
        };

        // 1. 摘除分组
        self.detach_item(group_id)?;
        self.items.remove(&group_id);

        // 2. 将子项们移动到父级
        // 倒序插入 + 固定索引，以保持子项的原始文档顺序
        // 子项此时 parent 仍指向已被移除的分组，先清除再复用 attach_item
        let mut new_ids = Vec::with_capacity(children.len());
        for child_id in children.iter().rev() {
            // 清除旧的 parent 指针，否则 attach_item → detach_item
            // 会尝试从已删除的旧分组中摘除，导致 "Group not found" 错误
            if let Some(child) = self.items.get_mut(child_id) {
                child.parent = None;
            }

            let target = match parent_id {
                Some(pid) => ItemLocation::InGroup(pid, original_idx),
                None => ItemLocation::Root(original_idx),
            };

            self.attach_item(*child_id, target)?;
            new_ids.push(*child_id);
        }

        new_ids.reverse(); // 恢复文档顺序
        Ok(new_ids)
    }

    // === 辅助方法 ===
    //
    /// 将一个**已经存在于 `self.items`** 的项链接到指定位置（更新 parent
    /// 指针并写入父级 `children` 或 `root_items`）。
    ///
    /// 幂等：内部先 `detach_item` 剥离旧位置，再挂新位置——重复调用或
    /// 移动到已有位置不会产生重复引用（root_items / children 不会重复）。
    ///
    /// 用于分组子树仅需要重新链接最高父级，剥离原始位置
    pub fn attach_item(&mut self, item_id: ItemId, location: ItemLocation) -> Result<()> {
        // 校验
        self.items
            .get(&item_id)
            .ok_or_else(|| anyhow!("Item not found: {}", item_id))?;

        if let ItemLocation::InGroup(group_id, _) = location {
            let group_item = self
                .items
                .get(&group_id)
                .ok_or_else(|| anyhow!("Group not found: {}", group_id))?;
            if !matches!(group_item.content, ItemContent::Group(_)) {
                return Err(anyhow!("Target item {} is not a group", group_id));
            }
        }

        self.detach_item(item_id)?;

        let new_parent = match location {
            ItemLocation::Root(_) => None,
            ItemLocation::InGroup(pid, _) => Some(pid),
        };

        let item = self
            .items
            .get_mut(&item_id)
            .unwrap_or_else(|| unreachable!("[attach_item] item_id 已在上方校验存在"));
        item.parent = new_parent;

        match location {
            ItemLocation::Root(idx) => {
                let pos = idx.min(self.root_items.len());
                self.root_items.insert(pos, item_id);
            }
            ItemLocation::InGroup(group_id, idx) => {
                let group_item = self
                    .items
                    .get_mut(&group_id)
                    .unwrap_or_else(|| unreachable!("[attach_item] group_id 已在上方校验存在"));

                match &mut group_item.content {
                    ItemContent::Group(g) => {
                        let pos = idx.min(g.children.len());
                        g.children.insert(pos, item_id);
                    }
                    _ => unreachable!("[attach_item] group 类型已在上方校验"),
                }
            }
        }
        Ok(())
    }

    // 将指定 Item 剥离树结构
    fn detach_item(&mut self, item_id: ItemId) -> Result<usize> {
        let old_parent = self
            .items
            .get(&item_id)
            .ok_or_else(|| anyhow!("Item not found: {}", item_id))?
            .parent;

        // 从旧容器摘除
        let removed = match old_parent {
            None => {
                let before = self.root_items.len();
                self.root_items.retain(|&x| x != item_id);
                before - self.root_items.len()
            }
            Some(parent_item_id) => {
                let group_item = self
                    .items
                    .get_mut(&parent_item_id)
                    .ok_or_else(|| anyhow!("Group not found: {}", parent_item_id))?;

                match &mut group_item.content {
                    ItemContent::Group(g) => {
                        let before = g.children.len();
                        g.children.retain(|&x| x != item_id);
                        before - g.children.len()
                    }
                    _ => return Err(anyhow!("Target item {} is not a group", parent_item_id)),
                }
            }
        };

        // 断开 parent 指针
        let item = self
            .items
            .get_mut(&item_id)
            .ok_or_else(|| anyhow!("Item not found: {}", item_id))?;
        item.parent = None;

        Ok(removed)
    }
}

fn parse_unit_based(input: &str) -> IResult<&str, TimeDelta> {
    let (input, (value, unit)) = separated_pair(double, multispace0, alpha1).parse(input)?;
    let ms = match unit {
        "ms" => value,
        "s" => value * 1_000.0,
        "m" => value * 60.0 * 1_000.0,
        "h" => value * 60.0 * 60.0 * 1_000.0,
        _ => {
            return Err(nom::Err::Error(nom::error::Error::new(
                input,
                nom::error::ErrorKind::Tag,
            )));
        }
    };
    let duration = TimeDelta::milliseconds(ms.round() as i64);
    Ok((input, duration))
}

fn parse_standard(input: &str) -> IResult<&str, TimeDelta> {
    let parse_int = |input| map_res(digit1, |s: &str| s.parse::<i64>()).parse(input);

    let (input, (hours, _, mins, _, secs_float)) =
        (parse_int, char(':'), parse_int, char(':'), double).parse(input)?;

    let total_ms =
        (hours * 60 * 60 * 1_000) + (mins * 60 * 1_000) + (secs_float * 1_000.0).round() as i64;

    Ok((input, TimeDelta::milliseconds(total_ms)))
}

fn parse_abbreviated(input: &str) -> IResult<&str, TimeDelta> {
    map_res(
        verify(digit1, |s: &str| !s.is_empty()),
        |digits: &str| -> Result<TimeDelta, std::num::ParseIntError> {
            let len = digits.len();

            // 辅助函数：尝试解析切片，如果有切片则 parse，否则返回 0
            // 这里我们使用 ? 操作符，如果 parse 失败（如溢出），直接返回 Err
            let parse_slice = |start: usize, end: usize| -> Result<i64, std::num::ParseIntError> {
                if start >= end {
                    Ok(0)
                } else {
                    digits[start..end].parse::<i64>()
                }
            };

            // 厘秒 (cs): 最后2位
            let cs_part = if len >= 2 {
                parse_slice(len - 2, len)?
            } else {
                parse_slice(0, len)?
            };

            // 秒 (s): 倒数3,4位
            let s_end = if len > 2 { len - 2 } else { 0 };
            let s_start = if s_end >= 2 { s_end - 2 } else { 0 };
            let s_part = parse_slice(s_start, s_end)?;

            // 分 (m): 倒数5,6位
            let m_end = if len > 4 { len - 4 } else { 0 };
            let m_start = if m_end >= 2 { m_end - 2 } else { 0 };
            let m_part = parse_slice(m_start, m_end)?;

            // 时 (h): 剩余部分
            let h_end = if len > 6 { len - 6 } else { 0 };
            let h_part = parse_slice(0, h_end)?;

            let total_ms =
                (h_part * 3_600_000) + (m_part * 60_000) + (s_part * 1_000) + (cs_part * 10); // 1 cs = 10 ms

            Ok(TimeDelta::milliseconds(total_ms))
        },
    )
    .parse(input)
}

// TODO: 按照帧率也可能需要实现(应该需要获取当前播放的视频帧率)
// 1500f -> 根据当前视频的帧率（例如 23.976fps）自动计算为 00:01:02.58。
fn parse_fuzzy_timecode(text: &str) -> Result<TimeDelta> {
    let input = text.trim();

    // 使用 all_consuming 确保全匹配
    let (_, delta) = all_consuming(alt((
        parse_unit_based,  // "1.5m"
        parse_standard,    // "1:23:45.67"
        parse_abbreviated, // "123456"
    )))
    .parse(input)
    .finish()
    .map_err(|e| anyhow!("无法解析时间格式: {} (错误代码: {:?})", e.input, e.code))?;

    Ok(delta)
}
