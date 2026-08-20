use std::{
    collections::{HashMap, HashSet},
    rc::Rc,
};

use anyhow::Result;
use aweft_model::{
    ids::ItemId,
    project::Project,
    sequence::{ItemContent, ItemLocation},
};
use chrono::TimeDelta;
use slint::{Model, ModelRc};

use crate::bridge::{SlintSelectionState, SlintSubtitleRow, SlintTimelineSubBlock};
use crate::bridge::types::SlintSelectionModifier;
use crate::store::utils::marquee::{clamp_rows, normalize_time};
use crate::store::utils::selection::{is_additive, is_replacing};
use crate::store::sequence_rows_model::{SequenceRowsModel, RowData};
use slint::{ToSharedString, VecModel};

type SubtitleFilterModel = slint::FilterModel<Rc<SequenceRowsModel>, Box<dyn Fn(&RowData) -> bool>>;

pub struct SequenceState {
    // 数据源模型（Slint Model，实时从权威 project 投影行）
    rows_model: SequenceRowsModel,
    filter_model: Option<Rc<SubtitleFilterModel>>,
    // 运行时折叠状态集合（共享引用，供 FilterModel 的闭包使用。索引到实际数据）
    collapsed_groups: Rc<std::cell::RefCell<HashSet<ItemId>>>,

    // 时间线根级收起时显示的"全部字幕行片段块"（派生自全量 Event 行，
    // 与分组收起显示的 child-blocks 共用同一渲染组件）
    timeline_root_blocks: Rc<VecModel<SlintTimelineSubBlock>>,

    // 选中的行（group 和 event 都包含）。直接选中态由 SequenceRowsModel.selected
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

impl SequenceState {
    pub fn new() -> Self {
        let rows_model = SequenceRowsModel::new();
        Self {
            collapsed_groups: rows_model.collapsed_groups(),
            rows_model,
            filter_model: None,
            timeline_root_blocks: Rc::new(VecModel::default()),
            all_highlighted_items: HashSet::new(),
            selected_event_counts: HashMap::new(),
            event_count_cache: HashMap::new(),
            last_selection_anchor: None,
        }
    }

    /// 暴露时间线根级块模型（派生自全量 Event 行）。
    pub fn timeline_root_blocks_model(&self) -> Rc<VecModel<SlintTimelineSubBlock>> {
        self.timeline_root_blocks.clone()
    }

    /// 从全量行（经 SequenceRowsModel + 权威 sequence）派生根级收起要显示的全部字幕行片段块。
    /// 所有会改变 Event 行集合或时间的入口都应调用，保持与权威数据同步。
    pub fn refresh_timeline_root_blocks(&self, project: &Project) {
        let blocks: Vec<SlintTimelineSubBlock> = (0..self.rows_model.row_count())
            .filter_map(|i| {
                let data = self.rows_model.row_data(i)?;
                if data.content.row_type != crate::bridge::SlintSubtitleRowType::Event {
                    return None;
                }
                // 仅有有效时间范围的块才渲染（与 Slint 侧 if 守卫一致）
                if data.content.end_sec <= data.content.start_sec {
                    return None;
                }
                // 从权威 sequence 取最新时间与文本（防止 content 快照滞后）
                // 但 content 已是投影构建时的快照，通常一致；此处直接用 content.
                Some(SlintTimelineSubBlock {
                    item_id: data.content.item_id.clone(),
                    name: data.content.text.clone(),
                    start_sec: data.content.start_sec,
                    end_sec: data.content.end_sec,
                })
            })
            .collect();
        // Fallback：若 rows_model 尚未填充（如 initialize_rows 前），从 sequence 直接派生
        let blocks = if blocks.is_empty() && self.rows_model.row_count() == 0 {
            // 直接遍历 sequence 的所有 Event
            let mut v = Vec::new();
            for (id, item) in project.sequence().items.iter() {
                if let ItemContent::Event(ev) = &item.content {
                    let s = ev.start_time.num_milliseconds() as f32 / 1000.0;
                    let e = ev.end_time.num_milliseconds() as f32 / 1000.0;
                    if e > s {
                        v.push(SlintTimelineSubBlock {
                            item_id: id.to_string().to_shared_string(),
                            name: ev.text.to_shared_string(),
                            start_sec: s,
                            end_sec: e,
                        });
                    }
                }
            }
            v
        } else {
            blocks
        };
        self.timeline_root_blocks.set_vec(blocks);
    }

    /// 按 item_id 局部更新 timeline_root_blocks 中的一条（set_row_data，保住 grab）。
    /// 若该 id 不在模型中或时间无效则不操作。
    pub fn update_timeline_root_block(&self, project: &Project, item_id: ItemId) {
        let Some(item) = project.sequence().get(item_id) else {
            return;
        };
        let ItemContent::Event(ev) = &item.content else {
            return;
        };
        let s = ev.start_time.num_milliseconds() as f32 / 1000.0;
        let e = ev.end_time.num_milliseconds() as f32 / 1000.0;
        let id_str = item_id.to_string();
        // 找到对应行
        let mut idx_opt = None;
        for i in 0..self.timeline_root_blocks.row_count() {
            if let Some(b) = self.timeline_root_blocks.row_data(i) {
                if b.item_id.as_str() == id_str {
                    idx_opt = Some(i);
                    break;
                }
            }
        }
        if let Some(idx) = idx_opt {
            if e > s {
                let block = SlintTimelineSubBlock {
                    item_id: id_str.to_shared_string(),
                    name: ev.text.to_shared_string(),
                    start_sec: s,
                    end_sec: e,
                };
                self.timeline_root_blocks.set_row_data(idx, block);
            } else {
                // 时间无效→移除该块（Slint 侧不渲染但模型仍占位会导致空白；直接移除）
                self.timeline_root_blocks.remove(idx);
            }
        } else if e > s {
            // 新 Event 且时间有效→追加（按扁平顺序插入更精确，此处简化追加）
            // 结构变化时已全量 refresh，此分支仅用于时间码从无效变有效
            let block = SlintTimelineSubBlock {
                item_id: id_str.to_shared_string(),
                name: ev.text.to_shared_string(),
                start_sec: s,
                end_sec: e,
            };
            self.timeline_root_blocks.push(block);
        }
    }

    /// 暴露数据源模型（实现 slint::Model）。
    pub fn rows_model(&self) -> &SequenceRowsModel {
        &self.rows_model
    }

    /// 创建过滤模型
    ///
    /// 顺序：`SequenceRowsModel(RowData) → FilterModel → MapModel(SlintSubtitleRow)`。
    /// 过滤在轻量 `RowData` 层做（`ancestor_ids` 是 ItemId，零 UUID parse），
    /// 被过滤掉的行完全不构造 `SlintSubtitleRow`。
    /// 过滤规则：
    /// - 根级别的行（无祖先）始终可见
    /// - 有祖先的行（包括子分组头和事件）只有在所有祖先都未折叠时才可见
    pub fn init_filter_model(&mut self) -> ModelRc<SlintSubtitleRow> {
        // 1. 数据源模型：产出轻量 RowData
        let rows_model: Rc<SequenceRowsModel> = Rc::new(self.rows_model.clone());
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
        let filter_model: Rc<slint::FilterModel<Rc<SequenceRowsModel>, Box<dyn Fn(&RowData) -> bool>>> =
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
                if let Some(item) = project.sequence().get(id) {
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
    /// 从 SequenceRowsModel.selected（三态单一来源）派生。
    pub fn directly_selected_items(&self) -> std::collections::HashSet<ItemId> {
        self.rows_model.directly_selected_items()
    }

    /// 初始化 UI 行数据（首次绑定模型）。
    ///
    /// 仅在初始化时调用。结构变化用精确锚点操作，不得走全量重建。
    pub fn initialize_rows(&mut self, project: &Project) {
        self.rows_model.set_sequence(&project.sequence);
        self.invalidate_event_counts();
        self.refresh_timeline_root_blocks(project);
    }

    // === 时间码更新 ===

    /// 更新单个事件行的时间码：重建该行 + 所有祖先分组头快照，并通知 Slint。
    /// 权威数据（project）已由 reducer 修改；投影 op 调用此方法做 UI 刷新。
    pub fn update_row_time(
        &mut self,
        project: &Project,
        event_id: ItemId,
        _is_start_time: bool,
        _time: TimeDelta,
    ) {
        self.rows_model
            .refresh_row_with_ancestors(&project.sequence, event_id);
        // 同步时间线的根级块（局部 set_row_data，保住 grab）
        self.update_timeline_root_block(project, event_id);
    }

    /// 批量更新多个事件行的时间码，并统一重建受影响行 + 所有祖先分组头。
    pub fn update_rows_batch_then_ancestors(
        &mut self,
        project: &Project,
        event_ids: &[ItemId],
        _is_start_time: bool,
        _times: &[TimeDelta],
    ) {
        let mut dirty: HashSet<ItemId> = HashSet::new();
        for &id in event_ids {
            dirty.insert(id);
            dirty.extend(self.rows_model.ancestors_of(id));
        }
        self.rows_model.refresh_ids(
            &project.sequence,
            &dirty.iter().copied().collect::<Vec<_>>(),
        );
        for &id in event_ids {
            self.update_timeline_root_block(project, id);
        }
    }

    /// 计算某个 item 的祖先分组链（叶子到根，不含自身）。
    /// 从行锚点的 `ancestor_ids` 读（投影时构建），无权威数据访问。
    fn ancestors_of(&self, item_id: ItemId) -> Vec<ItemId> {
        self.rows_model.ancestors_of(item_id)
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
            let Some(item) = project.sequence().get(item_id) else {
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

    /// 清空全部选中（空白点击）。
    pub fn deselect_all(&mut self, project: &Project) {
        let ids: Vec<ItemId> = self.all_highlighted_items.drain().collect();
        for id in ids {
            self.rows_model
                .set_selected(id, SlintSelectionState::Unselected);
        }
        // 组头 IndirectlySelected 由 rebuild/refresh 收敛
        self.rebuild_selected_event_counts();
        self.refresh_group_selected_by_counts(project);
        self.last_selection_anchor = None;
    }

    /// 时间线框选：按世界时间范围 [x_lo,x_hi] × 可见行区间 [row_lo,row_hi]
    /// 选中时间与行同时相交的行（事件/分组头）。可见行指 FilterModel 中
    /// 未被折叠隐藏的行，row 索引为过滤后的可见索引（Slint rows 下标）。
    /// modifier：None=替换，Shift=加选，Ctrl=切换。
    pub fn marquee_select(
        &mut self,
        project: &Project,
        x_lo: f32,
        x_hi: f32,
        row_lo: i32,
        row_hi: i32,
        modifier: SlintSelectionModifier,
    ) {
        let (t_lo, t_hi) = normalize_time(x_lo, x_hi);
        // 可见行：未被折叠分组隐藏的行（复用摄影表 row 求交语义，与 NLA/Dope 同源）
        let collapsed = self.collapsed_groups.borrow().clone();
        let mut visible: Vec<(usize, ItemId, f32, f32)> = Vec::new();
        for raw_idx in 0..self.rows_model.row_count() {
            let Some(row) = self.rows_model.row_data(raw_idx) else {
                continue;
            };
            if row.ancestor_ids.iter().any(|id| collapsed.contains(id)) {
                continue;
            }
            visible.push((
                raw_idx,
                row.item_id,
                row.content.start_sec,
                row.content.end_sec,
            ));
        }
        let n_vis = visible.len() as i32;
        let Some((r_lo, r_hi)) = clamp_rows(row_lo, row_hi, n_vis) else {
            if is_replacing(modifier) {
                self.deselect_all(project);
            }
            return;
        };
        let mut hits: Vec<ItemId> = Vec::new();
        for idx in r_lo as usize..=r_hi as usize {
            if let Some((_, item_id, start_sec, end_sec)) = visible.get(idx) {
                if *end_sec <= *start_sec {
                    continue;
                }
                if *end_sec < t_lo || *start_sec > t_hi {
                    continue;
                }
                hits.push(*item_id);
            }
        }
        if hits.is_empty() {
            if is_replacing(modifier) {
                self.deselect_all(project);
            }
            return;
        }
        if is_replacing(modifier) {
            let ids: Vec<ItemId> = self.all_highlighted_items.drain().collect();
            for id in ids {
                self.rows_model
                    .set_selected(id, SlintSelectionState::Unselected);
            }
            for &hit in &hits {
                self.add_select_hit(hit);
            }
        } else if is_additive(modifier) {
            for &hit in &hits {
                self.add_select_hit(hit);
            }
        } else {
            for &hit in &hits {
                self.toggle_select_hit(hit);
            }
        }
        self.rebuild_selected_event_counts();
        self.refresh_group_selected_by_counts(project);
        if let Some(&first) = hits.first() {
            if let Some(idx) = self.rows_model.row_index_of(first) {
                self.last_selection_anchor = Some(idx);
            }
        }
    }

    fn add_select_hit(&mut self, item_id: ItemId) {
        let Some(idx) = self.rows_model.row_index_of(item_id) else {
            return;
        };
        if self.rows_model.is_group_item(item_id) {
            let cnt = self.rows_model.group_row_count(item_id);
            for i in idx..idx + cnt {
                if let Some(id) = self.rows_model.item_id_at(i) {
                    self.rows_model
                        .set_selected(id, SlintSelectionState::DirectlySelected);
                    self.all_highlighted_items.insert(id);
                }
            }
        } else {
            self.rows_model
                .set_selected(item_id, SlintSelectionState::DirectlySelected);
            self.all_highlighted_items.insert(item_id);
        }
    }

    fn toggle_select_hit(&mut self, item_id: ItemId) {
        let is_selected = self.rows_model.is_selected(item_id);
        if is_selected {
            let Some(idx) = self.rows_model.row_index_of(item_id) else {
                return;
            };
            if self.rows_model.is_group_item(item_id) {
                let cnt = self.rows_model.group_row_count(item_id);
                for i in idx..idx + cnt {
                    if let Some(id) = self.rows_model.item_id_at(i) {
                        self.rows_model
                            .set_selected(id, SlintSelectionState::Unselected);
                        self.all_highlighted_items.remove(&id);
                    }
                }
            } else {
                self.rows_model
                    .set_selected(item_id, SlintSelectionState::Unselected);
                self.all_highlighted_items.remove(&item_id);
            }
        } else {
            self.add_select_hit(item_id);
        }
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
        let sequence = &*project.sequence();
        let count = match sequence.get(*item_id) {
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

    /// 结构变化后失效事件数缓存（投影 op_sequence_structure 入口调用）。
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
        project: &Project,
        item_id: ItemId,
        new_item_id: ItemId,
    ) -> Result<()> {
        // 精确插入锚点 + 构建新行内容快照（权威数据已由 reducer 更新）。
        self.rows_model
            .insert_after(&project.sequence, item_id, new_item_id);
        self.refresh_timeline_root_blocks(project);
        Ok(())
    }

    /// 处理已从 `project.sequence` 移除的项的 UI 投影更新（精确：移除锚点 + row_removed）。
    pub fn handle_items_removed(&mut self, project: &Project, item_ids: &[ItemId]) {
        // 事件数缓存失效由投影 op_sequence_structure 入口统一处理。
        for &id in item_ids {
            self.rows_model.remove_anchor(id);
            // 清除该行的拖拽预览覆盖（防止残留显示错误值）
            self.rows_model.clear_preview_timecode(id);
        }
        self.cleanup_selection();
        self.refresh_timeline_root_blocks(project);
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
        project: &Project,
        group_id: ItemId,
        _new_ids: &[ItemId],
    ) -> Result<()> {
        // 解散不改事件归属（事件数不变），但父组头需 notify_row_changed
        // 强制重算 total_row_count（含组头行数，aggregate_group 实时递归）。
        // 事件数缓存失效由投影 op_sequence_structure 入口统一处理。
        if let Some((idx, parent)) = self.rows_model.ungroup_anchor(&project.sequence, group_id) {
            // 更新父组头聚合（若有父组）。父组在 sequence 中仍在，可查。
            if let Some(parent) = parent {
                if let Some(pidx) = self.rows_model.row_index_of(parent) {
                    self.rows_model
                        .rebuild_row_content(pidx, &project.sequence);
                    self.rows_model.notify_row_changed(pidx);
                }
            }
            let _ = idx;
        }
        self.cleanup_selection();
        self.refresh_timeline_root_blocks(project);
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
        self.rows_model
            .insert_subtrees(&project.sequence, at, new_root_ids);
        // 事件数缓存失效由投影 op_sequence_structure 入口统一处理。
        if let Some(&first) = new_root_ids.first() {
            self.select_single(project, first);
        }
        self.refresh_timeline_root_blocks(project);
        Ok(())
    }

    /// 从 ItemLocation 计算扁平化插入索引（粘贴锚点）。
    fn compute_insert_at(&self, project: &Project, location: &ItemLocation) -> Result<usize> {
        use aweft_model::sequence::ItemContent as IC;
        let sequence = &*project.sequence();
        match location {
            ItemLocation::Root(k) => {
                // 第 k 个 root item 之后；若 k 超出 root_items 则插末尾
                if let Some(&prev) = sequence.root_items.get(*k) {
                    if let Some(idx) = self.rows_model.row_index_of(prev) {
                        let cnt = self.rows_model.group_row_count(prev);
                        return Ok(idx + cnt);
                    }
                }
                // 没有 prev（插在开头或空）——找第 k 个 root 的位置
                // 简化：若 k=0 插最前；否则找前一个 root
                if *k > 0 {
                    if let Some(&prev) = sequence.root_items.get(k - 1) {
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
                let Some(group) = sequence.get(*pid) else {
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
