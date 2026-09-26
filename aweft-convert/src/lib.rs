pub mod animate_to_ass;
pub use animate_to_ass::{convert_anim_data, AnimAssResult, Cursor, ProgressEvent, TimedTags};
pub mod parse_srt;
pub(crate) mod segmentation;
