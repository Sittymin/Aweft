use std::collections::HashMap;

use serde::{Deserialize, Serialize};
use thiserror::Error;

use crate::animation::action::Action;
use crate::animation::anim_data::AnimData;
use crate::animation::fcurve::FCurve;
use crate::animation::sample::{sample_anim_data, ActionProvider, EvaluatedStrip};
use crate::animation::strip::{NlaStrip, NlaTrack, StripPlacement};
use crate::animation::PropertyPath;
use crate::ids::{ActionId, AnimDataId};

/// The animation library stores both reusable `Action`s (FCurve sets)
/// and `AnimData`s (NLA track configurations).
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct AnimationLibrary {
    pub actions: HashMap<ActionId, Action>,
    pub anim_datas: HashMap<AnimDataId, AnimData>,
}

/// Error type for animation sampling.
#[derive(Debug, Clone, Error)]
pub enum AnimationError {
    #[error("动画数据 {0} 不存在")]
    MissingAnimData(AnimDataId),
    #[error("动画动作 {0} 不存在")]
    MissingAction(ActionId),
    #[error("动画数据 {0} 没有基础编辑 Action，无法下压")]
    NoBaseAction(AnimDataId),
}

impl AnimationLibrary {
    pub fn insert_action(&mut self, action: Action) -> ActionId {
        let id = action.id;
        self.actions.insert(id, action);
        id
    }

    pub fn insert_anim_data(&mut self, anim_data: AnimData) -> AnimDataId {
        let id = anim_data.id;
        self.anim_datas.insert(id, anim_data);
        id
    }

    /// Sample an anim data at the given event time and duration.
    /// Returns a list of evaluated strips (with blend_mode and influence).
    pub fn sample_tracks(
        &self,
        anim_data_id: AnimDataId,
        time: f64,
        target_duration: f64,
    ) -> Result<Vec<EvaluatedStrip>, AnimationError> {
        let anim_data = self
            .anim_datas
            .get(&anim_data_id)
            .ok_or(AnimationError::MissingAnimData(anim_data_id))?;
        Ok(sample_anim_data(anim_data, time, target_duration, self))
    }

    /// Sample and blend all strips, returning a flat property→value map.
    pub fn sample(
        &self,
        anim_data_id: AnimDataId,
        time: f64,
        target_duration: f64,
    ) -> Result<HashMap<PropertyPath, f64>, AnimationError> {
        let strips = self.sample_tracks(anim_data_id, time, target_duration)?;
        Ok(crate::animation::sample::blend_strip_results(&strips))
    }

    /// Get an Action by ID.
    pub fn get_action(&self, action_id: ActionId) -> Option<&Action> {
        self.actions.get(&action_id)
    }

    /// 把"基础编辑 Action"下压为 NLA 轨道上的一个片断，并重置编辑工作区。
    ///
    /// - 要求 `anim_data.base_action` 存在（`None` 时返回 `NoBaseAction`）；
    /// - 新建片断以 `no_time_map = true` 精确还原"按绝对时间求值"的行为，
    ///   保证下压前后同一时刻的采样结果一致；
    /// - 片断压入最底层（底部）新轨道，随后 `set_base_action` 指向新建的
    ///   空 Action，供继续下一段编辑。
    ///
    /// 返回新的"基础编辑 Action"的 `ActionId`。
    pub fn push_down_editing_action(
        &mut self,
        anim_data_id: AnimDataId,
    ) -> Result<ActionId, AnimationError> {
        let action_id = self
            .anim_datas
            .get(&anim_data_id)
            .ok_or(AnimationError::MissingAnimData(anim_data_id))?
            .base_action
            .ok_or(AnimationError::NoBaseAction(anim_data_id))?;

        let action_name = self
            .actions
            .get(&action_id)
            .map(|a| a.name.clone())
            .unwrap_or_else(|| "action".to_string());

        let strip = NlaStrip {
            name: action_name.clone(),
            placement: StripPlacement::Full,
            no_time_map: true,
            ..NlaStrip::new_action(action_name.clone(), action_id)
        };

        if let Some(anim_data) = self.anim_datas.get_mut(&anim_data_id) {
            let track = NlaTrack {
                name: action_name.clone(),
                muted: false,
                solo: false,
                strips: vec![strip],
            };
            anim_data.tracks.insert(0, track);
        }

        // 重置编辑工作区：新建一份空 Action 供继续编辑
        let fresh = Action::new(action_name);
        let fresh_id = fresh.id;
        self.actions.insert(fresh_id, fresh);
        if let Some(anim_data) = self.anim_datas.get_mut(&anim_data_id) {
            anim_data.set_base_action(fresh_id);
        }

        Ok(fresh_id)
    }
}

// ── ActionProvider impl (allows sample_anim_data to resolve actions) ──────

impl ActionProvider for AnimationLibrary {
    fn get_fcurves(
        &self,
        action_id: ActionId,
    ) -> Option<Vec<(PropertyPath, FCurve)>> {
        let action = self.actions.get(&action_id)?;
        Some(
            action
                .curves
                .iter()
                .map(|ac| (ac.property.clone(), ac.curve.clone()))
                .collect(),
        )
    }
}
