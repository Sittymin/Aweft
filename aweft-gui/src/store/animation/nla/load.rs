use super::NlaPushDownResult;
use super::{NlaEditorState, NlaStripData};
use crate::bridge::types::SlintNlaTrack;
use crate::depsgraph::Tag;
use aweft_model::{
    animation::{AnimData, strip::StripPlacement},
    ids::ActionId,
};
use slint::{ModelRc, SharedString};

impl NlaEditorState {
    /// 从 model AnimData 装载（仅重建数据层，派生层由 `NlaStructureChanged` 投影重建）。
    /// 旧实现曾直接 `apply_structure`（初始化例外），现统一走 depsgraph：调用方需
    /// `tag(NlaStructureChanged)` + `evaluate`（`reducers/animation/nla.rs:load_nla_from_model` 已接入）。
    /// 单测直接测数据层/派生层时可手动 `apply_structure`。
    pub(crate) fn load_from_model(&mut self, anim_data: &AnimData, target_duration: f32) {
        self.next_strip_id = 1;
        self.drag = None;
        let mut tv = vec![SlintNlaTrack {
            name: "<无动作>".into(),
            enabled: true,
            locked: false,
            strips: ModelRc::default(),
            is_action: true,
            has_data: false,
            orig_index: 0,
        }];
        let mut strips: Vec<Vec<NlaStripData>> = vec![Vec::new()];
        for t in &anim_data.tracks {
            tv.push(SlintNlaTrack {
                name: t.name.clone().into(),
                enabled: !t.muted,
                locked: false,
                strips: ModelRc::default(),
                is_action: false,
                has_data: false,
                orig_index: tv.len() as i32,
            });
            strips.push(
                t.strips
                    .iter()
                    .map(|s| {
                        let id = self.allocate_strip_id();
                        NlaStripData::from_model(id, s, target_duration)
                    })
                    .collect(),
            );
        }
        self.anim_data_id = Some(anim_data.id);
        self.track_view = tv;
        self.editing_strips = strips;
    }

    /// 把数据层写回 model AnimData（model 是唯一真相，store 视图是派生状态）。
    /// 轨道结构按位置对应（数据层轨道 i >= 1 ↔ model tracks[i-1]），片断整体重建：
    /// 保留 model 未编辑字段（blend_mode/extend_mode/modifiers/strip_type），
    /// 只更新时间范围（Absolute placement）、名称与 muted。
    pub(crate) fn commit_to_model(&self, anim_data: &mut AnimData) {
        for ti in 1..self.editing_strips.len() {
            let Some(track) = anim_data.tracks.get_mut(ti - 1) else {
                continue;
            };
            track.name = self.track_view[ti].name.to_string();
            track.muted = !self.track_view[ti].enabled;
            track.strips = self.editing_strips[ti]
                .iter()
                .map(|e| {
                    let mut s = e.model.clone();
                    s.name = e.view.name.to_string();
                    s.muted = e.view.muted;
                    s.placement = StripPlacement::Absolute {
                        start: e.view.start as f64,
                        end: e.view.end as f64,
                    };
                    s
                })
                .collect();
        }
    }

    /// 同步全局 Action 轨（第 0 轨 is-action）：名称用当前实际 Action 名，
    /// `has_data` 表示当前 Action 是否有关键帧（无数据时"向下插入"按钮隐藏）。
    /// 无名称（空 Action）时显示"<无动作>"。
    /// 仅改数据层，派生层由 `NlaTrackChanged{0}` 投影更新（`set_row_data`局部）。
    pub fn set_global_action(&mut self, name: impl Into<SharedString>, has_data: bool) -> Vec<Tag> {
        let Some(t) = self.track_view.first() else {
            return Vec::new();
        };
        if !t.is_action {
            return Vec::new();
        }
        let name: SharedString = name.into();
        let display: SharedString = if name.trim().is_empty() {
            "<无动作>".into()
        } else {
            name
        };
        let cur = &self.track_view[0];
        if cur.name == display && cur.has_data == has_data {
            return Vec::new();
        }
        self.track_view[0].name = display;
        self.track_view[0].has_data = has_data;
        vec![Tag::NlaTrackChanged { track: 0 }]
    }

    /// 按名字搜索轨道并重建过滤投影（空关键字 = 全部）。
    pub fn filter_tracks(&mut self, text: impl Into<String>) -> Vec<Tag> {
        self.search = text.into();
        vec![Tag::NlaStructureChanged]
    }

    /// 向下插入辅助（reducer 调用）：把当前 Action 作为片断加入 NLA——
    /// 先找能容纳 [start, end] 的已有轨道插入之，否则新建一条轨道再插入。
    /// 返回带语义字段的 `NlaPushDownResult`。**不自同步派生层**——
    /// 已有轨道走局部插入（NlaStripsChanged.inserted，投影 row_added 片断）；
    /// 新建轨道走 NlaTrackAdded（投影 row_added 轨道行），由投影执行。
    pub(crate) fn push_down_insert(
        &mut self,
        name: impl Into<String>,
        start: f32,
        end: f32,
        action_id: ActionId,
    ) -> Option<NlaPushDownResult> {
        if !start.is_finite() || !end.is_finite() || end <= start {
            return None;
        }
        if let Some(track) = self.find_fit_track(start, end) {
            let id = self.allocate_strip_id();
            let idx = self.insert_strip(
                track,
                NlaStripData::new_action(id, name, action_id, start, end),
            );
            return Some(NlaPushDownResult {
                track,
                strip: idx,
                new_track: false,
            });
        }
        // 新建轨道：手动追加数据层（不触发全量重建，由 NlaTrackAdded 投影局部插入）
        let idx = self.track_view.len();
        self.track_view.push(SlintNlaTrack {
            name: "NLA 片段".into(),
            enabled: true,
            locked: false,
            strips: ModelRc::default(),
            is_action: false,
            has_data: false,
            orig_index: idx as i32,
        });
        let id = self.allocate_strip_id();
        self.editing_strips.push(vec![NlaStripData::new_action(
            id, name, action_id, start, end,
        )]);
        Some(NlaPushDownResult {
            track: idx,
            strip: 0,
            new_track: true,
        })
    }
}

impl crate::store::AppStore {
    /// HACK: 填充 NLA 编辑器测试数据（临时；与 `create_test_animation_data` 配套）。
    ///
    /// 创建 `AnimData`（存 `animation_library.anim_datas`）并装载到 NLA 编辑器，
    /// 轨道引用上面创建的 Action（主 Action/奔跑/跳跃）。正确路径（未来）：
    /// 用户通过"向下插入"把编辑中的 Action 加入 NLA。
    /// 归属动画域 NLA 子集（`store/animation/nla`），由 `store/animation/graph::create_test_animation_data` 调用。
    pub(crate) fn create_test_nla_data(&mut self) {
        use aweft_model::animation::AnimData;
        use aweft_model::animation::strip::{NlaStrip, NlaTrack, StripPlacement, StripType};

        let main_id = self.ui_models.animation.graph.actions[0].id;
        let run_id = self.ui_models.animation.graph.actions[1].id;
        let jump_id = self.ui_models.animation.graph.actions[2].id;

        let mut anim_data = AnimData::new("主 NLA");
        anim_data.set_base_action(main_id);

        // 轨道 1：Idle（引用主 Action 的片断）
        let mut t1 = NlaTrack::new("Idle");
        let mut s1 = NlaStrip::new_action("主 Action", main_id);
        s1.placement = StripPlacement::Absolute {
            start: 0.0,
            end: 15.0,
        };
        t1.strips = vec![s1];

        // 轨道 2：动作（奔跑 + 跳跃 两个片断）
        let mut t2 = NlaTrack::new("动作");
        let mut s2 = NlaStrip::new_action("奔跑", run_id);
        s2.placement = StripPlacement::Absolute {
            start: 2.0,
            end: 6.0,
        };
        let mut s3 = NlaStrip::new_action("跳跃", jump_id);
        s3.placement = StripPlacement::Absolute {
            start: 8.0,
            end: 12.0,
        };
        t2.strips = vec![s2, s3];

        // 轨道 3：Meta（组合片断，含过渡占位）
        let mut t3 = NlaTrack::new("Meta");
        let m1 = NlaStrip {
            name: "组合".into(),
            placement: StripPlacement::Absolute {
                start: 1.0,
                end: 13.0,
            },
            strip_type: StripType::Meta { strips: Vec::new() },
            ..NlaStrip::new_action("组合", run_id)
        };
        t3.strips = vec![m1];

        anim_data.tracks = vec![t1, t2, t3];
        let anim_data_id = self
            .state
            .project
            .library
            .animation_library
            .insert_anim_data(anim_data);
        self.ui_models.animation.nla.anim_data_id = Some(anim_data_id);
        self.load_nla_from_model();
    }
}
