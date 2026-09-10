use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};

/// 插件目录里的清单文件名。`PluginManager::load_dir` 按这个文件名认插件目录。
pub const MANIFEST_FILE: &str = "aweft-plugin.json";

/// 插件能力申请。宿主只链接申请过的入口：清单没写的字段一律 `false`（少写=少给）。
///
/// 规则是"改了真相才要申请"：读行数与起止时间、写自己的状态行与日志不必申请，
/// 未授予 `project_write` 时宿主根本不链接 `set_event_*_ms`，插件 import 它就在装载期失败。
/// `network`/`fs_read`/`fs_write` 仍是申报项——宿主还没有这三类进口，先不假装已经管住了。
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(default)]
pub struct Permissions {
    pub network: bool,
    pub fs_read: bool,
    pub fs_write: bool,
    /// 允许写回工程真相（字幕行起止时间）。
    pub project_write: bool,
}

/// 插件清单：静态声明插件身份、构件位置与能力申请。
/// 与 VSCode `contributes` 同思路：宿主只开放清单声明过的能力。
/// 注意：插件的功能（命令标签）不由清单声明，一律由 Guest 的描述 JSON 自述，
/// 清单只负责指到目录里的文件。
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PluginManifest {
    pub id: String,
    pub name: String,
    pub version: String,
    /// Guest WASM 字节码，相对插件目录。
    pub guest: String,
    /// 面板 Slint 片段，相对插件目录。
    pub panel: String,
    pub permissions: Permissions,
}

impl PluginManifest {
    pub fn from_json(text: &str) -> Result<Self> {
        serde_json::from_str(text).context("插件清单解析失败")
    }

    /// 读 `<dir>/aweft-plugin.json`。
    pub fn load_dir(dir: &Path) -> Result<Self> {
        let path = dir.join(MANIFEST_FILE);
        let text = std::fs::read_to_string(&path)
            .with_context(|| format!("插件清单读取失败：{}", path.display()))?;
        Self::from_json(&text)
            .with_context(|| format!("插件清单无效：{}", path.display()))
    }

    pub fn guest_path(&self, dir: &Path) -> PathBuf {
        dir.join(&self.guest)
    }

    pub fn panel_path(&self, dir: &Path) -> PathBuf {
        dir.join(&self.panel)
    }

    /// 读 Guest 字节码。
    pub fn read_guest(&self, dir: &Path) -> Result<Vec<u8>> {
        let path = self.guest_path(dir);
        std::fs::read(&path).with_context(|| format!("Guest 字节码读取失败：{}", path.display()))
    }

    /// 读面板片段源码。
    pub fn read_panel(&self, dir: &Path) -> Result<String> {
        let path = self.panel_path(dir);
        std::fs::read_to_string(&path)
            .with_context(|| format!("面板片段读取失败：{}", path.display()))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn manifest_declares_file_locations_and_permissions() {
        let manifest = PluginManifest::from_json(
            r#"{ "id":"p","name":"P","version":"1.0","guest":"g.wasm","panel":"p.slint",
                 "permissions":{"network":false,"fs_read":false,"fs_write":true} }"#,
        )
        .expect("清单解析失败");
        assert_eq!(manifest.id, "p");
        assert_eq!(manifest.guest_path(Path::new("/plugins/p")), PathBuf::from("/plugins/p/g.wasm"));
        assert_eq!(manifest.panel_path(Path::new("/plugins/p")), PathBuf::from("/plugins/p/p.slint"));
        assert!(manifest.permissions.fs_write);
        assert!(!manifest.permissions.network);
        // 少写=少给：清单里没申请的字段一律按不授予处理。
        assert!(!manifest.permissions.project_write);
        let granted = PluginManifest::from_json(
            r#"{ "id":"p","name":"P","version":"1.0","guest":"g.wasm","panel":"p.slint",
                 "permissions":{"project_write":true} }"#,
        )
        .expect("清单解析失败");
        assert!(granted.permissions.project_write);
    }
}
