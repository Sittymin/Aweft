//! 去中心协作：把字幕行的起止时间经 `p2panda` 明文主题流同步给队友。
//!
//! 分层与设计定案见 `docs/architecture/collab-design.md`。模块划分即数据流向：
//! - `cap`：身份、路径与凭证（授权层，纯计算，无状态）。
//! - `roster`：每个成员现行凭证的登记表，收紧与撤销在这里生效。
//! - `wire`：线上一条消息长什么样。
//! - `engine`：后台线程持有 `p2panda::Node` 与运行时，本地变更进、远端变更出。
//! - `subscriber`：与 `ProjectStore` 相接的两个适配器（出去的回执、进来的写入）。
//! - `config`：密钥、会话记录与邀请文本的落盘。
//!
//! 真相仍然只在 `ProjectStore`：协作既不持有工程数据，也不在异步回调里碰它。

pub mod cap;
pub mod config;
pub mod engine;
pub mod roster;
pub mod subscriber;
pub mod wire;

pub use cap::{authorize, Member, Namespace};
pub use config::ProjectRecord;
pub use engine::{Bootstrap, CollabEngine, CollabEvent, RemoteEdit, Session};
pub use roster::Roster;
pub use wire::PendingEdit;
