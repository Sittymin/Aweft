//! 错误类型定义
//!
//! 该模块定义了 aweft-ass 库中所有的错误类型。

use thiserror::Error;

/// ASS 解析相关的所有错误
#[derive(Error, Debug)]
pub enum ParseError {
    /// 时间解析错误
    #[error("时间解析错误: {0}")]
    Time(#[from] TimeParseError),

    /// 效果解析错误
    #[error("效果解析错误: {0}")]
    Effect(#[from] EffectParseError),

    /// 边框样式解析错误
    #[error("边框样式解析错误: {0}")]
    BorderStyle(#[from] BorderStyleParseError),

    /// 粗体类型解析错误
    #[error("粗体类型解析错误: {0}")]
    BoldType(#[from] BoldTypeParseError),

    /// 换行样式解析错误
    #[error("换行样式解析错误: {0}")]
    WrapStyle(#[from] WrapStyleParseError),

    /// 脚本类型解析错误
    #[error("脚本类型解析错误: {0}")]
    ScriptType(#[from] ScriptTypeParseError),

    /// YCbCr 矩阵解析错误
    #[error("YCbCr 矩阵解析错误: {0}")]
    YCbCrMatrix(#[from] YCbCrMatrixParseError),

    /// 样式字段解析错误
    #[error("样式字段解析错误: {0}")]
    StyleField(#[from] StyleFieldParseError),

    /// 事件字段解析错误
    #[error("事件字段解析错误: {0}")]
    EventField(#[from] EventFieldParseError),

    /// libass 对齐方式解析错误
    #[error("对齐方式解析错误: {0}")]
    LibassAlignment(#[from] aweft_libass::AlignmentParseError),

    /// 文件 I/O 错误
    #[error("文件操作失败: {0}")]
    Io(#[from] std::io::Error),

    /// 格式错误
    #[error("格式错误: {message}")]
    Format { message: String },

    /// 字段缺失
    #[error("必需字段缺失: {field}")]
    MissingField { field: String },

    /// 字段值无效
    #[error("字段 '{field}' 的值无效: {value}, 原因: {reason}")]
    InvalidFieldValue {
        field: String,
        value: String,
        reason: String,
    },

    /// 数值解析错误
    #[error("数值解析失败: {message}")]
    ParseNumber { message: String },

    /// Section 解析错误
    #[error("Section 解析失败: {message}")]
    Section { message: String },

    /// 对话行解析错误
    #[error("对话行解析失败: {message}")]
    DialogueLine { message: String },

    /// 通用解析错误
    #[error("解析失败: {message}")]
    Generic { message: String },
}

/// 库专用的 Result 类型
pub type Result<T> = std::result::Result<T, ParseError>;

/// 时间解析错误
#[derive(Error, Debug, Clone, PartialEq, Eq)]
pub enum TimeParseError {
    #[error("时间格式无效: 期望 'H:MM:SS.CC' 格式，实际输入 '{input}'")]
    InvalidFormat { input: String },

    #[error("小时解析失败: '{value}' 不是有效的整数")]
    InvalidHours { value: String },

    #[error("分钟解析失败: '{value}' 不是有效的整数或超出范围 [0-59]")]
    InvalidMinutes { value: String },

    #[error("秒数解析失败: '{value}' 不是有效的整数或超出范围 [0-59]")]
    InvalidSeconds { value: String },

    #[error("厘秒解析失败: '{value}' 不是有效的整数或超出范围 [0-99]")]
    InvalidCentiseconds { value: String },

    #[error("时间值溢出: 无法转换为 TimeDelta")]
    Overflow,
}

/// 效果解析错误
#[derive(Error, Debug, Clone, PartialEq, Eq)]
pub enum EffectParseError {
    #[error("未知的效果类型: '{effect_type}'")]
    UnknownEffect { effect_type: String },

    #[error("Scroll 效果参数无效: 期望 3 个参数 (y1, y2, delay)，但解析失败")]
    InvalidScrollParameters,

    #[error("Banner 效果参数无效: 期望 1-2 个参数 (delay, [direction])，但解析失败")]
    InvalidBannerParameters,

    #[error("效果格式无效: '{input}'")]
    InvalidFormat { input: String },

    #[error("参数解析失败: {message}")]
    ParameterError { message: String },
}

/// 边框样式解析错误
#[derive(Error, Debug, Clone, PartialEq, Eq)]
pub enum BorderStyleParseError {
    #[error("无效的边框样式值: '{value}'，仅支持 '1' (OutlineAndDropShadow) 或 '3' (OpaqueBox)")]
    InvalidValue { value: String },
}

/// 粗体类型解析错误
#[derive(Error, Debug, Clone, PartialEq, Eq)]
pub enum BoldTypeParseError {
    #[error("无效的粗体值: '{input}' 不是有效的整数")]
    ParseIntError { input: String },

    #[error("无效的粗体权重: {value}，必须是 -1 (Bold), 0 (NonBold) 或正整数 (Weight)")]
    InvalidWeight { value: i32 },
}

/// 换行样式解析错误
#[derive(Error, Debug, Clone, PartialEq, Eq)]
pub enum WrapStyleParseError {
    #[error("无效的换行样式值: '{value}'，仅支持 '0' (Smart), '1' (EndOfWord), '2' (NoWrap), '3' (Smart)")]
    InvalidValue { value: String },
}

/// 脚本类型解析错误
#[derive(Error, Debug, Clone, PartialEq, Eq)]
pub enum ScriptTypeParseError {
    #[error("无效的脚本类型: '{value}'，仅支持 'v4.00' (SSAv4), 'v4.00+' (ASS), 'v4.00++' (ASS2)")]
    InvalidValue { value: String },
}

/// YCbCr 矩阵解析错误（此类型实际上总是成功，返回 Unknown 作为后备）
#[derive(Error, Debug, Clone, PartialEq, Eq)]
pub enum YCbCrMatrixParseError {
    #[error("无效的 YCbCr 矩阵值: '{value}'")]
    InvalidValue { value: String },
}

/// 样式字段解析错误
#[derive(Error, Debug, Clone, PartialEq, Eq)]
pub enum StyleFieldParseError {
    #[error("未知或不支持的样式格式字段: '{field}'")]
    UnknownField { field: String },
}

/// 事件字段解析错误
#[derive(Error, Debug, Clone, PartialEq, Eq)]
pub enum EventFieldParseError {
    #[error("未知的 Event 格式字段: '{field}'")]
    UnknownField { field: String },
}
