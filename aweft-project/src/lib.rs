//! 存盘工程形状的唯一所有者。
//!
//! - `saved`：磁盘与协作快照共用的形状（`Project` + 布局表 + 媒体条目表）与文件格式版本。
//! - `receipt`：写入回执（来源标记 + 变更明细）。
//! - `store`：文档唯一所有权存储；已建模的写入经单一入口并留下回执，
//!   存储自己把回执广播给注册的订阅者（写入即广播，批量作用域可合并）。
//! - `subscriber`：变更订阅者接口与回声过滤的来源约定。
//!
//! 依赖方向：本成员依赖 `aweft-model` 与 `aweft-storage`；图形界面、命令行、插件、
//! 协作依赖本成员。

pub mod receipt;
pub mod saved;
pub mod store;
pub mod subscriber;

pub use receipt::{EventTimeChanged, MutationKind, MutationReceipt, Origin};
pub use saved::{AWEFT_PROJECT_FORMAT, LayoutArea, LayoutTable, MediaEntry, SavedProject};
pub use store::{BatchToken, ProjectStore, MAX_BROADCAST_ROUNDS};
pub use subscriber::{MutationSubscriber, SubscriberRef};
