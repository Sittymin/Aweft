use crate::actions::animation::graph::GraphAction;
use crate::bridge::types::SlintKeyframeDragMode;
use crate::depsgraph::Tag;

/// 动画编辑器（图编辑器）域 reducer（`impl AppStore`）：reducer 只改数据层并返回
/// 精确 tag，派生层由 depsgraph 投影（`op_graph_*`）更新。
impl crate::store::AppStore {
    /// 分发曲线/摄影表子 Action：reducer 只改数据层并返回精确 tag。
    ///
    /// 权威数据在 model（`Project.library.animation_library.actions`）：
    ///  - **拖拽（高频）**：`KeyframeDragged`/`HandleDragged` 只改编辑数据（preview，对齐
    ///    Sequence 的 preview 语义），保住 Slint 拖拽抓取；
    ///  - **视图写回**：松开/插值变更后 `write_view_to_model` 把编辑数据写回 model Action；
    ///  - **装载**：`ActionSelected` 读 model Action 重建编辑数据。
    ///
    /// 派生层（段/手柄/连接线/汇总/显示掩码/标量）由 depsgraph 投影更新，
    /// reducer 不直接触碰 Slint 模型。
    pub(crate) fn dispatch_graph_action(
        &mut self,
        action: GraphAction,
    ) -> anyhow::Result<Vec<Tag>> {
        use GraphAction::*;

        let tags = {
            let ce = &mut self.ui_models.animation.graph;
            match action {
                KeyframePressed {
                    curve,
                    key,
                    modifier,
                } => ce.keyframe_pressed(curve, key, modifier),
                KeyframeDragged {
                    curve,
                    key,
                    delta_time,
                    delta_value,
                    mode,
                } => ce.keyframe_dragged(curve, key, delta_time, delta_value, mode),
                KeyframeReleased { curve, key, moved } => {
                    let tags = ce.keyframe_released(curve, key, moved);
                    self.write_view_to_model();
                    tags
                }
                DeselectKeyframes => {
                    let ms = ce.deselect_all();
                    vec![Tag::GraphSelectionChanged { summary_ms: ms }]
                }
                DragCancel => ce.drag_cancel(),
                HandlePressed {
                    curve,
                    key,
                    is_in,
                    modifier,
                } => ce.handle_pressed(curve, key, is_in, modifier),
                HandleDragged {
                    curve,
                    key,
                    is_in,
                    delta_time,
                    delta_value,
                    mode,
                } => ce.handle_dragged(curve, key, is_in, delta_time, delta_value, mode),
                HandleReleased {
                    curve,
                    key,
                    is_in,
                    moved,
                } => {
                    let tags = ce.handle_released(curve, key, is_in, moved);
                    self.write_view_to_model();
                    tags
                }
                GraphMarqueeSelect {
                    x_lo,
                    x_hi,
                    y_lo,
                    y_hi,
                    modifier,
                } => ce.graph_marquee_select(x_lo, x_hi, y_lo, y_hi, modifier),
                ChannelPressed { curve, modifier } => ce.channel_pressed(curve, modifier),
                ChannelDeselectAll => ce.deselect_channel(),
                GraphSearchChanged { text } => ce.set_graph_search(&text),
                DopeSearchChanged { text } => ce.set_dope_search(&text),
                DopeChannelPressed { view_row, modifier } => {
                    ce.dope_channel_pressed(view_row, modifier)
                }
                // —— 摄影表（Dope Sheet）：与曲线编辑器共用数据/选中，拖拽只调时间 ——
                DopeKeyframePressed {
                    curve,
                    key,
                    modifier,
                } => ce.keyframe_pressed(curve, key, modifier),
                DopeKeyframeDragged {
                    curve,
                    key,
                    delta_time,
                } => {
                    // 与曲线编辑器同一条拖拽路径，值增量恒为 0（摄影表只有时间轴）
                    ce.keyframe_dragged(curve, key, delta_time, 0.0, SlintKeyframeDragMode::Move)
                }
                DopeKeyframeReleased { curve, key, moved } => {
                    let tags = ce.keyframe_released(curve, key, moved);
                    self.write_view_to_model();
                    tags
                }
                DopeKeyframeDeselectAll => {
                    let ms = ce.deselect_all();
                    vec![Tag::GraphSelectionChanged { summary_ms: ms }]
                }
                DopeMarqueeSelect {
                    x_lo,
                    x_hi,
                    row_lo,
                    row_hi,
                    modifier,
                } => ce.dope_marquee_select(x_lo, x_hi, row_lo, row_hi, modifier),
                DopeSummaryPressed { index, modifier } => {
                    ce.summary_keyframe_pressed(index, modifier)
                }
                DopeSummaryDragged { index, delta_time } => {
                    ce.summary_keyframe_dragged(index, delta_time)
                }
                DopeSummaryReleased { index, moved } => {
                    let tags = ce.summary_keyframe_released(index, moved);
                    self.write_view_to_model();
                    tags
                }
                CurveColorModeChanged { curve, mode } => ce.set_curve_color_mode(curve, mode),
                KeyframeInterpolationChanged { interpolation } => {
                    let tags = ce.set_selected_keyframes_interpolation(interpolation);
                    self.write_view_to_model();
                    tags
                }
                ChannelToggleVisible { curve } => ce.toggle_channel_visible(curve),
                ChannelToggleModifier { curve } => ce.toggle_channel_modifier(curve),
                ChannelToggleActive { curve } => ce.toggle_channel_active(curve),
                ChannelToggleEditable { curve } => ce.toggle_channel_editable(curve),
                SetCurrentTime { time } => {
                    ce.current_time = time as f64;
                    let sync_timeline =
                        crate::reducers::media::media_active_video_drives_timeline(self);
                    // 编辑器是时间权威：拖动播放头同步驱动当前选中（无则主）视频
                    // 到同一时刻（无视频时忽略；播放中 seek 后位置查询自然从新位置继续）
                    let mut tags = vec![Tag::GraphScalarsChanged];
                    if let Some(index) = self.seek_target(time as f64) {
                        tags.push(Tag::VideoTimeChanged {
                            indices: vec![index],
                            sync_timeline,
                        });
                    }
                    tags
                }
                ViewChanged { zoom_x, zoom_y } => ce.set_view(zoom_x, zoom_y),
                ActionSelected { index } => {
                    let mut tags = self.rebuild_view_from_model(index);
                    // 装载后同步 NLA 全局 Action 轨（名称/是否有数据）→ NlaTrackChanged 走投影局部更新
                    tags.extend(self.sync_nla_global_action());
                    tags
                }
            }
        };
        Ok(tags)
    }

    /// 视图关键帧写回 model Action（model 是唯一真相，store 视图是派生状态）。
    fn write_view_to_model(&mut self) {
        let Some(id) = self.ui_models.animation.graph.current_action_id() else {
            return;
        };
        let Some(action) = self
            .state
            .project
            .project_mut()
            .library
            .animation_library
            .actions
            .get_mut(&id)
        else {
            return;
        };
        self.ui_models.animation.graph.commit_to_model(action);
    }

    /// 从 model 重建视图（`ActionSelected` 的 reducer 步骤）：读 model Action 重建编辑数据，
    /// 返回结构变化 tag（投影重建派生层）。
    fn rebuild_view_from_model(&mut self, index: i32) -> Vec<Tag> {
        if index < 0 {
            return self.ui_models.animation.graph.reset_action();
        }
        let Some(id) = self
            .ui_models
            .animation
            .graph
            .actions
            .get(index as usize)
            .map(|a| a.id)
        else {
            return Vec::new();
        };
        let Some(action) = self
            .state
            .project
            .project()
            .library
            .animation_library
            .actions
            .get(&id)
        else {
            return Vec::new();
        };
        self.ui_models
            .animation
            .graph
            .load_action(index as usize, action)
    }

    /// 把投影重建的过滤视图重新绑定到 Slint 全局（`op_graph_structure` 调用）。
    /// FilterModel 构造时捕获搜索词，重建后旧实例不感知新词，必须重新 set_。
    pub(crate) fn rebind_animation_filters(&mut self) {
        let Some(weak) = &self.animation_state else {
            return;
        };
        let Some(state) = weak.upgrade() else {
            return;
        };
        let ce = &self.ui_models.animation.graph;
        state.set_graph_channels(ce.graph_filter.clone());
        state.set_dope_channels(ce.dope_filter.clone());
        state.set_graph_segments(ce.graph_segments.clone());
        state.set_graph_handles(ce.graph_handles.clone());
    }

    /// 同步动画编辑器标量到 Slint 全局（`op_graph_scalars` 调用）。
    /// 通道/关键帧等 VecModel 一次性绑定（共享 inner 模型，变更自动通知），
    /// 只有 current-time/action-name/current-action/action-range/current-channel/
    /// 选中数/插值等标量需要在此推送。
    pub(crate) fn sync_animation_scalars(&mut self) {
        let Some(weak) = &self.animation_state else {
            return;
        };
        let Some(state) = weak.upgrade() else {
            return;
        };
        let ce = &self.ui_models.animation.graph;
        state.set_current_time(ce.current_time as f32);
        state.set_action_name(ce.action_name.clone());
        state.set_current_action(ce.current_action);
        state.set_action_start(ce.action_start);
        state.set_action_end(ce.action_end);
        state.set_current_channel(ce.current_channel as i32);
        state.set_selected_keyframes(ce.selected_keyframe_count() as i32);
        state.set_selected_interpolation(ce.selected_interpolation_state());
    }

    /// 实时播放时钟只同步 current-time，避免每帧重复失效动作、通道和选择标量。
    pub(crate) fn sync_animation_current_time(&mut self) {
        let Some(weak) = &self.animation_state else {
            return;
        };
        let Some(state) = weak.upgrade() else {
            return;
        };
        state.set_current_time(self.ui_models.animation.graph.current_time as f32);
    }
}
