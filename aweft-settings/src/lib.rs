//! 应用设置。不依赖 Slint，也不依赖任何具体平台的目录约定。
//!
//! - `settings`（本文件）：`Settings` 结构与文件格式版本。
//! - `location`：配置目录怎么定（显式覆盖 → 平台目录），以及进程内的那一份。
//! - `storage`：读写落盘，原子写与版本门。
//!
//! 分层规矩（详见 `docs/architecture/settings.md`）：
//! - 只有界面用得上的字段将来放进独立的 `gui` 段，命令行只读非 `gui` 段；
//!   现在还没有这样的字段，所以没有空段。
//! - 插件自己的偏好将来放 `plugins` 段（键为插件 id，值为不透明 JSON），
//!   宿主字段不为插件增长。
//! - 设置改动没有撤销：只有 `is_dirty`、显式保存、以及"放弃改动＝重新读盘"。
//!
//! 依赖方向：本成员不依赖任何其它成员；图形界面、命令行、Android 入口依赖本成员。

mod location;
mod storage;

use std::path::PathBuf;
use std::sync::OnceLock;

use serde::{Deserialize, Serialize};

pub use location::{config_dir, platform_config_dir, set_config_dir, SETTINGS_DIR_ENV};

/// 设置文件的格式版本。加字段不必 bump（serde 的 `default` 兜住旧文件）；
/// 只有"旧值需要改写才有意义"的改动才 bump，并在 `storage::migrate` 里加一步。
pub const AWEFT_SETTINGS_FORMAT: u32 = 1;

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct Settings {
    /// 文件自身的格式版本。缺字段的旧文件按 0 处理，`migrate` 之后盖成当前值。
    pub format: u32,
    pub paths: Paths,
}

#[derive(Debug, Clone, PartialEq, Default, Serialize, Deserialize)]
#[serde(default)]
pub struct Paths {
    /// 插件根目录：下面每一层是一个插件目录（含 `aweft-plugin.json`）。
    /// 没配则只走各入口的默认候选，见 `aweft-gui` 的 `plugin_roots`。
    pub plugin_root: Option<PathBuf>,
}

impl Default for Settings {
    /// 出厂默认就是 `Default`，"恢复出厂"= 跳过读盘，不存在第二套值。
    fn default() -> Self {
        Self {
            format: AWEFT_SETTINGS_FORMAT,
            paths: Paths::default(),
        }
    }
}

static SETTINGS: OnceLock<Settings> = OnceLock::new();

/// 进程内唯一的一份设置：首次访问时按配置目录读盘，之后一直返回同一份。
///
/// 顺序约束：入口必须在任何一次 `settings()` 之前调 `set_config_dir`，
/// 否则这里缓存的是错目录（或"目录未定"）的结果，后提交不再生效。
/// 桌面在 `src/main.rs`、Android 在 `android_main` 里都是进界面之前先定目录。
pub fn settings() -> &'static Settings {
    SETTINGS.get_or_init(Settings::load_default)
}

impl Settings {
    /// 用进程当前配置目录读设置。目录还没定（入口没调 `set_config_dir`）、
    /// 文件读不出来或解析失败，都退回出厂值并留一条警告：设置坏了不该挡住启动。
    fn load_default() -> Settings {
        match config_dir() {
            None => {
                log::warn!("配置目录还没定（入口未调 set_config_dir），设置按出厂值");
                Settings::default()
            }
            Some(dir) => match storage::load(&dir) {
                Ok(settings) => settings,
                Err(error) => {
                    log::warn!("设置读取失败，按出厂值继续：{error:#}");
                    Settings::default()
                }
            },
        }
    }

    /// 写盘到指定目录。设置界面接上之前，生产路径上没有调用点，
    /// 手工编辑 `aweft-settings.json` 是当前唯一的写入口。
    pub fn save_to_dir(&self, dir: &std::path::Path) -> anyhow::Result<()> {
        storage::save(dir, self)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn default_carries_current_format() {
        assert_eq!(Settings::default().format, AWEFT_SETTINGS_FORMAT);
    }

    #[test]
    fn round_trip_keeps_plugin_root() {
        let dir = tempfile::tempdir().expect("临时目录");
        let mut settings = Settings::default();
        settings.paths.plugin_root = Some(PathBuf::from("/somewhere/plugins"));

        settings.save_to_dir(dir.path()).expect("写入设置文件");
        let loaded = storage::load(dir.path()).expect("读回设置文件");

        assert_eq!(loaded, settings);
    }

    #[test]
    fn missing_file_is_factory_defaults() {
        let dir = tempfile::tempdir().expect("临时目录");
        assert_eq!(
            storage::load(dir.path()).expect("缺文件不是错误"),
            Settings::default()
        );
    }

    #[test]
    fn absent_fields_fall_back_to_defaults() {
        let dir = tempfile::tempdir().expect("临时目录");
        std::fs::write(dir.path().join("aweft-settings.json"), "{}").expect("写入测试文件");

        let loaded = storage::load(dir.path()).expect("旧文件缺字段要能读");
        assert_eq!(loaded, Settings::default());
    }

    #[test]
    fn newer_format_is_rejected() {
        let dir = tempfile::tempdir().expect("临时目录");
        std::fs::write(
            dir.path().join("aweft-settings.json"),
            r#"{"format": 999, "paths": {"plugin_root": null}}"#,
        )
        .expect("写入测试文件");

        let error = storage::load(dir.path()).expect_err("更新的版本必须被拒");
        assert!(
            error.to_string().contains("999"),
            "错误里要点名版本：{error}"
        );
    }

    #[test]
    fn save_leaves_no_temp_file() {
        let dir = tempfile::tempdir().expect("临时目录");
        Settings::default()
            .save_to_dir(dir.path())
            .expect("写入设置文件");

        let leftovers: Vec<String> = std::fs::read_dir(dir.path())
            .expect("列目录")
            .filter_map(|entry| entry.ok())
            .map(|entry| entry.file_name().to_string_lossy().into_owned())
            .collect();
        assert_eq!(leftovers, vec!["aweft-settings.json".to_string()]);
    }
}
