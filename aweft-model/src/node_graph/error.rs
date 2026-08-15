use thiserror::Error;

use crate::ids::{NodeGraphId, NodeId, SocketId};

use super::socket::SocketType;

#[derive(Debug, Error)]
pub enum EvalError {
    #[error("图 {0} 存在环路，无法求值")]
    CyclicGraph(NodeGraphId),

    #[error("端口类型不匹配：期望 {expected:?}，实际 {got:?}")]
    TypeMismatch {
        expected: SocketType,
        got: SocketType,
    },

    #[error("找不到节点 {0}")]
    MissingNode(NodeId),

    #[error("找不到端口 {0}")]
    MissingSocket(SocketId),

    #[error("找不到节点图 {0}")]
    MissingGraph(NodeGraphId),

    #[error("图中已存在 Viewer，不能添加第二个")]
    DuplicateViewer,

    #[error("完全静态节点不允许追加额外端口")]
    StaticNodeExtras,

    #[error("节点（{0}）的样式输入未连接；请把样式来源连接到样式端口（默认样式可连接默认样式链）")]
    StyleNotConnected(String),
}
