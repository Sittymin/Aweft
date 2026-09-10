//! 统一工程文档的唯一所有者。
//!
//! - `document`：工程文档结构（`Project` + 布局表 + 媒体条目表）与文件格式版本。
//! - `receipt`：写入回执（来源标记 + 变更明细）。
//! - `store`：文档唯一所有权存储；已建模的写入经单一入口并留下回执，
//!   存储自己把回执广播给注册的订阅者（写入即广播，批量作用域可合并）。
//! - `subscriber`：变更订阅者接口与回声过滤的来源约定。
//!
//! 依赖方向：本成员依赖 `aweft-model`；图形界面、命令行、插件、协作依赖本成员。

pub mod document;
pub mod receipt;
pub mod store;
pub mod subscriber;

pub use document::{AweftDocument, LayoutArea, LayoutTable, MediaEntry, AWEFT_DOCUMENT_FORMAT};
pub use receipt::{EventTimeChanged, MutationKind, MutationReceipt, Origin};
pub use store::{BatchToken, ProjectStore, MAX_BROADCAST_ROUNDS};
pub use subscriber::{MutationSubscriber, SubscriberRef};
