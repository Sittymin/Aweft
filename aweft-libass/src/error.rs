use std::ffi::NulError;
use std::str::Utf8Error;
use thiserror::Error;

/// libass FFI 绑定的统一错误类型
#[derive(Error, Debug)]
pub enum LibassError {
    /// libass 库初始化失败
    #[error("libass 库初始化失败: {context}")]
    LibraryInit { context: String },

    /// libass 渲染器初始化失败
    #[error("libass 渲染器初始化失败: {context}")]
    RendererInit { context: String },

    /// libass 轨道初始化失败
    #[error("libass 轨道初始化失败: {context}")]
    TrackInit { context: String },

    /// 内存分配失败
    #[error("libass 内存分配失败: {context}")]
    MemoryAllocation { context: String },

    /// 无效参数
    #[error("无效参数: {context}")]
    InvalidParameter { context: String },

    /// UTF-8 编码错误
    #[error("UTF-8 编码错误: {source}")]
    Utf8Conversion {
        #[from]
        source: Utf8Error,
    },

    /// C 字符串包含 NUL 字符
    #[error("C 字符串包含无效的 NUL 字符: {context}")]
    NulInString { context: String },

    /// IO 错误（文件路径处理等）
    #[error("文件操作失败: {source}")]
    Io {
        #[from]
        source: std::io::Error,
    },

    /// 内部错误（例如锁获取失败）
    #[error("内部错误: {context}")]
    Internal { context: String },

    /// 无效的 ID（样式或事件）
    #[error("无效的 {resource_type} ID: {id}")]
    InvalidId { resource_type: String, id: i32 },

    /// 资源不可用（例如没有可用的样式）
    #[error("资源不可用: {context}")]
    ResourceUnavailable { context: String },

    /// 对齐方式解析错误
    #[error("对齐方式解析错误: {0}")]
    Alignment(#[from] AlignmentParseError),
}

impl From<NulError> for LibassError {
    fn from(err: NulError) -> Self {
        LibassError::NulInString {
            context: err.to_string(),
        }
    }
}

/// 对齐方式解析错误
#[derive(Error, Debug, Clone, PartialEq, Eq)]
pub enum AlignmentParseError {
    #[error("无效的对齐值: '{value}'，必须是 1-9 之间的整数")]
    InvalidValue { value: String },

    #[error("无效的 libass 对齐位标志: {bits}")]
    InvalidLibassBits { bits: i32 },
}

/// libass 的 Result 类型别名
pub type Result<T> = std::result::Result<T, LibassError>;

