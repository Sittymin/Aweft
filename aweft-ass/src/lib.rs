mod parser;
mod processor;
pub mod types;
mod utils;

use std::{io, path::Path};

use crate::processor::{merge_ass_files_with_offsets, FileWithOffset};
pub use crate::types::ASSFile;

pub fn load_ass(path: impl AsRef<Path>) -> io::Result<ASSFile> {
    ASSFile::parse_from_file(&path.as_ref().to_path_buf())
}

pub fn save_ass(ass: &ASSFile, path: impl AsRef<Path>) -> io::Result<()> {
    ass.write_to_file_buffered(&path.as_ref().to_path_buf(), 1000)
}

pub fn merge_ass_files<'a>(
    files_with_offsets: Vec<(&'a Path, i32, i32, i32, i32)>,
    index: usize,
) -> io::Result<ASSFile> {
    let files = files_with_offsets
        .into_iter()
        .map(|(path, h, m, s, cs)| FileWithOffset::new(path, h, m, s, cs))
        .collect();

    merge_ass_files_with_offsets(files, index)
}
