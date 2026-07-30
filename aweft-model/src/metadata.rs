use serde::{Deserialize, Serialize};
use std::path::PathBuf;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ProjectMetadata {
    pub title: String,
    pub author: String,
    pub description: String,
    pub video_path: Option<PathBuf>,
    pub audio_path: Option<PathBuf>,
}

impl Default for ProjectMetadata {
    fn default() -> Self {
        Self {
            title: "New Project".to_string(),
            author: "Unknown".to_string(),
            description: String::new(),
            video_path: None,
            audio_path: None,
        }
    }
}
