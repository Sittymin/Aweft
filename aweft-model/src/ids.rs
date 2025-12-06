use serde::{Deserialize, Serialize};
use std::fmt;
use uuid::Uuid;

// 宏定义：快速生成各种 ID 类型
macro_rules! define_id {
    ($name:ident) => {
        #[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
        pub struct $name(pub Uuid);

        impl $name {
            pub fn new() -> Self {
                Self(Uuid::new_v4())
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
define_id!(CurveId); // 动画曲线资源 ID
define_id!(NodeGraphId); // 节点图模板资源 ID
define_id!(MediaId); // 媒体文件引用 ID
