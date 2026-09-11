//! logcat 落地：`log` facade 的实现，标签固定 `aweft`。
//!
//! 为什么必须自己写：`aweft-gui` 桌面侧用 env_logger 往 stderr 写，而 Android 上
//! 应用进程的 stdout/stderr 不进 logcat；GStreamer 的符号缺失、管线协商失败这些
//! 只能从 logcat 看，没有它等于蒙着眼排错。

use log::{Level, LevelFilter, Metadata, Record};

const ANDROID_LOG_VERBOSE: i32 = 2;
const ANDROID_LOG_DEBUG: i32 = 3;
const ANDROID_LOG_INFO: i32 = 4;
const ANDROID_LOG_WARN: i32 = 5;
const ANDROID_LOG_ERROR: i32 = 6;

unsafe extern "C" {
    fn __android_log_write(prio: i32, tag: *const i8, text: *const i8) -> i32;
}

const TAG: &[u8] = b"aweft\0";

fn write_line(prio: i32, text: &str) {
    // logcat 按行显示，多行的消息拆开写，否则第二行之后整条丢掉。
    for line in text.lines() {
        let owned = line.as_bytes();
        let mut buf = Vec::with_capacity(owned.len() + 1);
        buf.extend_from_slice(owned);
        buf.push(0);
        unsafe {
            __android_log_write(
                prio,
                TAG.as_ptr().cast::<i8>(),
                buf.as_ptr().cast::<i8>(),
            );
        }
    }
}

struct Logcat;

/// 与桌面 env_logger 那条默认过滤器同构：本 workspace 的 debug+，其余 warn+。
/// 手机上没有 RUST_LOG 可设，所以规则写死在这里。
fn enabled_for(level: Level, target: &str) -> bool {
    let ceiling = if target.starts_with("aweft") {
        LevelFilter::Debug
    } else {
        LevelFilter::Warn
    };
    level <= ceiling
}

impl log::Log for Logcat {
    fn enabled(&self, metadata: &Metadata) -> bool {
        enabled_for(metadata.level(), metadata.target())
    }

    fn log(&self, record: &Record) {
        if !enabled_for(record.level(), record.target()) {
            return;
        }
        let prio = match record.level() {
            Level::Error => ANDROID_LOG_ERROR,
            Level::Warn => ANDROID_LOG_WARN,
            Level::Info => ANDROID_LOG_INFO,
            Level::Debug => ANDROID_LOG_DEBUG,
            Level::Trace => ANDROID_LOG_VERBOSE,
        };
        write_line(prio, &format!("{}: {}", record.target(), record.args()));
    }

    fn flush(&self) {}
}

/// 装好 logger。框架总闸放到 Trace，级别的取舍只在 `enabled_for` 一处。
/// 只该被 `android_main` 调一次，`set_logger` 第二次会返回 Err 并被忽略。
pub(crate) fn install() {
    if log::set_boxed_logger(Box::new(Logcat)).is_ok() {
        log::set_max_level(LevelFilter::Trace);
    }
}
