//! 协作网络上跑的应用消息。
//!
//! 两种消息共用一条流：改数据（[`CollabMessage::Edit`]）与换权限
//! （[`CollabMessage::Grant`]）。权限通告走同一条日志同步，是因为它必须像数据一样
//! 到达每个成员——名册（`roster`）只有在收到收紧通告的那一刻才生效，走旁路就得自己
//! 再实现一遍可靠投递。
//!
//! 消息本身不签名：`p2panda` 会用节点的签名密钥签每条操作，而单一种子方案下
//! 那把密钥的公钥就是 `willow` 的成员标识，所以签名者身份经
//! `ProcessedOperation::author` 拿到即可，见 `cap::authorize`。

use serde::{Deserialize, Serialize};

/// 一条协作消息。凭证随每条消息携带，接收端独立验算，不依赖发件人已获批准。
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum CollabMessage {
    /// 一行起止时间的变更。
    Edit {
        /// 轨道组标识：16 字节 `uuid`。凭证按轨道转授，验算要用它重建路径。
        track: Vec<u8>,
        /// 行标识：16 字节 `uuid`。接收端按它定位自己工程里的同一行。
        row: Vec<u8>,
        /// 变更后的行起止时间（毫秒）。成对发送：接收端需要一对合法值才能写回。
        start_ms: i64,
        end_ms: i64,
        /// 发送者持有的写凭证，`cap::encode_capability` 的产物。
        capability: Vec<u8>,
    },
    /// 通告一张新凭证：既用于「我给某人签了这个范围」，也用于「我的凭证换成这张了」。
    /// 谁能宣布、宣布完名册怎么变，全在 `roster::Roster::apply`。
    Grant { capability: Vec<u8> },
    /// 完整工程快照：初次加入时所有者把整份 `AweftDocument` 发给空工程的加入者，
    /// 后者直接替换本地真相（丢失本地进度，见 bridge 注释）。
    Snapshot {
        data: Vec<u8>,
        capability: Vec<u8>,
    },
}

/// 一次待发送的本地变更：还没有附上凭证。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PendingEdit {
    pub track: Vec<u8>,
    pub row: Vec<u8>,
    pub start_ms: i64,
    pub end_ms: i64,
}
