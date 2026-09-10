//! 插件运行时：目录装载 + WASM Guest + 宿主函数 + 面板编译。
//!
//! - `manifest`：插件清单（身份、构件位置、能力申请）。
//! - `runtime`：wasmi 装载 Guest，读 Guest 自描述的命令与订阅表并调用导出。
//! - `manager`：插件装载、写日志投递、命令执行。
//! - `ui`：插件面板 Slint 片段的运行时编译与窗口填值。
//! - `ctx`：宿主快照上下文（模块内部使用）。
//!
//! 数据真相在使用方手里（图形界面的 `AppStore` 持有 `ProjectStore`）；
//! 本成员只在 Guest 读写期间借用该存储，不复制工程数据。
//!
//! 依赖方向：本成员依赖 `aweft-model` 与 `aweft-project`；
//! 图形界面与命令行依赖本成员。本成员不得使用 `slint!` 编译期宏与 `gstreamer`。

pub mod ctx;
pub mod manager;
pub mod manifest;
pub mod runtime;
pub mod ui;

pub use manager::{LoadedPlugin, MAX_DELIVERY_ROUNDS, PluginManager};
pub use manifest::{MANIFEST_FILE, Permissions, PluginManifest};
pub use runtime::{ABI_VERSION, EVENT_TIME_CHANGED, GuestDescriptor, GuestPlugin, HOST_MODULE};
pub use ui::{
    COMMAND_CLICKED_CALLBACK, COMMAND_LABELS_PROP, STATUS_LINES_PROP, PluginWindow, compile_panel,
    compile_panel_file,
};
