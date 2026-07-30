use std::collections::HashMap;

use serde::{Deserialize, Serialize};
use thiserror::Error;

use crate::animation::action::Action;
use crate::animation::anim_data::AnimData;
use crate::animation::fcurve::FCurve;
use crate::animation::sample::{sample_anim_data, ActionProvider, EvaluatedStrip};
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
