pub mod action;
pub mod anim_data;
pub mod fcurve;
pub mod keyframe;
pub mod library;
pub mod merge;
pub mod modifier;
pub mod noise;
pub mod sample;
pub mod strip;

#[cfg(test)]
mod tests;

pub use action::Action;
pub use anim_data::{AnimData, BlendMode, PropertyPath};
pub use keyframe::{BezierHandle, Extrapolation, Interpolation, Keyframe, merge_keyframe_times};
pub use library::{AnimationError, AnimationLibrary};
pub use merge::apply_animation_to_style;
pub use sample::{ActionProvider, EvaluatedStrip, blend_strip_results, sample_anim_data};
