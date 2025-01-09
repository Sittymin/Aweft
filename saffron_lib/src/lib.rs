mod parser;
mod processor;
mod types;
mod utils;
pub use crate::types::ASSFile;

pub fn load_ass(path: impl AsRef<std::path::Path>) -> std::io::Result<ASSFile> {
    ASSFile::parse_from_file(&path.as_ref().to_path_buf())
}

pub fn save_ass(ass: &ASSFile, path: impl AsRef<std::path::Path>) -> std::io::Result<()> {
    ass.write_to_file_buffered(&path.as_ref().to_path_buf(), 1000)
}
