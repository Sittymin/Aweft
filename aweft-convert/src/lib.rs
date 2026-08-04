pub mod animate_to_ass;
pub use animate_to_ass::{AnimAssResult, Cursor, ProgressEvent, TimedTags, convert_anim_data};
pub mod parse_srt;
pub(crate) mod segmentation;
