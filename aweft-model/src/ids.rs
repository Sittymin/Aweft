use serde::{Deserialize, Serialize};
use std::fmt;
use uuid::Uuid;

// 宏定义：快速生成各种 ID 类型
macro_rules! define_id {
    ($name:ident) => {
        #[derive(
            Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize,
        )]
        pub struct $name(pub Uuid);

        impl $name {
            pub fn new() -> Self {
                Self(Uuid::new_v4())
            }
        }

        impl Default for $name {
            fn default() -> Self {
                Self::new()
            }
        }

        impl fmt::Display for $name {
            fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
                write!(f, "{}", self.0)
            }
        }
    };
}

// 定义项目里用到的各种 ID
define_id!(ItemId); // 时间轴上的项（分组或事件行）
define_id!(ActorId); // 角色
define_id!(ActionId);          // 可复用动画曲线集合（比 AnimData 更底层）
define_id!(AnimDataId); // NLA 动画数据配置 ID
define_id!(NodeGraphId); // 节点图模板资源 ID
define_id!(MediaId); // 媒体文件引用 ID
define_id!(NodeId); // 节点图内的节点
define_id!(SocketId); // 节点图内的端口
define_id!(AnchorId); // 行内锚点
