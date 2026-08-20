use super::NlaEditorState;
use crate::bridge::types::{SlintNlaStrip, SlintNlaTrack};
use crate::depsgraph::Tag;
use slint::{Model, ModelRc, VecModel};
use std::rc::Rc;

impl NlaEditorState {
    /// 结构变化投影：重建派生层（strip_models/tracks/过滤视图）。全量重建仅用于
    /// 装载与搜索过滤视图重建；轨道/片断增删走对应的 row_added/remove 局部操作。
    pub(crate) fn apply_structure(&mut self) {
        self.strip_models = self
            .editing_strips
            .iter()
            .map(|list| {
                Rc::new(VecModel::from(
                    list.iter().map(|e| e.view.clone()).collect::<Vec<_>>(),
                ))
            })
            .collect();
        let rows: Vec<SlintNlaTrack> = self
            .track_view
            .iter()
            .enumerate()
            .map(|(i, t)| {
                let mut t = t.clone();
                t.strips = ModelRc::from(self.strip_models[i].clone());
                t
            })
            .collect();
        self.tracks.set_vec(rows);
        self.refresh_filter();
    }

    /// 片断选中变化投影：只更新选中态（`set_row_data` 选中列），不触碰位置/结构。
    pub(crate) fn apply_selection(&mut self, tags: &[Tag]) {
        for tag in tags {
            let Tag::NlaSelectionChanged {
                updated,
                deselect_all,
            } = tag
            else {
                continue;
            };
            if *deselect_all {
                for model in &self.strip_models {
                    for i in 0..model.row_count() {
                        if let Some(mut s) = model.row_data(i) {
                            if s.selected {
                                s.selected = false;
                                model.set_row_data(i, s);
                            }
                        }
                    }
                }
            }
            for update in updated {
                let t = update.track;
                let i = update.index;
                let Some(view) = self
                    .editing_strips
                    .get(t)
                    .and_then(|l| l.get(i))
                    .map(|e| e.view.clone())
                else {
                    continue;
                };
                if let Some(model) = self.strip_models.get(t) {
                    if i < model.row_count() {
                        model.set_row_data(i, view);
                    }
                }
            }
        }
    }

    /// 片断位置值变化投影：就地 `set_row_data`（保住 grab）。
    pub(crate) fn apply_position(&mut self, tags: &[Tag]) {
        for tag in tags {
            let Tag::NlaStripPositionChanged { updated } = tag else {
                continue;
            };
            for update in updated {
                let t = update.track;
                let i = update.index;
                let Some(view) = self
                    .editing_strips
                    .get(t)
                    .and_then(|l| l.get(i))
                    .map(|e| e.view.clone())
                else {
                    continue;
                };
                if let Some(model) = self.strip_models.get(t) {
                    if i < model.row_count() {
                        model.set_row_data(i, view);
                    }
                }
            }
        }
    }

    /// 片断结构变化投影：精确 remove+insert（跨轨移动/同轨重排/插入），存量复用。
    /// 拆为三子函数各司其一（SRP），由依赖图分别保证顺序（结构→位置）。
    pub(crate) fn apply_strip_structure(&mut self, tags: &[Tag]) {
        for tag in tags {
            let Tag::NlaStripStructureChanged {
                moved,
                reordered,
                inserted,
            } = tag
            else {
                continue;
            };
            self.apply_moved(moved);
            self.apply_reordered(reordered);
            self.apply_inserted(inserted);
        }
    }

    fn apply_moved(&mut self, moved: &[crate::depsgraph::NlaStripMove]) {
        for move_info in moved {
            let src = move_info.src_track;
            let idx = move_info.src_index;
            let dst = move_info.dst_track;
            if let Some(sm) = self.strip_models.get(src) {
                if idx < sm.row_count() {
                    let s = sm.remove(idx);
                    if let Some(dm) = self.strip_models.get(dst) {
                        Self::insert_sorted(&dm, s);
                    }
                }
            }
        }
    }

    fn apply_reordered(&mut self, reordered: &[crate::depsgraph::NlaStripReorder]) {
        // 同轨重排：remove 旧下标 + insert 新下标（新下标为 remove 后列表的下标，
        // 与 reducer 的 reinsert_sorted 同规则）。须先于 position 更新（position 用新下标）。
        for reorder in reordered {
            if let Some(model) = self.strip_models.get(reorder.track) {
                let old_idx = reorder.old_index;
                if old_idx < model.row_count() {
                    let s = model.remove(old_idx);
                    let new_idx = reorder.new_index.min(model.row_count());
                    model.insert(new_idx, s);
                }
            }
        }
    }

    fn apply_inserted(&mut self, inserted: &[crate::depsgraph::NlaStripInsert]) {
        // 新增片断（向下插入到已有轨道）：row_added 局部插入，存量复用
        for insert in inserted {
            let t = insert.track;
            let i = insert.index;
            let Some(view) = self
                .editing_strips
                .get(t)
                .and_then(|l| l.get(i))
                .map(|e| e.view.clone())
            else {
                continue;
            };
            if let Some(model) = self.strip_models.get(t) {
                let i = i.min(model.row_count());
                model.insert(i, view);
            }
        }
    }

    /// 轨道属性行更新投影（启用/锁定）：只改对应行，不重建存量。
    pub(crate) fn apply_track_properties(&mut self, tags: &[Tag]) {
        for tag in tags {
            let Tag::NlaTrackChanged { track } = tag else {
                continue;
            };
            let Some(mut view) = self.track_view.get(*track).cloned() else {
                continue;
            };
            // `track_view.strips` 是数据层占位字段；行级更新必须保留派生层
            // 已绑定的 inner 模型，否则切换启用/锁定会断开片断 Repeater。
            if let Some(inner) = self.strip_models.get(*track) {
                view.strips = ModelRc::from(inner.clone());
            }
            if *track < self.tracks.row_count() {
                self.tracks.set_row_data(*track, view.clone());
            }
            for i in 0..self.tracks_filter.row_count() {
                if let Some(t) = self.tracks_filter.row_data(i) {
                    if t.orig_index as usize == *track {
                        self.tracks_filter.set_row_data(i, view);
                        break;
                    }
                }
            }
        }
    }

    /// 轨道结构新增投影：追加轨道行与新片断 inner 模型，复用既有行。
    pub(crate) fn apply_track_added(&mut self, tags: &[Tag]) {
        for tag in tags {
            let Tag::NlaTrackAdded { track } = tag else {
                continue;
            };
            let Some(view) = self.track_view.get(*track).cloned() else {
                continue;
            };
            // 新片断模型：数据层 editing_strips[track] 已就绪（含下插片断）
            let inner: Rc<VecModel<SlintNlaStrip>> = Rc::new(VecModel::from(
                self.editing_strips
                    .get(*track)
                    .map(|list| list.iter().map(|e| e.view.clone()).collect::<Vec<_>>())
                    .unwrap_or_default(),
            ));
            if *track < self.strip_models.len() {
                // 极端情况下已有 inner 时复用对应位置；正常路径只能追加。
                self.strip_models[*track] = Rc::clone(&inner);
            } else {
                self.strip_models.push(Rc::clone(&inner));
            }
            // tracks 追加行（row_added）；新轨总是 append，orig_index 不变
            let mut row = view.clone();
            row.strips = ModelRc::from(inner);
            self.tracks.push(row.clone());
            // 过滤视图：按搜索词是否匹配插入对应位置（不匹配则不插入，行数不变）
            let orig = *track as i32;
            let mut idx = self.tracks_filter.row_count();
            for i in 0..self.tracks_filter.row_count() {
                if let Some(t) = self.tracks_filter.row_data(i) {
                    if t.orig_index > orig {
                        idx = i;
                        break;
                    }
                }
            }
            if self.search.is_empty() || view.name.contains(&self.search) {
                self.tracks_filter.insert(idx, row);
            }
        }
    }

    /// 把 `strip` 按 start 升序插入 `model`（派生层），返回插入下标。
    fn insert_sorted(model: &Rc<VecModel<SlintNlaStrip>>, strip: SlintNlaStrip) -> usize {
        let mut idx = model.row_count();
        for i in 0..model.row_count() {
            if let Some(x) = model.row_data(i) {
                if strip.start < x.start {
                    idx = i;
                    break;
                }
            }
        }
        model.insert(idx, strip);
        idx
    }

    /// 由权威轨道 + 搜索关键字重建过滤投影（派生层）。
    pub(crate) fn refresh_filter(&mut self) {
        let mut v: Vec<SlintNlaTrack> = Vec::new();
        for i in 0..self.tracks.row_count() {
            if let Some(t) = self.tracks.row_data(i) {
                if self.search.is_empty() || t.name.contains(&self.search) {
                    v.push(t);
                }
            }
        }
        self.tracks_filter.set_vec(v);
    }
}
