use std::{io::Error, path::PathBuf};

use crate::types::*;

struct FileWithOffset {
    file_path: String,
    offset_hours: i32,
    offset_minutes: i32,
    offset_seconds: i32,
    offset_centiseconds: i32,
}

// TODO: Test this
fn merge_ass_files_with_offsets(files: Vec<FileWithOffset>) -> Result<String, Error> {
    if files.is_empty() {
        return Err(Error::new(
            std::io::ErrorKind::InvalidInput,
            "No files provided for merging.",
        ));
    }
    let first_file = &files[0];
    let mut merged_ass = ASSFile::parse_from_file(&PathBuf::from(&first_file.file_path))?;

    for event in &mut merged_ass.events {
        event.adjust_time(
            first_file.offset_hours,
            first_file.offset_minutes,
            first_file.offset_seconds,
            first_file.offset_centiseconds,
            TimeAdjustTarget::Both,
        );
    }

    for file in &files[1..] {
        let path = PathBuf::from(&file.file_path);
        let ass_file = ASSFile::parse_from_file(&path)?;
        for mut event in ass_file.events {
            event.adjust_time(
                file.offset_hours,
                file.offset_minutes,
                file.offset_seconds,
                file.offset_centiseconds,
                TimeAdjustTarget::Both,
            );
        }
    }
    Ok(format!("Merge ass in first file: {}", first_file.file_path))
}
