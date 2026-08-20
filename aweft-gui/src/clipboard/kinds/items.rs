//! 字幕条目剪贴板 Kind。
//!
//! 持有一棵子树（一个或多个 root + 全部后代），可序列化为多种 MIME：
//! - 主 MIME：JSON（高保真，含所有 `StructureItem` 字段与树结构）
//! - `text/plain;charset=utf-8`：每行一段字幕文本（兜底，便于跨应用粘贴）
//!
//! `from_selection` 从用户选中集合构造（自动过滤"祖先已选中"的项）。
//! `instantiate` 把 clip 实例化进 `Sequence`，会**为所有项重新分配
//! `ItemId`**（避免与现有项冲突），并修复 parent/children 引用。

use std::collections::{HashMap, HashSet};

use anyhow::Result;
use aweft_model::{
    ids::ItemId,
    sequence::{ItemContent, ItemLocation, Sequence, StructureItem},
};
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ItemsClip {
    /// 顶层项（按文档顺序）。每个 root 在 `items` 中都有自洽的子树。
    pub roots: Vec<ItemId>,
    /// 子树内的所有项（含 roots 与所有后代）。
    pub items: HashMap<ItemId, StructureItem>,
}

impl ItemsClip {
    /// 从 Sequence 与用户选中集合构造 clip。
    ///
    /// 过滤掉"祖先也被选中"的项（避免重复打包整棵子树）；剩余的"顶层选中"
    /// 按文档顺序成为 `roots`，每个 root 的子树整体被克隆进 `items`。
    pub fn from_selection(sequence: &Sequence, selected: &HashSet<ItemId>) -> Self {
        let top_level: HashSet<ItemId> = selected
            .iter()
            .copied()
            .filter(|id| !sequence.has_ancestor_in_set(id, selected))
            .collect();

        let mut roots = Vec::new();
        let mut items = HashMap::new();
        walk_collect(
            sequence,
            &sequence.root_items,
            &top_level,
            &mut roots,
            &mut items,
        );

        Self { roots, items }
    }

    /// 将 clip 实例化进 sequence，**所有 root 作为 `anchor` 的兄弟/子项**
    /// （取决于 anchor 自身类型，由 `find_insert_location` 决定基准位置），
    /// 并按 `self.roots` 顺序连续排列。
    ///
    /// 设计取舍：之前用"链式 anchor"会导致——若用户复制了 [G1, G2] 两个分组
    /// 粘到事件 C 之后，G2 会被错放进 G1 内部。改为"以同一基准位置 + 索引
    /// 递增"，多个 root 永远是兄弟。
    ///
    /// 返回新分配的 root ItemId 列表（按 `self.roots` 的顺序）。
    pub fn instantiate(
        &self,
        sequence: &mut Sequence,
        anchor: ItemId,
    ) -> Result<(Vec<ItemId>, ItemLocation)> {
        // 1. 旧 → 新 ItemId 映射
        let id_map: HashMap<ItemId, ItemId> =
            self.items.keys().map(|&old| (old, ItemId::new())).collect();

        // 2. 克隆子树并 remap，批量装载到 sequence.items（暂未链接到树）
        for (&old_id, item) in &self.items {
            let new_id = id_map[&old_id];
            let mut cloned = item.clone();
            cloned.id = new_id;
            // 父项不在 clip 内时（即 root）→ None；在内时 → 新 ID
            cloned.parent = item.parent.and_then(|p| id_map.get(&p).copied());
            if let ItemContent::Group(ref mut g) = cloned.content {
                g.children = g
                    .children
                    .iter()
                    .filter_map(|c| id_map.get(c).copied())
                    .collect();
            }
            sequence.items.insert(new_id, cloned);
        }

        // 3. 计算基准位置
        let base_location = sequence.find_insert_location(anchor)?;

        let mut new_root_ids = Vec::with_capacity(self.roots.len());

        // 链接到树
        for (i, &old_root) in self.roots.iter().enumerate() {
            let new_root = id_map[&old_root];
            let location = match base_location {
                ItemLocation::Root(k) => ItemLocation::Root(k + i),
                ItemLocation::InGroup(pid, k) => ItemLocation::InGroup(pid, k + i),
            };
            sequence.attach_item(new_root, location)?;
            new_root_ids.push(new_root);
        }

        Ok((new_root_ids, base_location))
    }

    /// 内部统计：用于"已复制 N 项"提示。
    pub fn root_count(&self) -> usize {
        self.roots.len()
    }

    pub fn to_plain_text(&self) -> String {
        let mut out = String::new();
        for &root_id in &self.roots {
            append_plain_text(&self.items, root_id, &mut out);
        }
        out
    }
}

/// 按文档顺序遍历，把 `top_level` 中的项作为 root 收集，并克隆其子树。
/// 从根开始扩散收集位置与 ID，并且扁平化
fn walk_collect(
    sequence: &Sequence,
    ids: &[ItemId],
    top_level: &HashSet<ItemId>,
    roots: &mut Vec<ItemId>,
    items: &mut HashMap<ItemId, StructureItem>,
) {
    for &id in ids {
        if top_level.contains(&id) {
            roots.push(id);
            collect_subtree(sequence, id, items);
        } else if let Some(item) = sequence.get(id) {
            // 进入未被选中的分组继续向下找（而不是跳过）
            if let ItemContent::Group(g) = &item.content {
                walk_collect(sequence, &g.children, top_level, roots, items);
            }
        }
    }
}

fn collect_subtree(
    sequence: &Sequence,
    id: ItemId,
    items: &mut HashMap<ItemId, StructureItem>,
) {
    if let Some(item) = sequence.get(id) {
        items.insert(id, item.clone());
        if let ItemContent::Group(g) = &item.content {
            for &c in &g.children {
                collect_subtree(sequence, c, items);
            }
        }
    }
}

/// 把子树以"每个事件一行文本"的形式追加到 `out`。
fn append_plain_text(items: &HashMap<ItemId, StructureItem>, id: ItemId, out: &mut String) {
    let Some(item) = items.get(&id) else {
        return;
    };
    match &item.content {
        ItemContent::Event(e) => {
            out.push_str(&e.text);
            out.push('\n');
        }
        ItemContent::Group(g) => {
            for &c in &g.children {
                append_plain_text(items, c, out);
            }
        }
    }
}
