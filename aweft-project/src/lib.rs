//! 统一工程文档的唯一所有者。
//!
//! - `document`：工程文档结构（`Project` + 布局表 + 媒体条目表）与文件格式版本。
//! - `receipt`：写入回执（来源标记 + 变更明细）。
//! - `store`：文档唯一所有权存储；已建模的写入经单一入口并留下一条回执，
//!   持有者在一轮交互结束时用 `ProjectStore::take_pending` 取走分发。
//!
//! 依赖方向：本成员依赖 `aweft-model`；图形界面、命令行、插件、协作依赖本成员。

pub mod document;
pub mod receipt;
pub mod store;

pub use document::{AweftDocument, LayoutArea, LayoutTable, MediaEntry, AWEFT_DOCUMENT_FORMAT};
pub use receipt::{EventTimeChanged, MutationKind, MutationReceipt, Origin};
pub use store::ProjectStore;
