use std::collections::HashSet;

use crate::actions::area::AreaAction;
use crate::actions::animation::AnimationAction;
use crate::actions::line_editor::LineEditorAction;
use crate::actions::Action;
use crate::clipboard::{Clipboard, ItemsClip};
use crate::depsgraph::{Depsgraph, Tag};
use crate::store::state::AppState;
use crate::store::ui_models::UiModels;
use anyhow::Result;
use aweft_model::ids::ItemId;
use aweft_model::structure::ItemContent;
use chrono::TimeDelta;

/// 应用状态存储 - 单一数据源
pub struct AppStore {
    pub(crate) state: AppState,
    pub(crate) ui_models: UiModels,
    pub(crate) depsgraph: Depsgraph,
    /// OverlayState global 弱引用：overlay 投影经此写出拖拽预览。
    /// 在 `AppStore::new` 中从 MainWindow 注入。
    pub(crate) overlay_state: Option<slint::Weak<crate::bridge::types::OverlayState<'static>>>,
    /// AnimationState global 弱引用：动画投影经此同步标量/重新绑定过滤视图到 Slint。
    pub(crate) animation_state: Option<slint::Weak<crate::bridge::types::AnimationState<'static>>>,
    clipboard: Clipboard,
}

impl AppStore {
    /// 创建新的 Store（从 MainWindow 以访问主题系统）
    pub fn new(main_window: &crate::bridge::types::MainWindow) -> Self {
        use slint::{ComponentHandle, Global};

        let theme_settings = crate::bridge::types::ThemeSettings::from_main_window(main_window);
        let mut depsgraph = Depsgraph::new();
        crate::store::projections::register_all(&mut depsgraph);
        let overlay_state = main_window
            .global::<crate::bridge::types::OverlayState>()
            .as_weak();
        let animation_state = main_window
            .global::<crate::bridge::types::AnimationState>()
            .as_weak();
        Self {
            state: AppState::new(&main_window.window(), theme_settings),
            ui_models: UiModels::new(),
            depsgraph,
            overlay_state: Some(overlay_state),
            animation_state: Some(animation_state),
            clipboard: Clipboard::new(),
        }
    }

    /// 分发 Action：修改数据并向依赖图打 tag。
    pub fn dispatch(&mut self, action: Action) -> Result<()> {
        let tags = match action {
            Action::Screen(screen_action) => self.dispatch_screen(screen_action)?,
            Action::Drag(drag_action) => self.dispatch_drag(drag_action)?,
            Action::Area(area_action) => self.dispatch_area(area_action)?,
            Action::Media(_media_action) => {
                // TODO: 实现媒体相关的 reducer
                vec![]
            }
            Action::LineEditor(act) => self.dispatch_line_editor(act)?,
            Action::NodeGraph(act) => self.dispatch_node_graph(act)?,
            Action::Animation(act) => self.dispatch_animation(act)?,
        };
        for tag in tags {
            self.depsgraph.tag(tag);
        }
        Ok(())
    }

    /// 把已挂起的依赖图标记应用到 UI 投影。
    ///
    /// 通过 `mem::take` 暂时取出 depsgraph，避免 `&mut self` 与 `&mut self.depsgraph`
    /// 同时借用的冲突（投影重算需要访问 store 的其他部分）。
    pub fn evaluate_depsgraph(&mut self) {
        if !self.depsgraph.has_pending() {
            return;
        }
        let mut depsgraph = std::mem::take(&mut self.depsgraph);
        depsgraph.evaluate(self);
        self.depsgraph = depsgraph;
    }

    /// 标记一个数据变更（reducer 内部使用）。
    pub(crate) fn tag(&mut self, tag: Tag) {
        self.depsgraph.tag(tag);
    }

    /// 获取当前状态（只读）
    pub fn state(&self) -> &AppState {
        &self.state
    }

    /// 获取可变状态（内部使用）
    pub fn state_mut(&mut self) -> &mut AppState {
        &mut self.state
    }

    /// 获取 UI 模型（只读）
    pub fn ui_models(&self) -> &UiModels {
        &self.ui_models
    }

    /// 获取 UI 模型（可变）
    pub fn ui_models_mut(&mut self) -> &mut UiModels {
        &mut self.ui_models
    }

    /// 同时获取只读 `state` 与可变 `ui_models`。
    ///
    /// 用于需要“读取 Domain、写入 UI Projection”的场景（典型是投影重算应用 LineEditor 更新）。
    pub fn split_mut(&mut self) -> (&AppState, &mut UiModels) {
        (&self.state, &mut self.ui_models)
    }

    /// 初始化 LineEditor 的行数据与过滤模型，返回过滤后的 ModelRc。
    pub fn rebuild_line_editor_models(
        &mut self,
    ) -> slint::ModelRc<crate::bridge::types::SlintSubtitleRow> {
        self.ui_models
            .line_editor_state
            .initialize_rows(&self.state.project);
        self.ui_models.line_editor_state.init_filter_model()
    }

    // ========== 内部 dispatch 方法 ==========

    /// 可变访问字幕结构（model 纯值语义，直接 &mut）。
    fn structure_mut(&mut self) -> &mut aweft_model::structure::ProjectStructure {
        self.state.project.structure_mut()
    }

    /// 计算某个 item 的祖先分组链（叶子到根）。用于聚合传播。
    fn ancestor_chain(&self, item_id: ItemId) -> Vec<ItemId> {
        let mut chain = Vec::new();
        let mut cur = Some(item_id);
        while let Some(id) = cur {
            let structure = self.state.project.structure();
            let Some(item) = structure.get(id) else {
                break;
            };
            if item.parent.is_some() {
                chain.push(id);
            }
            cur = item.parent;
        }
        chain
    }

    /// 构造单个时间码变化的 tag（带祖先链聚合传播）。
    fn event_timecode_tag(&self, event_id: ItemId, is_start_time: bool, time: TimeDelta) -> Tag {
        Tag::EventTimecodeChanged {
            event_id,
            is_start_time,
            time,
            ancestors: self.ancestor_chain(event_id),
        }
    }

    /// 构造批量时间码变化的 tag（updates 与祖先链去重收集）。
    fn events_timecode_tag(&self, ids: &[ItemId], is_start_time: bool, times: &[TimeDelta]) -> Tag {
        Tag::EventsTimecodeChanged {
            updates: ids
                .iter()
                .copied()
                .zip(std::iter::repeat(is_start_time))
                .zip(times.iter().copied())
                .map(|((id, s), t)| (id, s, t))
                .collect(),
            ancestors: ids.iter().flat_map(|&id| self.ancestor_chain(id)).collect(),
        }
    }

    /// 构造选中变化的 tag（affected = 祖先链 + 自身）。
    fn selection_tag(&self, item_id: ItemId, kind: crate::depsgraph::SelectionKind) -> Tag {
        let mut affected = self.ancestor_chain(item_id);
        affected.push(item_id);
        Tag::SelectionChanged {
            item_id,
            kind,
            affected,
        }
    }

    /// 批量时间码调整的公共逻辑：收集选中事件 + 当前项，逐个调整并打批量 tag。
    ///
    /// `adjust` 决定调整方式（增量、预览、提交等），返回该次调整后的新时间；
    /// 失败的事件跳过。返回 `Some(tag)` 当有事件成功调整，否则 `None`。
    fn batch_timecode_tag<F>(
        &mut self,
        event_id: ItemId,
        is_start_time: bool,
        mut adjust: F,
    ) -> Option<Tag>
    where
        F: FnMut(&mut Self, ItemId) -> Option<TimeDelta>,
    {
        let mut event_ids: HashSet<ItemId> = self
            .ui_models
            .line_editor_state
            .get_selected_events(&self.state.project)
            .into_iter()
            .collect();
        // 确保当前 event_id 也在列表中（同时也避免重复）
        event_ids.insert(event_id);

        let mut updated_ids: Vec<ItemId> = Vec::new();
        let mut updated_times: Vec<TimeDelta> = Vec::new();
        for id in event_ids {
            if let Some(time) = adjust(self, id) {
                updated_ids.push(id);
                updated_times.push(time);
            }
        }
        if !updated_ids.is_empty() {
            Some(self.events_timecode_tag(&updated_ids, is_start_time, &updated_times))
        } else {
            None
        }
    }

    fn dispatch_screen(
        &mut self,
        action: crate::actions::screen::ScreenAction,
    ) -> Result<Vec<Tag>> {
        use crate::actions::screen::ScreenAction;

        match action {
            ScreenAction::WindowResized { width, height } => {
                crate::store::reducers::screen::handle_resize(&mut self.state, width, height)
            }
            ScreenAction::ScaleFactorChanged { scale_factor } => {
                crate::store::reducers::screen::handle_scale_change(&mut self.state, scale_factor)
            }
            ScreenAction::InitializeLayout { width, height } => {
                crate::store::reducers::screen::handle_initialize(&mut self.state, width, height)
            }
        }
    }

    fn dispatch_drag(&mut self, action: crate::actions::drag::DragAction) -> Result<Vec<Tag>> {
        use crate::actions::drag::DragAction;

        match action {
            DragAction::ActionZone(az_action) => self.dispatch_actionzone_drag(az_action),
            DragAction::Edge(edge_action) => self.dispatch_edge_drag(edge_action),
        }
    }

    fn dispatch_actionzone_drag(
        &mut self,
        action: crate::actions::drag::ActionZoneDrag,
    ) -> Result<Vec<Tag>> {
        use crate::actions::drag::ActionZoneDrag;

        match action {
            ActionZoneDrag::Update {
                area_key,
                corner,
                offset_x,
                offset_y,
            } => crate::store::reducers::drag::handle_actionzone_update(
                &mut self.state,
                area_key,
                corner,
                offset_x,
                offset_y,
            ),
            ActionZoneDrag::End {
                area_key,
                is_cancel,
            } => crate::store::reducers::drag::handle_actionzone_end(
                &mut self.state,
                area_key,
                is_cancel,
            ),
        }
    }

    fn dispatch_edge_drag(&mut self, action: crate::actions::drag::EdgeDrag) -> Result<Vec<Tag>> {
        use crate::actions::drag::EdgeDrag;

        match action {
            EdgeDrag::Update {
                vert_keys,
                offset_x,
                offset_y,
            } => crate::store::reducers::drag::handle_edge_update(
                &mut self.state,
                vert_keys,
                offset_x,
                offset_y,
            ),
            EdgeDrag::End { is_cancel } => {
                crate::store::reducers::drag::handle_edge_end(&mut self.state, is_cancel)
            }
        }
    }

    fn dispatch_area(&mut self, action: AreaAction) -> Result<Vec<Tag>> {
        use crate::actions::area::AreaAction;

        match action {
            AreaAction::SwitchType {
                area_key,
                area_type,
            } => crate::store::reducers::area::handle_switch_type(
                &mut self.state,
                area_key,
                area_type,
            ),
            AreaAction::Split {
                area_key,
                preview_data,
            } => {
                crate::store::reducers::area::handle_split(&mut self.state, area_key, preview_data)
            }
            AreaAction::Join {
                source_area_key,
                target_area_key,
                adjacency,
            } => crate::store::reducers::area::handle_join(
                &mut self.state,
                source_area_key,
                target_area_key,
                adjacency,
            ),
        }
    }

    /// 分发节点图操作：修改 model + 打精确 `Tag::NodeGraph`（投影据此局部更新）。
    ///
    /// 图类型用 model 的 `NodeGraphKind`（树类型），不再区分 Style/App 的访问路径——
    /// 全部经 `with_graph`（唯一访问器）路由，`dispatch` 本身与树类型无关。
    fn dispatch_node_graph(
        &mut self,
        action: crate::actions::node_graph::NodeGraphAction,
    ) -> Result<Vec<Tag>> {
        use crate::actions::node_graph::NodeGraphAction;
        use crate::depsgraph::NodeGraphChange;

        // 每个分支返回 (kind, change)，dispatch 据此组装精确 tag
        let (kind, change) = match action {
            NodeGraphAction::AddNode {
                graph_type,
                template_index,
                x,
                y,
            } => {
                let node_id = crate::store::graph_editor::add_node_dispatch(
                    self, graph_type, template_index, x, y,
                );
                let Some(node_id) = node_id else {
                    return Ok(vec![]);
                };
                let mut positions = std::collections::HashMap::new();
                positions.insert(node_id, (x, y));
                (
                    graph_type,
                    NodeGraphChange::NodesInserted {
                        node_ids: vec![node_id],
                        positions,
                    },
                )
            }
            NodeGraphAction::DeleteNode {
                graph_type,
                node_index,
            } => {
                let node_id =
                    crate::store::graph_editor::delete_node_dispatch(self, graph_type, node_index);
                let Some(node_id) = node_id else {
                    return Ok(vec![]);
                };
                (
                    graph_type,
                    NodeGraphChange::NodesRemoved {
                        node_ids: vec![node_id],
                    },
                )
            }
            NodeGraphAction::SocketDragEnded {
                graph_type,
                node_index,
                socket_index,
                is_input,
                x,
                y,
            } => {
                let affected_nodes = crate::store::graph_editor::socket_drag_ended_dispatch(
                    self, graph_type, node_index, socket_index, is_input, x, y,
                );
                (
                    graph_type,
                    NodeGraphChange::LinksChanged { affected_nodes },
                )
            }
            NodeGraphAction::MenuSelected {
                graph_type,
                node_index,
                menu_index,
            } => {
                crate::store::graph_editor::menu_selected_dispatch(
                    self, graph_type, node_index, menu_index,
                );
                // 原位改引用链：单行重建该节点（不增删结构）
                let Some(node_id) = self
                    .with_graph(graph_type, |editor, _| editor.id_at(node_index))
                    .flatten()
                else {
                    return Ok(vec![]);
                };
                (graph_type, NodeGraphChange::ReferenceChanged { node_id })
            }
            NodeGraphAction::SocketOptionSelected {
                graph_type,
                node_index,
                socket_index,
                option_index,
            } => {
                crate::store::graph_editor::socket_option_selected_dispatch(
                    self, graph_type, node_index, socket_index, option_index,
                );
                let Some((node_id, socket_id)) =
                    self.node_and_socket_id(graph_type, node_index, socket_index)
                else {
                    return Ok(vec![]);
                };
                (
                    graph_type,
                    NodeGraphChange::SocketValueChanged { node_id, socket_id },
                )
            }
            NodeGraphAction::CreateActor {
                graph_type,
                node_index,
                socket_index,
                name,
            } => {
                crate::store::graph_editor::create_actor_dispatch(
                    self, graph_type, node_index, socket_index, &name,
                );
                let Some((node_id, socket_id)) =
                    self.node_and_socket_id(graph_type, node_index, socket_index)
                else {
                    return Ok(vec![]);
                };
                (
                    graph_type,
                    NodeGraphChange::SocketValueChanged { node_id, socket_id },
                )
            }
            NodeGraphAction::SocketInputChanged {
                graph_type,
                node_index,
                socket_index,
                text,
            } => {
                crate::store::graph_editor::set_socket_text_default_dispatch(
                    self, graph_type, node_index, socket_index, &text,
                );
                let Some((node_id, socket_id)) =
                    self.node_and_socket_id(graph_type, node_index, socket_index)
                else {
                    return Ok(vec![]);
                };
                (
                    graph_type,
                    NodeGraphChange::SocketValueChanged { node_id, socket_id },
                )
            }
            NodeGraphAction::NewChain { graph_type } => {
                // reducer 只建链（model），返回目标下标；菜单/当前链/渲染由投影完成
                let target = crate::store::graph_editor::new_chain_dispatch(self, graph_type);
                (graph_type, NodeGraphChange::ChainSwitched { target })
            }
        };
        Ok(vec![change.into_tag(kind)])
    }

    /// 取指定图类型某端口对应节点的真实 (NodeId, SocketId)。
    fn node_and_socket_id(
        &mut self,
        kind: crate::depsgraph::NodeGraphKind,
        node_index: usize,
        socket_index: usize,
    ) -> Option<(aweft_model::ids::NodeId, aweft_model::ids::SocketId)> {
        self.with_graph(kind, |editor, access| {
            let node_id = editor.id_at(node_index)?;
            let socket_id = access.socket_id_at(editor, node_index, socket_index)?;
            Some((node_id, socket_id))
        })
        .flatten()
    }

    /// 分发动画编辑器（曲线编辑器）交互操作：reducer 只改数据层并返回精确 tag。
    ///
    /// 权威数据在 model（`Project.library.animation_library.actions`）：
    ///  - **拖拽（高频）**：`KeyframeDragged`/`HandleDragged` 只改编辑数据（preview，对齐
    ///    LineEditor 的 preview 语义），保住 Slint 拖拽抓取；
    ///  - **视图写回**：松开/插值变更后 `write_view_to_model` 把编辑数据写回 model Action；
    ///  - **装载**：`ActionSelected` 读 model Action 重建编辑数据。
    ///
    /// 派生层（段/手柄/连接线/汇总/显示掩码/标量）由 depsgraph 投影更新，
    /// reducer 不直接触碰 Slint 模型。
    fn dispatch_animation(&mut self, action: AnimationAction) -> Result<Vec<Tag>> {
        use AnimationAction::*;

        let tags = {
            let ce = &mut self.ui_models.curve_editor;
            match action {
                KeyframePressed { curve, key, modifier } => ce.keyframe_pressed(curve, key, modifier),
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
                    vec![Tag::AnimationSelectionChanged { summary_ms: ms }]
                }
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
                DopeChannelPressed { view_row, modifier } => ce.dope_channel_pressed(view_row, modifier),
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
                    vec![Tag::AnimationScalarsChanged]
                }
                ViewChanged {
                    zoom_x,
                    zoom_y,
                } => ce.set_view(zoom_x, zoom_y),
                ActionSelected { index } => self.rebuild_view_from_model(index),
            }
        };
        Ok(tags)
    }

    /// 视图关键帧写回 model Action（model 是唯一真相，store 视图是派生状态）。
    fn write_view_to_model(&mut self) {
        let Some(id) = self.ui_models.curve_editor.current_action_id() else {
            return;
        };
        let Some(action) = self
            .state
            .project
            .library
            .animation_library
            .actions
            .get_mut(&id)
        else {
            return;
        };
        self.ui_models.curve_editor.commit_to_model(action);
    }

    /// 从 model 重建视图（`ActionSelected` 的 reducer 步骤）：读 model Action 重建编辑数据，
    /// 返回结构变化 tag（投影重建派生层）。
    fn rebuild_view_from_model(&mut self, index: i32) -> Vec<Tag> {
        if index < 0 {
            return self.ui_models.curve_editor.reset_action();
        }
        let Some(id) = self
            .ui_models
            .curve_editor
            .actions
            .get(index as usize)
            .map(|a| a.id)
        else {
            return Vec::new();
        };
        let Some(action) = self
            .state
            .project
            .library
            .animation_library
            .actions
            .get(&id)
        else {
            return Vec::new();
        };
        self.ui_models.curve_editor.load_action(index as usize, action)
    }

    /// 把投影重建的过滤视图重新绑定到 Slint 全局（`op_animation_structure` 调用）。
    /// FilterModel 构造时捕获搜索词，重建后旧实例不感知新词，必须重新 set_。
    pub(crate) fn rebind_animation_filters(&mut self) {
        let Some(weak) = &self.animation_state else {
            return;
        };
        let Some(state) = weak.upgrade() else {
            return;
        };
        let ce = &self.ui_models.curve_editor;
        state.set_graph_channels(ce.graph_filter.clone());
        state.set_dope_channels(ce.dope_filter.clone());
        state.set_graph_segments(ce.graph_segments.clone());
        state.set_graph_handles(ce.graph_handles.clone());
    }

    /// 同步动画编辑器标量到 Slint 全局（`op_animation_scalars` 调用）。
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
        let ce = &self.ui_models.curve_editor;
        state.set_current_time(ce.current_time as f32);
        state.set_action_name(ce.action_name.clone());
        state.set_current_action(ce.current_action);
        state.set_action_start(ce.action_start);
        state.set_action_end(ce.action_end);
        state.set_current_channel(ce.current_channel as i32);
        state.set_selected_keyframes(ce.selected_keyframe_count() as i32);
        state.set_selected_interpolation(ce.selected_interpolation_state());
    }

    // 处理 LineEditor 相关 Action。
    //
    // 设计目标：本函数只做 Domain（`project.structure`）修改，并向依赖图打 tag；
    // UI 投影更新交给 `Depsgraph` 的投影重算（LineEditor 投影依赖
    // `ProjectStructure` / `Selection` 数据域）。
    fn dispatch_line_editor(&mut self, action: LineEditorAction) -> Result<Vec<Tag>> {
        match action {
            LineEditorAction::ToggleGroupCollapsed { group_id } => {
                // Rust 数据层修改 (持久化状态)
                if let Some(item) = self.structure_mut().get_mut(group_id) {
                    if let ItemContent::Group(g) = &mut item.content {
                        g.collapsed = !g.collapsed;
                    }
                }

                Ok(vec![Tag::GroupCollapsedToggled {
                    group_id,
                    ancestors: self.ancestor_chain(group_id),
                }])
            }
            LineEditorAction::InputTimecode {
                event_id,
                text,
                is_start_time,
            } => {
                let time_result =
                    self.structure_mut()
                        .set_event_time_fuzzy(event_id, is_start_time, &text);
                if let Ok(time) = time_result {
                    return Ok(vec![self.event_timecode_tag(event_id, is_start_time, time)]);
                }
                Ok(vec![])
            }
            LineEditorAction::IncrementTimecode {
                event_id,
                is_start_time,
                is_batch,
            } => {
                if is_batch {
                    Ok(self
                        .batch_timecode_tag(event_id, is_start_time, |s, id| {
                            s.structure_mut()
                                .adjust_event_time(id, is_start_time, TimeDelta::seconds(1))
                                .ok()
                        })
                        .map(|t| vec![t])
                        .unwrap_or_default())
                } else {
                    let time = self
                        .structure_mut()
                        .adjust_event_time(event_id, is_start_time, TimeDelta::seconds(1))
                        .ok();
                    Ok(time
                        .map(|t| vec![self.event_timecode_tag(event_id, is_start_time, t)])
                        .unwrap_or_default())
                }
            }
            LineEditorAction::DecrementTimecode {
                event_id,
                is_start_time,
                is_batch,
            } => {
                if is_batch {
                    Ok(self
                        .batch_timecode_tag(event_id, is_start_time, |s, id| {
                            s.structure_mut()
                                .adjust_event_time(id, is_start_time, TimeDelta::seconds(-1))
                                .ok()
                        })
                        .map(|t| vec![t])
                        .unwrap_or_default())
                } else {
                    let time = self
                        .structure_mut()
                        .adjust_event_time(event_id, is_start_time, TimeDelta::seconds(-1))
                        .ok();
                    Ok(time
                        .map(|t| vec![self.event_timecode_tag(event_id, is_start_time, t)])
                        .unwrap_or_default())
                }
            }
            LineEditorAction::DragTimecodePreview {
                event_id,
                is_start_time,
                delta_x,
                is_batch,
            } => {
                log::debug!("DragTimecodePreview delta_x: {}", delta_x);

                if !delta_x.is_finite()
                    || delta_x >= (i64::MAX as f32)
                    || delta_x <= (i64::MIN as f32)
                {
                    return Ok(vec![]);
                }

                let delta = TimeDelta::milliseconds((delta_x.trunc() as i64) * 10);

                if is_batch {
                    // 批量预览：不写 structure，构造批量 preview tag
                    let mut event_ids: HashSet<ItemId> = self
                        .ui_models
                        .line_editor_state
                        .get_selected_events(&self.state.project)
                        .into_iter()
                        .collect();
                    event_ids.insert(event_id);

                    let mut tags = Vec::new();
                    let mut ancestors: Vec<ItemId> = Vec::new();
                    for id in event_ids {
                        if let Ok(time) = self.state.project.structure().preview_adjust_event_time(
                            id,
                            is_start_time,
                            delta,
                        ) {
                            tags.push(Tag::TimecodePreviewChanged {
                                event_id: id,
                                is_start_time,
                                time,
                                ancestors: self.ancestor_chain(id),
                            });
                            ancestors.extend(self.ancestor_chain(id));
                        }
                    }
                    Ok(tags)
                } else {
                    // 单个预览：不写 structure，仅打 preview tag 供 UI 显示覆盖值
                    let time = self
                        .state
                        .project
                        .structure()
                        .preview_adjust_event_time(event_id, is_start_time, delta)
                        .ok();
                    Ok(time
                        .map(|t| {
                            vec![Tag::TimecodePreviewChanged {
                                event_id,
                                is_start_time,
                                time: t,
                                ancestors: self.ancestor_chain(event_id),
                            }]
                        })
                        .unwrap_or_default())
                }
            }
            LineEditorAction::DragTimecodeCommit {
                event_id,
                is_start_time,
                delta_x,
                is_batch,
            } => {
                log::debug!("DragTimecodeCommit delta_x: {}", delta_x);

                if !delta_x.is_finite()
                    || delta_x >= (i64::MAX as f32)
                    || delta_x <= (i64::MIN as f32)
                {
                    return Ok(vec![]);
                }

                let delta = TimeDelta::milliseconds((delta_x.trunc() as i64) * 10);

                if is_batch {
                    Ok(self
                        .batch_timecode_tag(event_id, is_start_time, |s, id| {
                            s.structure_mut()
                                .adjust_event_time(id, is_start_time, delta)
                                .ok()
                        })
                        .map(|t| vec![t])
                        .unwrap_or_default())
                } else {
                    let result =
                        self.structure_mut()
                            .adjust_event_time(event_id, is_start_time, delta);
                    match result {
                        Ok(time) => {
                            Ok(vec![self.event_timecode_tag(event_id, is_start_time, time)])
                        }
                        Err(e) => {
                            log::warn!("{e}");
                            Ok(vec![])
                        }
                    }
                }
            }
            LineEditorAction::DragTimecodeCancel {
                event_id,
                is_start_time,
                is_batch,
            } => {
                if is_batch {
                    Ok(self
                        .batch_timecode_tag(event_id, is_start_time, |s, id| {
                            s.state
                                .project
                                .structure()
                                .event_time(id, is_start_time)
                                .ok()
                        })
                        .map(|t| vec![t])
                        .unwrap_or_default())
                } else {
                    // 单个取消（恢复到 Domain 当前值）
                    match self
                        .state
                        .project
                        .structure()
                        .event_time(event_id, is_start_time)
                    {
                        Ok(time) => {
                            Ok(vec![self.event_timecode_tag(event_id, is_start_time, time)])
                        }
                        Err(_) => Ok(vec![]),
                    }
                }
            }
            LineEditorAction::SelectRow { item_id, modifier } => {
                let kind = match modifier {
                    crate::bridge::types::SlintSelectionModifier::Shift => {
                        crate::depsgraph::SelectionKind::Range
                    }
                    crate::bridge::types::SlintSelectionModifier::Ctrl => {
                        crate::depsgraph::SelectionKind::Toggle
                    }
                    crate::bridge::types::SlintSelectionModifier::None => {
                        crate::depsgraph::SelectionKind::Single
                    }
                };
                Ok(vec![self.selection_tag(item_id, kind)])
            }
            LineEditorAction::AddLine {
                item_id,
                end_time,
                actor,
                text,
                segment_timing,
                // FIXME: 之后需要更改为节点链
                style,
            } => {
                let new_item_id = self.structure_mut().insert_event_after(
                    item_id,
                    end_time,
                    actor,
                    text,
                    segment_timing,
                    style,
                )?;

                Ok(vec![
                    Tag::ItemsInserted {
                        item_id,
                        new_item_id,
                    },
                    self.selection_tag(new_item_id, crate::depsgraph::SelectionKind::Single),
                ])
            }
            LineEditorAction::AddGroup { item_id } => {
                let group_count = self
                    .state
                    .project
                    .structure()
                    .items
                    .values()
                    .filter(|item| matches!(item.content, ItemContent::Group(_)))
                    .count();

                let name = format!("新分组 {}", group_count + 1);

                let new_group_item_id = self.structure_mut().insert_group_after(item_id, name)?;
                Ok(vec![
                    Tag::ItemsInserted {
                        item_id,
                        new_item_id: new_group_item_id,
                    },
                    self.selection_tag(new_group_item_id, crate::depsgraph::SelectionKind::Single),
                ])
            }
            LineEditorAction::DeleteItems {} => {
                let removed = self.delete_currently_selected();
                Ok(vec![Tag::ItemsRemoved { item_ids: removed }])
            }
            LineEditorAction::CopyItems => {
                self.copy_currently_selected()?;
                Ok(vec![])
            }
            LineEditorAction::CutItems => {
                self.copy_currently_selected()?;
                let removed = self.delete_currently_selected();
                Ok(vec![Tag::ItemsRemoved { item_ids: removed }])
            }
            LineEditorAction::UnGroup { group_item_id } => {
                let new_ids = self.structure_mut().ungroup(group_item_id).map_err(|e| {
                    log::error!("解散分组失败 {:?}: {}", group_item_id, e);
                    e
                })?;
                // UI 更新：移除旧分组行 + 插入提升的子项行（精确锚点）。
                // 选中联动：子项提升后选中新根（reducer 声明数据变化，投影按 tag 原子执行）。
                let mut tags = vec![Tag::GroupsUngrouped {
                    group_id: group_item_id,
                    new_ids: new_ids.clone(),
                }];
                if let (Some(&first), Some(&last)) = (new_ids.first(), new_ids.last()) {
                    tags.push(self.selection_tag(first, crate::depsgraph::SelectionKind::Single));
                    tags.push(self.selection_tag(last, crate::depsgraph::SelectionKind::Range));
                }
                Ok(tags)
            }
            LineEditorAction::PasteItems { anchor } => {
                let Some(clip) = self.clipboard.paste_items()? else {
                    log::info!("剪贴板没有可粘贴的内容");
                    return Ok(vec![]);
                };
                let (new_root_ids, base_location) =
                    clip.instantiate(&mut self.structure_mut(), anchor)?;
                // 结构变化 + 选中联动（粘贴后选中新内容首个根）。
                let mut tags = vec![Tag::ItemsPasted {
                    new_root_ids: new_root_ids.clone(),
                    base_location,
                }];
                if let Some(&first) = new_root_ids.first() {
                    tags.push(self.selection_tag(first, crate::depsgraph::SelectionKind::Single));
                }
                Ok(tags)
            }
        }
    }

    /// 从当前选中集合构造 `ItemsClip` 并写入剪贴板，返回 root 数量。
    /// 选中为空时返回 0 且不写入剪贴板（保留旧内容）
    // NOTE: 也许输入为空的时候清空当前剪贴板更好？
    fn copy_currently_selected(&mut self) -> Result<usize> {
        let selected = self.ui_models.line_editor_state.directly_selected_items();
        if selected.is_empty() {
            return Ok(0);
        }
        let clip = ItemsClip::from_selection(&*self.state.project.structure(), &selected);
        let count = clip.root_count();
        self.clipboard.copy_items(&clip)?;
        Ok(count)
    }

    /// 从 Domain 删除当前选中集合（过滤"祖先已选中"），返回实际被移除的 root id 列表。
    fn delete_currently_selected(&mut self) -> Vec<ItemId> {
        let selected_set = self.ui_models.line_editor_state.directly_selected_items();
        let to_delete: Vec<ItemId> = selected_set
            .iter()
            .copied()
            .filter(|id| {
                !&self
                    .state
                    .project
                    .structure()
                    .has_ancestor_in_set(id, &selected_set)
            })
            .collect();

        let mut removed: Vec<ItemId> = Vec::with_capacity(to_delete.len());
        for id in to_delete {
            match self.structure_mut().remove_item(id) {
                Ok(()) => removed.push(id),
                Err(e) => log::warn!("删除 structure 项失败 id={:?}: {}", id, e),
            }
        }
        removed
    }
}
