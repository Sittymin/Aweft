//! 按领域组织的 reducer：
//!
//! - `area` / `drag` / `screen` / `media`：纯函数（`handle_*`，只改 `AppState`）+ `dispatch_*` 包装
//! - `line_editor` / `animation` / `node_graph`：`impl AppStore` 方法（改 Domain + UI 态，返回精确 tag）

pub mod animation;
pub mod area;
pub mod drag;
pub mod line_editor;
pub mod media;
pub mod nla;
pub mod node_graph;
pub mod screen;
