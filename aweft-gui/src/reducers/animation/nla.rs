//! NLA 编辑器域 reducer（`impl AppStore`）：reducer 只改数据层 + model（`AnimData.tracks`）
//! 并返回精确 tag，派生层由 depsgraph 投影（`op_nla_*`）更新。
//!
//! 权威数据在 model（`Project.library.animation_library.anim_datas` 的 `AnimData`）：
//!  - **拖拽（高频）**：`StripDragged` 只改编辑数据（preview，对齐曲线编辑器预览语义），
//!    保住 Slint 拖拽抓取；
//!  - **提交**：`StripReleased` 后 `commit_to_model` 把编辑数据写回 model AnimData；
//!  - **装载**：`load_nla_from_model` 读 model AnimData 重建编辑数据（初始化路径）。
//!  - **切换**：Header 下拉 `AnimDataSelected` 换 `anim_data_id` 后按装载路径重建
//!    （候选清单 `anim_data_entries` 与非线性动画引用节点的下拉同一份顺序）。

use crate::actions::animation::nla::NlaAction;
use crate::bridge::converters::build_name_menu_items;
use crate::depsgraph::{NlaStripInsert, Tag};
use aweft_model::animation::AnimData;
use aweft_model::animation::strip::NlaTrack;
use aweft_model::ids::AnimDataId;
use slint::{Model, SharedString};

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

    /// 库里的动画片段清单（按名排序）：NLA Header 的下拉与非线性动画引用节点的下拉
    /// 共用这一份顺序，两处的候选文字与下标始终一致。
    pub(crate) fn anim_data_entries(&self) -> Vec<(AnimDataId, String)> {
        let mut pairs: Vec<(AnimDataId, String)> = self
            .state
            .project
            .project()
            .library
            .animation_library
            .anim_datas
            .iter()
            .map(|(id, anim_data)| (*id, anim_data.name.clone()))
            .collect();
        pairs.sort_by(|a, b| a.1.cmp(&b.1));
        pairs
    }

    /// 写入 NLA Header 下拉的候选（清单由 `refresh_anim_data_menu` 派生一次，两处共用）。
    /// `anim_items` 是共享 inner 模型，初始化时绑定一次，这里 `set_vec` 即自动通知 Slint。
    pub(crate) fn set_nla_anim_menu(&mut self, entries: &[(AnimDataId, String)]) {
        let nla = &mut self.ui_models.animation.nla;
        nla.anim_order = entries.iter().map(|(id, _)| *id).collect();
        let names: Vec<SharedString> = entries
            .iter()
            .map(|(_, name)| name.as_str().into())
            .collect();
        nla.anim_items.set_vec(build_name_menu_items(&names));
    }

    /// 分发 NLA 子 Action：reducer 只改数据层 + model 并返回精确 tag。
    pub(crate) fn dispatch_nla_action(&mut self, action: NlaAction) -> anyhow::Result<Vec<Tag>> {
        use NlaAction::*;

        // 每分支独立作用域：先改数据层（借用 ui_models），再同步 model（借用 state），
        // 避免跨字段借用冲突。
        let tags = match action {
            AnimDataSelected { index } => self.select_anim_data(index),
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

    /// Header 下拉选中某一项：换成编辑那一段动画片段。
    ///
    /// 切换前不需要写回旧片段：数据层在静止时始终与 model 同步（拖拽预览只发生在
    /// 按下到松开之间，松开即 `commit_to_model`）。下标越界（含负数）或选的就是
    /// 当前片段时什么都不做。
    fn select_anim_data(&mut self, index: i32) -> Vec<Tag> {
        let Some(&id) = self.ui_models.animation.nla.anim_order.get(index as usize) else {
            return Vec::new();
        };
        if self.ui_models.animation.nla.anim_data_id == Some(id) {
            return Vec::new();
        }
        self.ui_models.animation.nla.anim_data_id = Some(id);
        self.load_nla_data_layer()
    }

    /// 把当前 `anim_data_id` 指向的动画片段装载到数据层，返回重建派生层所需的 tag。
    /// reducer 路径（`select_anim_data`）把 tag 交给 dispatch；初始化路径
    /// （`load_nla_from_model`）自己打 tag 并求值。
    fn load_nla_data_layer(&mut self) -> Vec<Tag> {
        let Some(anim_data) = self.current_anim_data().cloned() else {
            return Vec::new();
        };
        let target_duration = self.ui_models.animation.graph.action_end;
        self.ui_models
            .animation
            .nla
            .load_from_model(&anim_data, target_duration);
        let mut tags = vec![Tag::NlaStructureChanged];
        tags.extend(self.sync_nla_global_action());
        tags
    }

    /// 从 model 装载 NLA 编辑器（与 `create_test_animation_data` 配套）。
    /// 仅改数据层，派生层由 `NlaStructureChanged`/`NlaTrackChanged` 投影重建（初始化亦走 depsgraph）。
    pub fn load_nla_from_model(&mut self) {
        for tag in self.load_nla_data_layer() {
            self.depsgraph.tag(tag);
        }
        self.evaluate_depsgraph();
    }

    /// 把 Header 下拉的当前选中项同步到 Slint 全局（`op_nla_structure` 投影调用）。
    /// 下标由 `anim_data_id` 在 `anim_order` 里的位置派生，不另存字段；
    /// 片段已不在库里（或还没装载）时下标为 -1，按钮显示占位文案。
    pub(crate) fn sync_nla_scalars(&self) {
        let Some(state) = self.animation_state.as_ref().and_then(|w| w.upgrade()) else {
            return;
        };
        let nla = &self.ui_models.animation.nla;
        let selected = nla
            .anim_data_id
            .and_then(|id| nla.anim_order.iter().position(|&x| x == id));
        state.set_current_nla_anim(selected.map(|i| i as i32).unwrap_or(-1));
        state.set_nla_anim_name(
            selected
                .and_then(|i| nla.anim_items.row_data(i))
                .map(|item| item.text)
                .unwrap_or_else(|| "选择动画片段".into()),
        );
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

// 本分支不含 `app/test_data.rs`（那份字幕播种助手由 feat/subtitle-viewport 建），
// 依赖它的两条 NLA 切换测试随那条分支一起进来（见 39c3908 对本文件的 `mod tests`）。
