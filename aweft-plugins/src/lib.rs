//! 插件运行时：目录装载 + WASM Guest + 宿主函数 + 面板编译。
//!
//! - `manifest`：插件清单（身份、构件位置、能力申请）。
//! - `runtime`：wasmi 装载 Guest，读 Guest 自描述的命令与订阅表并调用导出。
//! - `worker`：每插件一个线程，独占其 Guest；与客户端之间只有两条通道。
//! - `manager`：插件装载、命令分发（只发送）、事件落盘（`apply_event` 写回真相）。
//! - `ui`：插件面板 Slint 片段的运行时编译与窗口填值。
//! - `ctx`：宿主快照上下文（模块内部使用）。
//!
//! 与客户端解耦的边界：数据真相与事件 pump 都在使用方手里（图形界面、命令行、
//! 测试各有一套）。本成员只发事件（`WorkerEvent`）、收 job，`run_command` 不等
//! Guest 跑完，`apply_event` 只做写回与缓存——pump 归各客户端自己，
//! 图形界面只是其中一个客户端。
//! 变更通知由存储自己广播，本成员不维护投递队列。
//!
//! 依赖方向：本成员依赖 `aweft-model` 与 `aweft-project`；
//! 图形界面与命令行依赖本成员。本成员不得使用 `slint!` 编译期宏与 `gstreamer`。

mod ctx;
pub mod manager;
pub mod manifest;
pub mod runtime;
pub mod ui;
mod worker;

pub use manager::{LoadedPlugin, PluginManager};
pub use manifest::{Permissions, PluginManifest, MANIFEST_FILE};
pub use runtime::{GuestDescriptor, ABI_VERSION, EVENT_TIME_CHANGED, HOST_MODULE};
pub use ui::{
    compile_panel, compile_panel_file, PanelHandle, COMMAND_BUSY_PROP, COMMAND_CLICKED_CALLBACK,
    COMMAND_LABELS_PROP, STATUS_LINES_PROP,
};
pub use worker::{PluginEvent, RowSnap, WorkerEvent};
