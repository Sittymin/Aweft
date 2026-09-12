//! 设置文件的读写。

use std::fs;
use std::io::{self, Write};
use std::path::{Path, PathBuf};

use anyhow::{bail, Context, Result};

use crate::{Settings, AWEFT_SETTINGS_FORMAT};

pub const SETTINGS_FILE: &str = "aweft-settings.json";

pub fn file_path(dir: &Path) -> PathBuf {
    dir.join(SETTINGS_FILE)
}

/// 读设置。文件不存在按出厂默认处理（首次启动就是这个状态，不是异常）。
pub fn load(dir: &Path) -> Result<Settings> {
    let path = file_path(dir);
    let text = match fs::read_to_string(&path) {
        Ok(text) => text,
        Err(error) if error.kind() == io::ErrorKind::NotFound => return Ok(Settings::default()),
        Err(error) => {
            return Err(error).with_context(|| format!("设置文件读取失败：{}", path.display()))
        }
    };
    let mut settings: Settings =
        serde_json::from_str(&text).with_context(|| format!("设置文件无效：{}", path.display()))?;

    if settings.format > AWEFT_SETTINGS_FORMAT {
        bail!(
            "设置文件版本 {} 比本程序支持的 {} 新，拒绝改写：{}",
            settings.format,
            AWEFT_SETTINGS_FORMAT,
            path.display()
        );
    }
    // 比当前旧的版本在这里逐级补写。加字段不用管：`Settings` 上挂了
    // `#[serde(default)]`，缺的字段自动取默认值；只有"旧值必须改写才有意义"
    // 的改动才需要在这条链上加一步。
    settings.format = AWEFT_SETTINGS_FORMAT;
    Ok(settings)
}

/// 原子写：先写同目录的临时文件并刷盘，再 rename 盖上去。
pub fn save(dir: &Path, settings: &Settings) -> Result<()> {
    fs::create_dir_all(dir).with_context(|| format!("设置目录创建失败：{}", dir.display()))?;
    let path = file_path(dir);
    let text = serde_json::to_string_pretty(settings)
        .with_context(|| format!("设置序列化失败：{}", path.display()))?;

    // 临时文件带进程号：命令行与界面可能同时在写同一份设置。
    let tmp = path.with_extension(format!("json.tmp-{}", std::process::id()));
    let result = (|| -> Result<()> {
        let mut file =
            fs::File::create(&tmp).with_context(|| format!("临时设置文件创建失败：{tmp:?}"))?;
        file.write_all(text.as_bytes())
            .with_context(|| format!("临时设置文件写入失败：{tmp:?}"))?;
        // sync 必须在 rename 之前：没刷盘的临时文件被 rename 上去之后，进程被杀
        // 或掉电会留下一个 0 字节的设置文件，下次启动就读不到设置了。
        file.sync_all()
            .with_context(|| format!("临时设置文件刷盘失败：{tmp:?}"))?;
        fs::rename(&tmp, &path)
            .with_context(|| format!("设置文件替换失败：{:?} -> {}", tmp, path.display()))
    })();

    if result.is_err() {
        // 失败路径上清掉半成品，别让目录里堆垃圾；清理本身失败不值得再盖掉原始错误。
        let _ = fs::remove_file(&tmp);
    }
    result
}
