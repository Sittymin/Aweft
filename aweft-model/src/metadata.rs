use serde::{Deserialize, Serialize};
use std::path::PathBuf;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ProjectMetadata {
    pub title: String,
    pub author: String,
    pub description: String,
    pub audio_path: Option<PathBuf>,
}

impl Default for ProjectMetadata {
    fn default() -> Self {
        Self {
            title: "New Project".to_string(),
            author: "Unknown".to_string(),
            description: String::new(),
            audio_path: None,
        }
    }
}
