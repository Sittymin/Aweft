use palette::Srgba;
use serde::{Deserialize, Serialize};

use crate::ids::{AnimDataId, SocketId};

use super::math::{ClipRect, Vec2, Vec3};
use super::style::StyleResult;

// ── Socket 类型系统 ────────────────────────────────────────────────────────────

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum SocketType {
    Style,
    Float,
    Bool,
    Color,
    Text,
    Int,
    Vec2,
    Vec3,
    ClipRect,
    Animation,
    /// 应用单元：应用匹配 / 应用动画的输出，组输出的输入。
    /// 承载一组"待应用"内容（静态样式 + 是否命中 + 可选动画），
    /// 由组输出按连接端口顺序收集合并。只有组输出的输入能接这种类型。
    Application,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum SocketValue {
    Float(f64),
    Bool(bool),
    Color(Srgba),
    Text(String),
    Int(i64),
    Vec2 { x: f64, y: f64 },
    Vec3 { x: f64, y: f64, z: f64 },
    ClipRect(ClipRect),
    Animation(AnimDataId),
    Application(ApplicationUnit),
}

impl SocketValue {
    pub fn data_type(&self) -> SocketType {
        match self {
            SocketValue::Float(_) => SocketType::Float,
            SocketValue::Bool(_) => SocketType::Bool,
            SocketValue::Color(_) => SocketType::Color,
            SocketValue::Text(_) => SocketType::Text,
            SocketValue::Int(_) => SocketType::Int,
            SocketValue::Vec2 { .. } => SocketType::Vec2,
            SocketValue::Vec3 { .. } => SocketType::Vec3,
            SocketValue::ClipRect(_) => SocketType::ClipRect,
            SocketValue::Animation(_) => SocketType::Animation,
            SocketValue::Application(_) => SocketType::Application,
        }
    }
}

/// 应用图的一个"待应用单元"：由 应用匹配 / 应用动画 节点产出，组输出按连接端口
/// 顺序收集后合并应用。`condition` 为真才生效。
/// `style` 为**明确连接的静态样式**：应用匹配 / 应用动画都必须连接样式来源
/// （样式链引用；想要默认样式就连接默认样式链），不提供"可选贡献"——缺失即报错。
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct ApplicationUnit {
    pub style: StyleResult,
    pub condition: bool,
    pub animation: Option<AnimDataId>,
}

// ── 类型化 Socket 引用 ────────────────────────────────────────────────────────

/// 类型化的 socket 引用。`T` 为 socket 承载的数据类型，只在编译期存在。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct SocketRef<T> {
    pub id: SocketId,
    _marker: std::marker::PhantomData<T>,
}

impl<T> SocketRef<T> {
    pub fn new(id: SocketId) -> Self {
        Self {
            id,
            _marker: std::marker::PhantomData,
        }
    }
}

/// 可在 UI/代码中手动输入默认值的 socket 类型。
///
/// 密封 trait，由 crate 内部实现 `to_socket_value`，外部无需关心 `SocketValue` 包装。
pub trait InputSocket: private::Sealed + SocketTypeInfo {
    type Value;
    fn to_socket_value(value: Self::Value) -> SocketValue;
}

/// 让类型级 marker 携带运行时 `SocketType`，供宏生成 `SocketDecl` 时使用。
pub trait SocketTypeInfo {
    const SOCKET_TYPE: SocketType;
}

mod private {
    pub trait Sealed {}
}

macro_rules! define_socket_types {
    ($($marker:ident => $value:ty, ($param:ident) => $body:expr => $stype:ident);* $(;)?) => {
        $(
            #[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
            pub struct $marker;
            impl private::Sealed for $marker {}
            impl SocketTypeInfo for $marker {
                const SOCKET_TYPE: SocketType = SocketType::$stype;
            }
            impl InputSocket for $marker {
                type Value = $value;
                fn to_socket_value($param: $value) -> SocketValue {
                    $body
                }
            }
        )*
    };
}

define_socket_types! {
    FloatSocket => f64, (v) => SocketValue::Float(v) => Float;
    BoolSocket => bool, (v) => SocketValue::Bool(v) => Bool;
    IntSocket => i64, (v) => SocketValue::Int(v) => Int;
    TextSocket => String, (v) => SocketValue::Text(v) => Text;
    ColorSocket => Srgba, (v) => SocketValue::Color(v) => Color;
    Vec2Socket => Vec2, (v) => SocketValue::Vec2 { x: v.x, y: v.y } => Vec2;
    Vec3Socket => Vec3, (v) => SocketValue::Vec3 { x: v.x, y: v.y, z: v.z } => Vec3;
    ClipRectSocket => ClipRect, (v) => SocketValue::ClipRect(v) => ClipRect;
}

/// Style 是一种计算结果，不能直接作为默认值输入。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct StyleSocket;
impl private::Sealed for StyleSocket {}
impl SocketTypeInfo for StyleSocket {
    const SOCKET_TYPE: SocketType = SocketType::Style;
}

/// Animation 引用端口，携带 `AnimDataId`。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct AnimationSocket;
impl private::Sealed for AnimationSocket {}
impl SocketTypeInfo for AnimationSocket {
    const SOCKET_TYPE: SocketType = SocketType::Animation;
}

/// 应用单元端口（应用匹配 / 应用动画的输出，组输出的输入）。
/// 计算结果类型，不能作为默认值输入（同 Style）。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct ApplicationSocket;
impl private::Sealed for ApplicationSocket {}
impl SocketTypeInfo for ApplicationSocket {
    const SOCKET_TYPE: SocketType = SocketType::Application;
}

// ── Socket 声明 & 实例 ────────────────────────────────────────────────────────

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SocketDecl {
    pub name: String,
    pub data_type: SocketType,
    pub default_value: Option<SocketValue>,
}

impl SocketDecl {
    pub fn input(name: impl Into<String>, data_type: SocketType) -> Self {
        Self {
            name: name.into(),
            data_type,
            default_value: None,
        }
    }

    pub fn input_with_default(
        name: impl Into<String>,
        data_type: SocketType,
        default: SocketValue,
    ) -> Self {
        Self {
            name: name.into(),
            data_type,
            default_value: Some(default),
        }
    }

    pub fn output(name: impl Into<String>, data_type: SocketType) -> Self {
        Self {
            name: name.into(),
            data_type,
            default_value: None,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Socket {
    pub id: SocketId,
    pub node: crate::ids::NodeId,
    pub data_type: SocketType,
    pub name: String,
    pub default_value: Option<SocketValue>,
}

// ── Link ───────────────────────────────────────────────────────────────────────

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Link {
    pub from: SocketId,
    pub to: SocketId,
}
