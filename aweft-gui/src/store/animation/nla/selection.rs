use super::{NlaEditorState, StripDrag};
use crate::bridge::types::SlintSelectionModifier;
use crate::depsgraph::{NlaStripRef, Tag};
use crate::store::utils::selection::{is_replacing, is_toggling};
use slint::Model;

impl NlaEditorState {
    /// 清除全部片断选中。
    pub fn deselect_all(&mut self) -> Vec<Tag> {
        for list in &mut self.editing_strips {
            for e in list.iter_mut() {
                e.view.selected = false;
            }
        }
        vec![Tag::NlaSelectionChanged {
            updated: Vec::new(),
            deselect_all: true,
        }]
    }

    /// 片断按下：选中并记录拖拽起点。语义对齐曲线编辑器 `keyframe_pressed`：
    /// - `None` 点已选中 → 保持多选（拖动多选时一起移动）；
    /// - `None` 点未选中 → 清空后单选；
    /// - `Shift/Ctrl` → 追加切换（当前 NLA 两者均切换，复用全局 `is_toggling`/`is_replacing`）。
    pub fn strip_pressed(
        &mut self,
        track: usize,
        strip: usize,
        modifier: SlintSelectionModifier,
    ) -> Vec<Tag> {
        let Some(s) = self
            .editing_strips
            .get(track)
            .and_then(|l| l.get(strip))
            .map(|e| e.view.clone())
        else {
            return Vec::new();
        };
        let already_selected = self
            .editing_strips
            .get(track)
            .and_then(|l| l.get(strip))
            .map(|e| e.view.selected)
            .unwrap_or(false);
        if is_replacing(modifier) {
            if already_selected {
                // 保持多选，不清除（对齐 Graph 多选拖动）
            } else {
                for list in &mut self.editing_strips {
                    for e in list.iter_mut() {
                        e.view.selected = false;
                    }
                }
                if let Some(e) = self
                    .editing_strips
                    .get_mut(track)
                    .and_then(|l| l.get_mut(strip))
                {
                    e.view.selected = true;
                }
            }
        } else {
            // Shift/Ctrl 追加：切换该片断（NLA 当前两者均切换）
            if let Some(e) = self
                .editing_strips
                .get_mut(track)
                .and_then(|l| l.get_mut(strip))
            {
                // 区分 Shift 加选 vs Ctrl 切换：Shift 仅加选不取消，Ctrl 切换
                if is_toggling(modifier) {
                    e.view.selected = !already_selected;
                } else {
                    e.view.selected = true;
                }
            }
        }
        let deselect_all = if is_replacing(modifier) {
            !already_selected
        } else {
            false
        };
        let mut snapshot = Vec::new();
        for (ti, list) in self.editing_strips.iter().enumerate() {
            for (idx, e) in list.iter().enumerate() {
                if e.view.selected {
                    snapshot.push((ti, idx, e.view.start, e.view.end - e.view.start));
                }
            }
        }
        self.drag = Some(StripDrag {
            press_track: track,
            press_strip: strip,
            track,
            strip,
            orig_start: s.start,
            orig_end: s.end,
            prev_delta_x: 0.0,
            selected_snapshot: snapshot,
        });
        vec![Tag::NlaSelectionChanged {
            updated: vec![NlaStripRef {
                track,
                index: strip,
            }],
            deselect_all,
        }]
    }

    /// 画布级纯命中检测：世界时间 x + 过滤视图行号 → 权威轨道/片断下标（只读，不改数据层）。
    /// Slint 按 (y-pan-y)/row-height 换算行号。全局 Action 轨/锁定轨命中直接返回 None。
    pub fn strip_hit_test(&self, x: f32, row: i32) -> Option<(usize, usize)> {
        if row < 0 {
            return None;
        }
        let t = self.tracks_filter.row_data(row as usize)?;
        let auth = t.orig_index as usize;
        // 只读性从数据层读：投影未同步时过滤视图状态可能陈旧
        let list = self.editing_strips.get(auth)?;
        if self
            .track_view
            .get(auth)
            .map(|t| t.is_action || t.locked)
            .unwrap_or(true)
        {
            return None;
        }
        for (j, e) in list.iter().enumerate() {
            if x >= e.view.start && x <= e.view.end {
                return Some((auth, j));
            }
        }
        None
    }

    /// 画布级 TouchArea 左键命中检测：命中则选中并记录拖拽起点（经 reducer 路径）。
    /// 返回编码 auth_track*10000+strip（命中）或 -1（未命中），并返回需要投影的 tag。
    /// 新代码优先使用 `strip_hit_test` + `Action::Animation(AnimationAction::Nla(NlaAction::StripPressed))` dispatch；
    /// 本方法仅保留给单测兼容（同样走 strip_pressed → 数据层改动）。
    pub fn strip_pick(
        &mut self,
        x: f32,
        row: i32,
        modifier: SlintSelectionModifier,
    ) -> (i32, Vec<Tag>) {
        let Some((auth, j)) = self.strip_hit_test(x, row) else {
            return (-1, Vec::new());
        };
        let tags = self.strip_pressed(auth, j, modifier);
        (auth as i32 * 10000 + j as i32, tags)
    }
}
