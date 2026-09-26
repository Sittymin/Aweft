//! 宿主文件系统上的落盘规矩：配置目录怎么定，以及一份带版本戳的 JSON 文件怎么读写。
//!
//! - `location`：配置目录的唯一来源（显式覆盖优先，其次入口交进来的平台目录）。
//! - `json`：原子写、版本门，以及过同一道门的内存编解码。
//! - `restrict_file`、`restrict_dir`：把只该本机用户看见的文件与目录收权限。
//!
//! 分层规矩：本成员不认识任何业务结构体。工程文档、应用设置、协作记录各自的形状由
//! 各自的成员实现 [`json::JsonFile`]，这里只提供一份政策，不提供第二份。
//!
//! 依赖方向：本成员不依赖任何其它成员；`aweft-settings`、`aweft-project`、
//! `aweft-collab`、`aweft-gui` 与两个入口依赖本成员。

pub mod json;
pub mod location;

pub use json::{
    decode_json, encode_json, read_json, read_json_or_default, write_json, write_text_atomic,
    JsonFile,
};
pub use location::{config_dir, platform_config_dir, set_config_dir, USER_CONFIG_ENV};

/// 文件收成本机用户可读写（0600）。密钥、凭证这类文件必须过这一道。
#[cfg(unix)]
pub fn restrict_file(path: &std::path::Path) {
    use std::os::unix::fs::PermissionsExt;
    let _ = std::fs::set_permissions(path, std::fs::Permissions::from_mode(0o600));
}

/// 目录收成本机用户可进出（0700）。0600 的目录连自己进不去，所以两者分开。
#[cfg(unix)]
pub fn restrict_dir(path: &std::path::Path) {
    use std::os::unix::fs::PermissionsExt;
    let _ = std::fs::set_permissions(path, std::fs::Permissions::from_mode(0o700));
}

#[cfg(not(unix))]
pub fn restrict_file(_path: &std::path::Path) {}

#[cfg(not(unix))]
pub fn restrict_dir(_path: &std::path::Path) {}
