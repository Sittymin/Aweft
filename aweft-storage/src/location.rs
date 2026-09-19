//! 配置目录怎么定：显式覆盖优先，其次由调用方交进来的平台目录。

use std::ffi::OsString;
use std::path::PathBuf;
use std::sync::OnceLock;

/// 覆盖配置目录的环境变量。它是给用户看的配置入口，不是测试开关：落盘文件自己的
/// 位置不能由文件里的字段决定，所以这一层必须留一个进程外的口。
/// 只覆盖目录，不覆盖任何单个设置项。
pub const USER_CONFIG_ENV: &str = "AWEFT_USER_CONFIG";

static CONFIG_DIR: OnceLock<PathBuf> = OnceLock::new();

/// 入口在启动早期调一次，把本平台算出来的目录交进来。之后再调不生效。
pub fn set_config_dir(platform_dir: PathBuf) {
    let dir = resolve(std::env::var_os(USER_CONFIG_ENV), platform_dir);
    if CONFIG_DIR.set(dir).is_err() {
        log::warn!("配置目录已定，忽略第二次提交：现有 {:?}", config_dir());
    }
}

/// 显式覆盖非空时用覆盖值，否则用平台目录。
fn resolve(explicit: Option<OsString>, platform_dir: PathBuf) -> PathBuf {
    match explicit {
        Some(value) if !value.is_empty() => PathBuf::from(value),
        _ => platform_dir,
    }
}

/// 已定下的配置目录；还没人提交过时为 `None`。
pub fn config_dir() -> Option<PathBuf> {
    CONFIG_DIR.get().cloned()
}

/// 桌面/类 Unix 的 `XDG_CONFIG_HOME`/`HOME` → `<配置根>/aweft`。
/// 规范里 `XDG_CONFIG_HOME` 必须是绝对路径，相对或空都按未设置处理。
#[cfg(all(unix, not(target_os = "android")))]
fn config_root(xdg: Option<OsString>, home: Option<OsString>) -> Option<PathBuf> {
    let from_xdg = xdg
        .filter(|value| !value.is_empty())
        .map(PathBuf::from)
        .filter(|path| path.is_absolute());
    let from_home = home
        .filter(|value| !value.is_empty())
        .map(|home| PathBuf::from(home).join(".config"));
    from_xdg.or(from_home)
}

/// Android 上没有可猜的平台目录，只能由 `android_main` 在 `app` 被移走之前把
/// `external_data_path()` 交给 `set_config_dir`；其它目标同理。
#[cfg(not(all(unix, not(target_os = "android"))))]
fn config_root(_xdg: Option<OsString>, _home: Option<OsString>) -> Option<PathBuf> {
    None
}

pub fn platform_config_dir() -> Option<PathBuf> {
    config_root(
        std::env::var_os("XDG_CONFIG_HOME"),
        std::env::var_os("HOME"),
    )
    .map(|root| root.join("aweft"))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn explicit_override_wins() {
        assert_eq!(
            resolve(
                Some(OsString::from("/other-config")),
                PathBuf::from("/platform")
            ),
            PathBuf::from("/other-config")
        );
    }

    #[test]
    fn empty_override_is_ignored() {
        assert_eq!(
            resolve(Some(OsString::new()), PathBuf::from("/platform")),
            PathBuf::from("/platform")
        );
        assert_eq!(
            resolve(None, PathBuf::from("/platform")),
            PathBuf::from("/platform")
        );
    }

    #[cfg(all(unix, not(target_os = "android")))]
    mod config_root {
        use super::super::config_root;
        use std::path::PathBuf;

        fn os(value: &str) -> Option<std::ffi::OsString> {
            Some(std::ffi::OsString::from(value))
        }

        #[test]
        fn absolute_xdg_config_home_is_used() {
            assert_eq!(
                config_root(os("/run/user/1000/config"), os("/home/me")),
                Some(PathBuf::from("/run/user/1000/config"))
            );
        }

        #[test]
        fn relative_or_empty_xdg_config_home_falls_back_to_home() {
            for xdg in ["relative/config", ""] {
                assert_eq!(
                    config_root(os(xdg), os("/home/me")),
                    Some(PathBuf::from("/home/me/.config")),
                    "`XDG_CONFIG_HOME={xdg}` 应按未设置处理"
                );
            }
        }

        #[test]
        fn no_home_no_root() {
            assert_eq!(config_root(None, None), None);
            assert_eq!(config_root(None, os("")), None);
        }
    }
}
