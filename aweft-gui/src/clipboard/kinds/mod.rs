//! 业务侧剪贴板数据结构集合。
//!
//! 当前：
//! - `items`：字幕条目（Event/Group 子树）
//!
//! 未来扩展（每种 = 一个文件 + 一个 struct）：
//! - `style`：样式
//! - `node_graph`：节点图
//! - `timecode`：单个时间码

pub mod items;

pub use items::ItemsClip;
