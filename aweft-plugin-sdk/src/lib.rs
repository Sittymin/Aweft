//! Aweft 插件 Guest 侧 SDK。插件 crate 编到 `wasm32-unknown-unknown`，
//! 用 [`plugin!`] 交出导出；宿主函数声明与描述 JSON 的胶水都由本文件生成。
//!
//! 插件作者只写两样东西：`static COMMANDS` 与命令处理函数。
//!
//! ```ignore
//! use aweft_plugin_sdk::{Api, Command};
//!
//! fn earliest(api: &Api, _id: i32) -> i32 {
//!     let best = (0..api.event_count()).map(|h| api.start_ms(h)).min();
//!     api.set_status(0, &format!("最早 {:?}", best));
//!     0
//! }
//!
//! static COMMANDS: &[Command] = &[Command::new("计算最早时间", earliest)];
//!
//! aweft_plugin_sdk::plugin! { commands: COMMANDS }
//! ```
//!
//! 显示文本一律由插件自己组装后写进面板状态行，宿主不做模板替换、
//! 不格式化时间，因此换任何语言绑定都不会改动宿主代码。
//!
//! 依赖方向：本 crate 不被主程序引用，只被插件 crate 引用。

use std::fmt::Write as _;

// 宿主函数所在的 wasm 导入模块名，与 `aweft_plugins::runtime::HOST_MODULE` 对应。
#[link(wasm_import_module = "aweft")]
extern "C" {
    fn event_count() -> i32;
    fn event_start_ms(handle: i32) -> i64;
    fn event_end_ms(handle: i32) -> i64;
    fn set_event_start_ms(handle: i32, ms: i64) -> i32;
    fn set_event_end_ms(handle: i32, ms: i64) -> i32;
    fn set_status_line(line: i32, ptr: *const u8, len: i32);
    fn log_text(ptr: *const u8, len: i32);
}

/// 一条命令：按钮文本与处理函数。命令编号就是它在 `COMMANDS` 里的下标。
pub struct Command {
    label: &'static str,
    handler: fn(&Api, i32) -> i32,
}

impl Command {
    pub const fn new(label: &'static str, handler: fn(&Api, i32) -> i32) -> Self {
        Self { label, handler }
    }

    /// 按钮文本。
    pub fn label(&self) -> &'static str {
        self.label
    }

    /// 处理函数。`plugin!` 生成的导出经它分发命令编号。
    pub fn handler(&self) -> fn(&Api, i32) -> i32 {
        self.handler
    }
}

/// 行句柄、面板状态行与日志的读写入口，由宿主快照支撑。
///
/// 句柄是宿主按「开始时间、再按行 id」排定的行序下标。时间写入先落在宿主快照，
/// 本次调用返回时由宿主一次性差分写回工程存储，因此读能立刻看到自己刚写的值。
#[derive(Debug, Clone, Copy, Default)]
pub struct Api {
    _priv: (),
}

impl Api {
    #[doc(hidden)]
    pub fn new() -> Self {
        Self { _priv: () }
    }

    /// 当前行数。
    pub fn event_count(&self) -> i32 {
        unsafe { event_count() }
    }

    /// 某行开始时间（毫秒）。越界句柄返回 `-1`。
    pub fn start_ms(&self, handle: i32) -> i64 {
        unsafe { event_start_ms(handle) }
    }

    /// 某行结束时间（毫秒）。越界句柄返回 `-1`。
    pub fn end_ms(&self, handle: i32) -> i64 {
        unsafe { event_end_ms(handle) }
    }

    /// 写某行开始时间（毫秒）。句柄越界时返回 `false`，写入被丢弃。
    /// 钳制由宿主负责：开始时间不越过结束时间。
    pub fn set_start_ms(&self, handle: i32, ms: i64) -> bool {
        unsafe { set_event_start_ms(handle, ms) == 0 }
    }

    /// 写某行结束时间（毫秒）。句柄越界时返回 `false`，写入被丢弃。
    pub fn set_end_ms(&self, handle: i32, ms: i64) -> bool {
        unsafe { set_event_end_ms(handle, ms) == 0 }
    }

    /// 往自己面板的第 `line` 行写显示文本。行号超出已有行数时宿主追加空行。
    pub fn set_status(&self, line: i32, text: &str) {
        unsafe { set_status_line(line, text.as_ptr(), text.len() as i32) }
    }

    /// 写一行宿主日志（诊断用，不显示给用户）。
    pub fn log(&self, message: &str) {
        unsafe { log_text(message.as_ptr(), message.len() as i32) }
    }
}

/// 生成描述 JSON：`{"abi":1,"commands":["标签"],"events":["time_changed"]}`。
/// 命令编号即数组下标；标签做 JSON 转义，非 ASCII 原样保留。
pub fn describe_json(commands: &[Command], time_changed: bool) -> String {
    fn push_escaped(out: &mut String, text: &str) {
        for ch in text.chars() {
            match ch {
                '"' => out.push_str("\\\""),
                '\\' => out.push_str("\\\\"),
                '\n' => out.push_str("\\n"),
                '\r' => out.push_str("\\r"),
                '\t' => out.push_str("\\t"),
                c if (c as u32) < 0x20 => {
                    let _ = write!(out, "\\u{:04x}", c as u32);
                }
                c => out.push(c),
            }
        }
    }

    let mut out = String::from("{\"abi\":1,\"commands\":[");
    for (index, command) in commands.iter().enumerate() {
        if index > 0 {
            out.push(',');
        }
        out.push('"');
        push_escaped(&mut out, command.label);
        out.push('"');
    }
    out.push_str("],\"events\":[");
    if time_changed {
        out.push_str("\"time_changed\"");
    }
    out.push_str("]}");
    out
}

/// 生成插件的全部导出。宿主与插件之间只有这几个名字：
/// `aweft_describe_ptr` / `aweft_describe_len` 指向一段 UTF-8 JSON 描述；
/// `aweft_on_command(id)` 执行命令，返回 0 表示成功；
/// `aweft_on_event_time(handle, start_ms, end_ms)` 只在订阅时生成。
#[macro_export]
macro_rules! plugin {
    ( commands: $commands:path, on_time_changed: $handler:path $(,)? ) => {
        $crate::__plugin_describe!($commands, true);
        $crate::__plugin_command_export!($commands);

        #[no_mangle]
        pub extern "C" fn aweft_on_event_time(handle: i32, start_ms: i64, end_ms: i64) {
            $handler(&$crate::Api::new(), handle, start_ms, end_ms);
        }
    };
    ( commands: $commands:path $(,)? ) => {
        $crate::__plugin_describe!($commands, false);
        $crate::__plugin_command_export!($commands);
    };
}

/// 描述导出：字符串只在首次调用时组装一次，之后地址必须稳定，
/// 因此把 `String` 的所有权交给进程（宿主装载期一次性读取，无释放时机）。
#[doc(hidden)]
#[macro_export]
macro_rules! __plugin_describe {
    ( $commands:path, $time_changed:expr ) => {
        fn aweft_descriptor() -> &'static str {
            static DESCRIPTOR: ::std::sync::OnceLock<&'static str> =
                ::std::sync::OnceLock::new();
            *DESCRIPTOR.get_or_init(|| {
                ::std::boxed::Box::leak(
                    $crate::describe_json($commands, $time_changed).into_boxed_str(),
                )
            })
        }

        #[no_mangle]
        pub extern "C" fn aweft_describe_ptr() -> i32 {
            aweft_descriptor().as_ptr() as i32
        }

        #[no_mangle]
        pub extern "C" fn aweft_describe_len() -> i32 {
            aweft_descriptor().len() as i32
        }
    };
}

#[doc(hidden)]
#[macro_export]
macro_rules! __plugin_command_export {
    ( $commands:path ) => {
        #[no_mangle]
        pub extern "C" fn aweft_on_command(id: i32) -> i32 {
            match $commands.get(id as usize) {
                Some(command) => {
                    let handler = command.handler();
                    handler(&$crate::Api::new(), id)
                }
                None => 1,
            }
        }
    };
}

#[cfg(test)]
mod tests {
    use super::*;

    fn noop(_: &Api, _: i32) -> i32 {
        0
    }

    #[test]
    fn descriptor_lists_escaped_labels() {
        let commands = [Command::new("带\"引号\"的标签", noop)];
        assert_eq!(
            describe_json(&commands, true),
            r#"{"abi":1,"commands":["带\"引号\"的标签"],"events":["time_changed"]}"#
        );
    }

    #[test]
    fn descriptor_without_subscription_has_empty_events() {
        assert_eq!(
            describe_json(&[], false),
            r#"{"abi":1,"commands":[],"events":[]}"#
        );
    }
}
