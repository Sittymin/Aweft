//! 关键帧交互（reducer 侧）：只改数据层（`editing_keyframes` selected/time/value、
//! 选中集）并返回 tag；曲线段/手柄/汇总/显示掩码由投影（projection.rs）完成。

use super::{ms_of, snap_ms, CurveEditorState};
use crate::bridge::types::{
    SlintBezierHandle, SlintInterpolation, SlintKeyframe,
    SlintKeyframeDragMode, SlintSelectionModifier,
};
use crate::depsgraph::Tag;

impl CurveEditorState {
    /// 关键帧按下：none=单选，shift=同通道区间扩展，ctrl=切换。锁定通道不可交互。
    ///
    /// 选中语义（对齐 Blender，避免拖拽反馈问题）：
    ///  - 普通点击**未选中**的关键帧：清空选择，单选它；
    ///  - 普通点击**已选中**的关键帧：保持现有多选（拖拽时所有选中一起移动）；
    ///  - Shift：从同通道锚点扩展区间；Ctrl：切换该关键帧。
    ///
    /// 注意：通道（`channels_data`）的选中状态在这里只记录到 `pending_channel_selection`，
    /// 等 `keyframe_released`（grab 结束后）才写回。原因是关键帧标记渲染在
    /// `for ch in channels: for kf in ch.keyframes` 的嵌套循环里，按下期间对外层
    /// channels 模型 `set_row_data` 会破坏 Slint 的拖拽抓取（见 tests/dope_drag.rs）。
    pub fn keyframe_pressed(&mut self, curve: usize, key: usize, modifier: SlintSelectionModifier) -> Vec<Tag> {
        if !self.channel_editable(curve) {
            return Vec::new();
        }
        self.pressed_modifier = modifier;
        let already_selected = self
            .editing_keyframes
            .get(&curve)
            .and_then(|kfs| kfs.get(key))
            .map(|kf| kf.selected)
            .unwrap_or(false);
        // 找附近重合的关键帧（用于单击循环选择）
        let coincident = self.find_coincident_keyframes(curve, key);
        // pressed_was_selected = 自身已选中 或 附近有其他已选中的点
        let has_other_selected = coincident.iter().any(|&(c, j)| {
            (c != curve || j != key)
                && self.editing_keyframes.get(&c).and_then(|kfs| kfs.get(j)).map(|k| k.selected).unwrap_or(false)
        });
        self.pressed_was_selected = already_selected || has_other_selected;
        let mut summary_ms = std::collections::HashSet::new();

        match modifier {
            SlintSelectionModifier::Shift => {
                let anchor = self.keyframe_selection_anchor.unwrap_or((curve, key));
                let (lo, hi) = if anchor.0 == curve {
                    (anchor.1.min(key), anchor.1.max(key))
                } else {
                    (key, key)
                };
                if anchor.0 != curve {
                    self.keyframe_selection_anchor = Some((curve, key));
                }
                for index in lo..=hi {
                    if let Some(ms) = self.set_keyframe_selected(curve, index, false) {
                        summary_ms.insert(ms);
                    }
                }
                let mut sel = self.effective_selected_channels();
                if !sel.contains(&curve) {
                    sel.push(curve);
                }
                self.pending_channel_selection = sel;
            }
            SlintSelectionModifier::Ctrl => {
                if let Some(ms) = self.set_keyframe_selected(curve, key, already_selected) {
                    summary_ms.insert(ms);
                }
                // Ctrl 切换后：从全量选择集重建 pending，移除已无选中关键帧的通道。
                // 兜底：被按通道若刚被选中但尚未在 effective 集合中，手动加入。
                let mut pending: Vec<usize> = self.effective_selected_channels()
                    .into_iter()
                    .filter(|&c| self.channel_has_selected_keyframe(c))
                    .collect();
                if self.channel_has_selected_keyframe(curve) && !pending.contains(&curve) {
                    pending.push(curve);
                }
                self.pending_channel_selection = pending;
            }
            SlintSelectionModifier::None => {
                self.keyframe_selection_anchor = Some((curve, key));
                if self.pressed_was_selected {
                    self.pending_channel_selection = self.effective_selected_channels();
                } else {
                    self.pending_channel_selection = vec![curve];
                    self.selection_anchor = curve as isize;
                    for ms in self.deselect_all() {
                        summary_ms.insert(ms);
                    }
                    if let Some(ms) = self.set_keyframe_selected(curve, key, false) {
                        summary_ms.insert(ms);
                    }
                }
            }
        }
        self.current_channel = curve as isize;
        vec![Tag::AnimationSelectionChanged {
            summary_ms: summary_ms.into_iter().collect(),
        }]
    }

    /// 找出被按关键帧附近（像素容差内）的全部关键帧（含自身，含其它通道）。
    fn find_coincident_keyframes(&self, curve: usize, key: usize) -> Vec<(usize, usize)> {
        let Some(kfs) = self.editing_keyframes.get(&curve) else {
            return Vec::new();
        };
        let Some(target) = kfs.get(key) else {
            return Vec::new();
        };
        let eps_t = 5.0 / self.time_zoom.max(0.01);
        let eps_v = 5.0 / self.value_zoom.max(0.01);
        let mut list = Vec::new();
        for (&c, kfs) in self.editing_keyframes.iter() {
            if !self.channels_data.get(c).map(|ch| ch.visible).unwrap_or(false) {
                continue;
            }
            for (j, kf) in kfs.iter().enumerate() {
                if (kf.time - target.time).abs() <= eps_t
                    && (kf.value - target.value).abs() <= eps_v
                {
                    list.push((c, j));
                }
            }
        }
        list.sort_unstable();
        list
    }

    /// 单击已选中的关键帧：循环选中几乎重合的若干点（Blender 行为）。
    /// 收集被按位置附近的全部关键帧，取"当前选中"的下一个；只有一个时保持自身。
    fn cycle_coincident(&mut self, coincident: &[(usize, usize)]) -> Vec<i64> {
        if coincident.is_empty() {
            return Vec::new();
        }
        // 当前选中者（在重合点中）的下一个；无选中则选第一个
        let cur = coincident
            .iter()
            .position(|&(c, j)| self.editing_keyframes.get(&c).and_then(|kfs| kfs.get(j)).map(|k| k.selected).unwrap_or(false));
        let next_idx = match cur {
            Some(i) => (i + 1) % coincident.len(),
            None => 0,
        };
        let (nc, nj) = coincident[next_idx];
        self.pending_channel_selection = vec![nc];
        self.selection_anchor = nc as isize;
        let mut summary_ms = self.deselect_all();
        self.selected_handles.clear();
        self.current_channel = nc as isize;
        if let Some(ms) = self.set_keyframe_selected(nc, nj, false) {
            summary_ms.push(ms);
        }
        summary_ms
    }

    /// 某通道是否有至少一个选中的关键帧。
    fn channel_has_selected_keyframe(&self, curve: usize) -> bool {
        self.editing_keyframes
            .get(&curve)
            .map(|kfs| kfs.iter().any(|k| k.selected))
            .unwrap_or(false)
    }

    /// 关键帧拖拽。mode：
    ///   - 0：把增量累加到**所有选中**关键帧（含被拖的这个，按下时已被选中）。
    ///     增量由 Slint 按"光标相对按下位置的位移 / 缩放"计算（与节点拖拽一致），
    ///     累加（而非以按下点为基准重算）可避免标记与光标之间的反馈振荡。
    ///   - 1（Shift）：从贝塞尔关键帧"拖出手柄"（关键帧本体不动，手柄跟随光标），
    ///     避免手柄过短与关键帧重合而无法抓取。
    pub fn keyframe_dragged(
        &mut self,
        curve: usize,
        key: usize,
        delta_time: f32,
        delta_value: f32,
        mode: SlintKeyframeDragMode,
    ) -> Vec<Tag> {
        if !self.channel_editable(curve) {
            return Vec::new();
        }
        if !delta_time.is_finite() || !delta_value.is_finite() {
            return Vec::new();
        }
        if mode == SlintKeyframeDragMode::DragOutHandle {
            return self.drag_out_handle(curve, key, delta_time, delta_value);
        }
        // 收集被移动的关键帧下标 + 记录旧 ms（汇总增量更新用）
        let mut updates: Vec<(usize, usize, SlintKeyframe)> = Vec::new();
        let mut moved: Vec<usize> = Vec::new();
        let mut affected_ms: std::collections::HashSet<i64> = std::collections::HashSet::new();
        for (&c, kfs) in self.editing_keyframes.iter() {
            if !self.channel_editable(c) {
                continue;
            }
            let mut idxs = Vec::new();
            for (j, kf) in kfs.iter().enumerate() {
                if kf.selected {
                    let old_ms = ms_of(kf.time);
                    let mut kf2 = kf.clone();
                    kf2.time = snap_ms(kf2.time + delta_time);
                    kf2.value += delta_value;
                    let new_ms = ms_of(kf2.time);
                    updates.push((c, j, kf2));
                    affected_ms.insert(old_ms);
                    affected_ms.insert(new_ms);
                    idxs.push(j);
                }
            }
            if !idxs.is_empty() {
                moved.push(c);
            }
        }
        if moved.is_empty() {
            return Vec::new();
        }
        for (c, j, kf2) in updates {
            if let Some(kfs) = self.editing_keyframes.get_mut(&c) {
                kfs[j] = kf2;
            }
        }
        // 被拖通道的手柄软钳制预览（投影据此替换 handles 区间）
        let dragged_curve = moved.iter().any(|&c| c == curve).then_some(curve);
        vec![Tag::AnimationKeyframesChanged {
            curves: moved,
            summary_ms: affected_ms.into_iter().collect(),
            dragged_curve,
            dragged_summary: None,
            handle_preview: None,
        }]
    }

    /// Shift+拖拽关键帧"拖出手柄"：关键帧本体不动，出柄/入柄按**原点对称**调节
    /// （入柄 = -出柄），与 Shift 拖手柄的原点对称模式一致，而不是只调半边。
    /// 关键帧元素在拖出手柄模式下不移动，KeyframeMarker 上报的 dt/dv 即为
    /// 光标相对关键帧的累计偏移（世界坐标），直接作为手柄偏移（不再累加）。
    fn drag_out_handle(&mut self, curve: usize, key: usize, dt: f32, dv: f32) -> Vec<Tag> {
        let Some(kfs) = self.editing_keyframes.get_mut(&curve) else {
            return Vec::new();
        };
        let Some(kf) = kfs.get_mut(key) else {
            return Vec::new();
        };
        if kf.interpolation != SlintInterpolation::Bezier {
            return Vec::new(); // 非贝塞尔关键帧不拖出手柄
        }
        kf.handle_out = SlintBezierHandle {
            time_offset: dt,
            value_offset: dv,
        };
        kf.handle_in = SlintBezierHandle {
            time_offset: -dt,
            value_offset: -dv,
        };
        vec![Tag::AnimationKeyframesChanged {
            curves: vec![curve],
            summary_ms: vec![],
            dragged_curve: Some(curve),
            dragged_summary: None,
            handle_preview: None,
        }]
    }

    /// 关键帧松开。moved=true 表示发生过拖拽（保持多选）；false 为单击 → 折叠为单选。
    /// 先写回按下时记录的通道选中（grab 已结束，此时对外层 channels 模型更新安全），
    /// 再恢复普通手柄。
    pub fn keyframe_released(&mut self, curve: usize, key: usize, moved: bool) -> Vec<Tag> {
        let mut summary_ms: std::collections::HashSet<i64> = std::collections::HashSet::new();
        if !moved && self.pressed_modifier == SlintSelectionModifier::None {
            // 无修饰键的单击才做折叠/循环
            if self.pressed_was_selected {
                let coincident = self.find_coincident_keyframes(curve, key);
                for ms in self.cycle_coincident(&coincident) {
                    summary_ms.insert(ms);
                }
            } else {
                // 首次单击：折叠为单选被按的关键帧
                self.pending_channel_selection = vec![curve];
                self.selection_anchor = curve as isize;
                for ms in self.deselect_all() {
                    summary_ms.insert(ms);
                }
                self.selected_handles.clear();
                if let Some(ms) = self.set_keyframe_selected(curve, key, false) {
                    summary_ms.insert(ms);
                }
            }
        }
        let mut tags = self.apply_pending_channel_selection();
        if let Tag::AnimationSelectionChanged { summary_ms: ms } = tags.first_mut().unwrap() {
            ms.extend(summary_ms);
        }
        tags
    }

    /// 设置单个关键帧的选中状态（additive=true 时取反，否则置为选中）。
    /// 只改数据层，返回选中状态变化的 ms（投影重算汇总条目掩码）。
    pub(crate) fn set_keyframe_selected(&mut self, curve: usize, key: usize, additive: bool) -> Option<i64> {
        let Some(kfs) = self.editing_keyframes.get_mut(&curve) else {
            return None;
        };
        let Some(kf) = kfs.get_mut(key) else {
            return None;
        };
        let ms = ms_of(kf.time);
        let new_sel = if additive { !kf.selected } else { true };
        if kf.selected == new_sel {
            return None;
        }
        kf.selected = new_sel;
        Some(ms)
    }

    /// 当前 Action 是否有关键帧数据（全局 Action 的"向下插入"按钮据此显示）。
    pub fn has_keyframes(&self) -> bool {
        self.editing_keyframes.values().any(|kfs| !kfs.is_empty())
    }

    /// 清除全部关键帧选中与控制柄选中（数据层）。
    /// 只清 editing_keyframes 的 selected 字段与 selected_handles，不改 channels 数据，
    /// 因此关键帧拖拽期间调用安全。
    /// 返回全部受影响 ms（投影重算汇总条带掩码）。
    pub(crate) fn deselect_all(&mut self) -> Vec<i64> {
        let mut summary_ms: std::collections::HashSet<i64> = std::collections::HashSet::new();
        for kfs in self.editing_keyframes.values_mut() {
            for kf in kfs.iter_mut() {
                if kf.selected {
                    kf.selected = false;
                    summary_ms.insert(ms_of(kf.time));
                }
            }
        }
        self.selected_handles.clear();
        summary_ms.into_iter().collect()
    }
}
