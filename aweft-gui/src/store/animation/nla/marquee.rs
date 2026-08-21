use super::NlaEditorState;
use crate::bridge::types::SlintSelectionModifier;
use crate::depsgraph::{NlaStripRef, Tag};
use crate::store::utils::marquee::{clamp_rows, normalize_time};
use crate::store::utils::selection::{is_replacing, is_toggling};
use slint::Model;

impl NlaEditorState {
    /// NLA 画布框选：按世界时间范围 [x_lo,x_hi] × 过滤轨道区间 [row_lo,row_hi]
    /// 选中时间与轨道同时相交的片断（交集语义）。跳过全局 Action 轨与锁定轨。
    /// modifier：None=替换，Shift=加选，Ctrl=切换。
    pub fn marquee_select(
        &mut self,
        x_lo: f32,
        x_hi: f32,
        row_lo: i32,
        row_hi: i32,
        modifier: SlintSelectionModifier,
    ) -> Vec<Tag> {
        let n = self.tracks_filter.row_count() as i32;
        let (t_lo, t_hi) = normalize_time(x_lo, x_hi);
        let Some((r_lo, r_hi)) = clamp_rows(row_lo, row_hi, n) else {
            if is_replacing(modifier) {
                return self.deselect_all();
            }
            return Vec::new();
        };

        let replacing = is_replacing(modifier);
        let toggling = is_toggling(modifier);

        let mut hits: Vec<NlaStripRef> = Vec::new();
        for f_row in r_lo as usize..=r_hi as usize {
            let Some(track) = self.tracks_filter.row_data(f_row) else {
                continue;
            };
            let auth = track.orig_index as usize;
            if self
                .track_view
                .get(auth)
                .map(|t| t.is_action || t.locked)
                .unwrap_or(true)
            {
                continue;
            }
            let Some(list) = self.editing_strips.get(auth) else {
                continue;
            };
            for (j, e) in list.iter().enumerate() {
                // 片断时间范围与框选时间相交即命中（片断为区间，不是点）
                if e.view.end < t_lo || e.view.start > t_hi {
                    continue;
                }
                if e.view.end <= e.view.start {
                    continue;
                }
                hits.push(NlaStripRef {
                    id: e.id,
                    track: auth,
                    index: j,
                });
            }
        }

        if hits.is_empty() {
            if replacing {
                return self.deselect_all();
            }
            return Vec::new();
        }

        // 按修饰键更新选中态（数据层）
        if replacing {
            for list in &mut self.editing_strips {
                for e in list.iter_mut() {
                    e.view.selected = false;
                }
            }
            for h in &hits {
                if let Some(e) = self
                    .editing_strips
                    .get_mut(h.track)
                    .and_then(|l| l.get_mut(h.index))
                {
                    e.view.selected = true;
                }
            }
        } else if toggling {
            for h in &hits {
                if let Some(e) = self
                    .editing_strips
                    .get_mut(h.track)
                    .and_then(|l| l.get_mut(h.index))
                {
                    e.view.selected = !e.view.selected;
                }
            }
        } else {
            // Shift 加选
            for h in &hits {
                if let Some(e) = self
                    .editing_strips
                    .get_mut(h.track)
                    .and_then(|l| l.get_mut(h.index))
                {
                    e.view.selected = true;
                }
            }
        }

        vec![Tag::NlaSelectionChanged {
            updated: hits,
            deselect_all: replacing,
        }]
    }
}
