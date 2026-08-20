use super::{NlaEditorState, NlaStripData, StripDrag};
use crate::depsgraph::{NlaStripMove, NlaStripRef, NlaStripReorder, Tag};

impl NlaEditorState {
    /// 片断拖拽。锁定轨道不可编辑。
    ///
    /// `delta_x` 为世界时间位移；`delta_row` 为垂直位移（行数，正 = 向下）。
    /// 垂直：**瞬间跳轨**——片段中心（按下轨道中线 + delta_row）越过轨道边界即把片段
    /// 立即移到目标轨道（按 start 排序插入），不做平滑跟随；目标无效（越界/全局
    /// Action 轨/锁定轨）则保持当前轨道。
    /// 水平：与当前所在轨道其他片断**硬钳制**——重叠时按候选中点相对片断中点停到
    /// 较近的一侧（可穿越）；若两片断间空隙放不下，则不塞入（保持片段当前位置）。
    /// 拖动中保持模型按 start 排序（否则穿越后延伸条带/空隙归属错乱）。
    pub fn strip_dragged(
        &mut self,
        track: usize,
        strip: usize,
        delta_x: f32,
        delta_row: f32,
    ) -> Vec<Tag> {
        if !delta_x.is_finite() || !delta_row.is_finite() {
            return Vec::new();
        }
        let Some(mut d) = self.drag else {
            return Vec::new();
        };
        if d.press_track != track || d.press_strip != strip {
            return Vec::new();
        }
        if self
            .track_view
            .get(d.track)
            .map(|t| t.locked)
            .unwrap_or(false)
        {
            return Vec::new();
        }
        let width = d.orig_end - d.orig_start;
        let mut updated = Vec::new();
        let mut moved = Vec::new();
        let mut reordered = Vec::new();

        // 垂直：目标轨道（基于按下轨道 + 绝对位移）；不同且有效 → 立即跨轨（按 start 插入）
        let target = self.drag_target_row(d.press_track, delta_row);
        if target != d.track && self.can_move_to(target) {
            // 源下标用 remove 前的 d.strip（拖动中 reinsert 后可能与按下下标不同），
            // 投影按同一下标 remove 派生层——错用 press 下标会移错片断导致两层错乱
            let src_idx = d.strip;
            let src_list = &mut self.editing_strips[d.track];
            if d.strip < src_list.len() {
                let s = src_list.remove(d.strip);
                let dst_list = &mut self.editing_strips[target];
                let start = s.view.start;
                let idx = dst_list
                    .iter()
                    .position(|x| start < x.view.start)
                    .unwrap_or(dst_list.len());
                dst_list.insert(idx, s);
                d.strip = idx;
                moved.push(NlaStripMove {
                    src_track: d.track,
                    src_index: src_idx,
                    dst_track: target,
                });
                d.track = target;
            }
        }

        // 水平：钳制到当前所在轨道的空隙（fallback = 片段当前实际位置，空隙不够时不塞入）
        let fallback = self
            .editing_strips
            .get(d.track)
            .and_then(|l| l.get(d.strip))
            .map(|e| e.view.start)
            .unwrap_or(d.orig_start + delta_x);
        let raw_start = d.orig_start + delta_x;
        let start = self.clamp_no_overlap(d.track, d.strip, raw_start, width, fallback);
        if let Some(e) = self
            .editing_strips
            .get_mut(d.track)
            .and_then(|l| l.get_mut(d.strip))
        {
            e.view.start = start;
            e.view.end = start + width;
            e.view.dragging = true;
        }
        // 保持模型按 start 排序（穿越后延伸/空隙按时间顺序计算）。
        // 数据层重排必须通知投影同步 remove+insert，否则派生层旧位置残留幽灵片断
        let old_idx = d.strip;
        d.strip = self.reinsert_sorted(d.track, d.strip);
        if d.strip != old_idx {
            reordered.push(NlaStripReorder {
                track: d.track,
                old_index: old_idx,
                new_index: d.strip,
            });
        }
        updated.push(NlaStripRef {
            track: d.track,
            index: d.strip,
        });
        self.drag = Some(d);
        let mut tags = Vec::new();
        if !moved.is_empty() || !reordered.is_empty() {
            tags.push(Tag::NlaStripStructureChanged {
                moved,
                reordered,
                inserted: Vec::new(),
            });
        }
        if !updated.is_empty() {
            tags.push(Tag::NlaStripPositionChanged { updated });
        }
        tags
    }

    /// 垂直位移后片段中心所在的轨道（行号）：中心 = 按下轨道中线 + delta_row，
    /// 片段中心越过轨道边界（|delta_row| > 0.5）即进入相邻轨道；越界钳到两端。
    fn drag_target_row(&self, press_row: usize, delta_row: f32) -> usize {
        let center = press_row as f32 + 0.5 + delta_row;
        let target = center.floor().max(0.0) as usize;
        target.min(self.track_view.len().saturating_sub(1))
    }

    /// 把候选 [start, start+width] 钳制到 `track` 轨道的空隙内（不与其他片断重叠）。
    /// 与片断重叠时按**候选中点相对片断中点**停到较近的一侧（可穿越到片断另一侧）；
    /// 但若两片断之间的空隙放不下候选（迭代钳制后仍与某片断重叠），则**不塞入**，
    /// 回退到 `fallback`（片段保持当前位置）。
    fn clamp_no_overlap(
        &self,
        track: usize,
        ignore_strip: usize,
        start: f32,
        width: f32,
        fallback: f32,
    ) -> f32 {
        let Some(list) = self.editing_strips.get(track) else {
            return start;
        };
        let mut s = start;
        for _ in 0..list.len() + 1 {
            let mut moved = false;
            for (j, o) in list.iter().enumerate() {
                if j == ignore_strip {
                    continue;
                }
                let o = &o.view;
                if s < o.end && s + width > o.start {
                    let cand_mid = s + width / 2.0;
                    let o_mid = (o.start + o.end) / 2.0;
                    if cand_mid < o_mid {
                        s = o.start - width;
                    } else {
                        s = o.end;
                    }
                    moved = true;
                    break;
                }
            }
            if !moved {
                break;
            }
        }
        // 空隙不够（迭代后仍与片断重叠）→ 不塞入，回退到片段当前位置
        for (j, o) in list.iter().enumerate() {
            if j == ignore_strip {
                continue;
            }
            let o = &o.view;
            if s < o.end && s + width > o.start {
                return fallback;
            }
        }
        s
    }

    /// 把 `track` 轨道的 `old_idx` 片断取出后按 start 重新插入（数据层），返回新下标。
    fn reinsert_sorted(&mut self, track: usize, old_idx: usize) -> usize {
        let list = &mut self.editing_strips[track];
        let s = list.remove(old_idx);
        let start = s.view.start;
        let idx = list
            .iter()
            .position(|x| start < x.view.start)
            .unwrap_or(list.len());
        list.insert(idx, s);
        idx
    }

    /// 片断松开：跨轨道已在拖动中瞬间完成，这里只退出"移动中"高亮并清空拖拽状态。
    /// `track/strip` 为按下时回传的编码，按 `press_track/press_strip` 匹配。
    /// model 同步（commit_to_model）由 reducer 在投影前完成。
    pub fn strip_released(&mut self, track: usize, strip: usize) -> Vec<Tag> {
        let Some(d) = self.drag else {
            return Vec::new();
        };
        if d.press_track != track || d.press_strip != strip {
            return Vec::new();
        }
        if let Some(e) = self
            .editing_strips
            .get_mut(d.track)
            .and_then(|l| l.get_mut(d.strip))
        {
            e.view.dragging = false;
        }
        self.drag = None;
        vec![Tag::NlaStripPositionChanged {
            updated: vec![NlaStripRef {
                track: d.track,
                index: d.strip,
            }],
        }]
    }
}
