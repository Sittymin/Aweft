use anyhow::{anyhow, Result};
use chrono::TimeDelta;
use std::path::{Path, PathBuf};

use crate::types::*;

pub struct FileWithOffset<'a> {
    file_path: &'a Path,
    offset_time: TimeDelta,
}

// TODO: implemented this

// pub fn merge_ass_files_with_offsets(files: Vec<FileWithOffset>, index: usize) -> Result<ASSFile> {
//     if files.is_empty() {
//         return Err(anyhow!("No files provided for merging.",));
//     }

//     let main_file = if index > files.len() {
//         &files[0]
//     } else {
//         &files[index]
//     };
//     let mut main_ass = ASSFile::parse_from_file(&PathBuf::from(&main_file.file_path))?;

//     for event in &mut main_ass.events {
//         event.adjust_time(
//             main_file.offset_hours,
//             main_file.offset_minutes,
//             main_file.offset_seconds,
//             main_file.offset_centiseconds,
//             TimeAdjustTarget::Both,
//         );
//     }

//     let mut merged_ass = ASSFile::from(&main_ass);

//     let iter = files.iter().enumerate().filter(|&(i, _)| i != index);

//     for (_index, file) in iter {
//         let path = PathBuf::from(&file.file_path);
//         let mut ass_file = ASSFile::parse_from_file(&path)?;

//         merged_ass.merge_styles(&ass_file);

//         for event in &mut ass_file.events {
//             event.adjust_time(
//                 file.offset_hours,
//                 file.offset_minutes,
//                 file.offset_seconds,
//                 file.offset_centiseconds,
//                 TimeAdjustTarget::Both,
//             );
//         }
//         merged_ass.events.extend(ass_file.events);
//     }
//     Ok(merged_ass)
// }
