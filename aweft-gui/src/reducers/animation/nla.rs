//! NLA 编辑器域 reducer（`impl AppStore`）：reducer 只改数据层 + model（`AnimData.tracks`）
//! 并返回精确 tag，派生层由 depsgraph 投影（`op_nla_*`）更新。
//!
//! 权威数据在 model（`Project.library.animation_library.anim_datas` 的 `AnimData`）：
//!  - **拖拽（高频）**：`StripDragged` 只改编辑数据（preview，对齐曲线编辑器预览语义），
//!    保住 Slint 拖拽抓取；
//!  - **提交**：`StripReleased` 后 `commit_to_model` 把编辑数据写回 model AnimData；
//!  - **装载**：`load_nla_from_model` 读 model AnimData 重建编辑数据（初始化路径）。

use crate::actions::animation::nla::NlaAction;
use crate::depsgraph::{NlaStripInsert, Tag};
use aweft_model::animation::AnimData;
use aweft_model::animation::strip::NlaTrack;

impl crate::store::AppStore {
    /// 当前 NLA 编辑的 AnimData（model 权威数据）。
    pub(crate) fn current_anim_data(&self) -> Option<&AnimData> {
        let id = self.ui_models.animation.nla.anim_data_id?;
        self.state
            .project
            .project()
            .library
            .animation_library
            .anim_datas
            .get(&id)
    }

    /// 当前 NLA 编辑的 AnimData（可变）。
    pub(crate) fn current_anim_data_mut(&mut self) -> Option<&mut AnimData> {
        let id = self.ui_models.animation.nla.anim_data_id?;
        self.state
            .project
            .project_mut()
            .library
            .animation_library
            .anim_datas
            .get_mut(&id)
    }

    /// 分发 NLA 子 Action：reducer 只改数据层 + model 并返回精确 tag。
    pub(crate) fn dispatch_nla_action(&mut self, action: NlaAction) -> anyhow::Result<Vec<Tag>> {
        use NlaAction::*;

        // 每分支独立作用域：先改数据层（借用 ui_models），再同步 model（借用 state），
        // 避免跨字段借用冲突。
        let tags = match action {
            StripPressed {
                track,
                strip,
                modifier,
            } => self
                .ui_models
                .animation
                .nla
                .strip_pressed(track, strip, modifier),
            StripDragged { delta_x, delta_row } => self
                .ui_models
                .animation
                .nla
                .strip_dragged(delta_x, delta_row),
            StripReleased => {
                let tags = self.ui_models.animation.nla.strip_released();
                // 拖拽结束提交：数据层写回 model（model 是唯一真相）
                self.write_nla_view_to_model();
                tags
            }
            StripDeselectAll => self.ui_models.animation.nla.deselect_all(),
            StripDragCancel => self.ui_models.animation.nla.strip_drag_cancel(),
            ToggleTrackEnabled { track } => {
                let tags = self.ui_models.animation.nla.toggle_track_enabled(track);
                // model 同步：enabled ↔ muted
                let enabled = self
                    .ui_models
                    .animation
                    .nla
                    .track_view
                    .get(track)
                    .map(|t| t.enabled)
                    .unwrap_or(false);
                if let Some(anim_data) = self.current_anim_data_mut() {
                    if let Some(model_track) = anim_data.tracks.get_mut(track.saturating_sub(1)) {
                        model_track.muted = !enabled;
                    }
                }
                tags
            }
            ToggleTrackLocked { track } => self.ui_models.animation.nla.toggle_track_locked(track),
            SearchChanged { text } => self.ui_models.animation.nla.filter_tracks(&text),
            PushDown => self.push_down_action(),
            MarqueeSelect {
                x_lo,
                x_hi,
                row_lo,
                row_hi,
                modifier,
            } => self
                .ui_models
                .animation
                .nla
                .marquee_select(x_lo, x_hi, row_lo, row_hi, modifier),
        };
        Ok(tags)
    }

    /// 视图片断写回 model AnimData（model 是唯一真相，store 视图是派生状态）。
    pub(crate) fn write_nla_view_to_model(&mut self) {
        let Some(id) = self.ui_models.animation.nla.anim_data_id else {
            return;
        };
        let Some(anim_data) = self
            .state
            .project
            .project_mut()
            .library
            .animation_library
            .anim_datas
            .get_mut(&id)
        else {
            return;
        };
        self.ui_models.animation.nla.commit_to_model(anim_data);
    }

    /// 同步全局 Action 轨（第 0 轨）：名称 = 当前编辑的 Action 名，
    /// `has_data` = 当前 Action 是否有关键帧（无数据时"向下插入"按钮隐藏）。
    /// 仅改数据层，返回 `NlaTrackChanged{0}` 由 depsgraph 投影局部更新（初始化亦同）。
    pub(crate) fn sync_nla_global_action(&mut self) -> Vec<Tag> {
        let (name, has_data) = self
            .ui_models
            .animation
            .graph
            .current_action_id()
            .map(|id| {
                let has_data = self
                    .state
                    .project
                    .project()
                    .library
                    .animation_library
                    .actions
                    .get(&id)
                    .map(|a| !a.curves.is_empty())
                    .unwrap_or(false);
                (self.ui_models.animation.graph.action_name.clone(), has_data)
            })
            .unwrap_or_else(|| ("".into(), false));
        self.ui_models
            .animation
            .nla
            .set_global_action(name, has_data)
    }

    /// "向下插入"（reducer 步骤）：把当前曲线编辑器编辑的 Action 作为片断加入 NLA，
    /// 时间范围用当前 Action 的关键帧范围（action_start/action_end）。
    /// 已有轨道能容纳 → 片断级局部插入（NlaStripsChanged.inserted，row_added）；
    /// 没有可容纳轨道 → 新建轨道（NlaTrackAdded，轨道行 row_added）。
    fn push_down_action(&mut self) -> Vec<Tag> {
        let Some(action_id) = self.ui_models.animation.graph.current_action_id() else {
            return Vec::new();
        };
        let name = self.ui_models.animation.graph.action_name.to_string();
        let start = self.ui_models.animation.graph.action_start;
        let end = self.ui_models.animation.graph.action_end;
        let Some(result) = self
            .ui_models
            .animation
            .nla
            .push_down_insert(name, start, end, action_id)
        else {
            return Vec::new();
        };
        let track = result.track;
        // model 同步：新建轨道时 model 需先补位（数据层轨道 i ↔ model tracks[i-1]）
        if result.new_track {
            if let Some(anim_data) = self.current_anim_data_mut() {
                while anim_data.tracks.len() < track {
                    anim_data.tracks.push(NlaTrack::new("NLA 片段"));
                }
            }
        }
        self.write_nla_view_to_model();
        if result.new_track {
            vec![Tag::NlaTrackAdded { track }]
        } else {
            vec![Tag::NlaStripStructureChanged {
                moved: Vec::new(),
                reordered: Vec::new(),
                inserted: vec![NlaStripInsert {
                    track,
                    index: result.strip,
                }],
            }]
        }
    }

    /// 从 model 装载 NLA 编辑器（与 `create_test_animation_data` 配套）。
    /// 仅改数据层，派生层由 `NlaStructureChanged`/`NlaTrackChanged` 投影重建（初始化亦走 depsgraph）。
    pub fn load_nla_from_model(&mut self) {
        let Some(anim_data) = self.current_anim_data().cloned() else {
            return;
        };
        let target_duration = self.ui_models.animation.graph.action_end;
        self.ui_models
            .animation
            .nla
            .load_from_model(&anim_data, target_duration);
        let mut tags = vec![Tag::NlaStructureChanged];
        tags.extend(self.sync_nla_global_action());
        for tag in tags {
            self.depsgraph.tag(tag);
        }
        self.evaluate_depsgraph();
    }

    /// 把投影重建的轨道过滤视图重新绑定到 Slint 全局（`op_nla_structure` 调用）。
    /// tracks_filter 为 VecModel（共享 inner，常规变更自动通知），此处为结构重建后保险。
    pub(crate) fn rebind_nla_tracks(&mut self) {
        let Some(weak) = &self.animation_state else {
            return;
        };
        let Some(state) = weak.upgrade() else {
            return;
        };
        state.set_nla_tracks(self.ui_models.animation.nla.tracks_filter.clone().into());
    }
}
