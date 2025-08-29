use super::WrapStyle;
use std::collections::HashMap;

#[derive(Debug, PartialEq, Eq, Clone)]
pub enum ScriptType {
    SSAv4,
    ASS,
    ASS2,
    Unknown(String),
}

// 用于指明当前色彩在最终播放时如何映射到视频的色彩格式
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum YCbCrMatrix {
    Unknown,     // 文件中字段是错误的（实际转换由播放器决定）
    None,        // 不进行色彩转换
    BT601TV,     // TV.601
    BT601PC,     // PC.601
    BT709TV,     // TV.709
    BT709PC,     // PC.709
    SMPTE240MTV, // TV.240m
    SMPTE240MPC, // PC.240m
    FCCTV,       // TV.fcc
    FCCPC,       // PC.fcc
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ScriptInfo {
    // 这一部分的 ; 行不需要读取，是注释
    // 渲染相关
    // SSA 版本号
    pub script_type: ScriptType,
    // 缩放边框与阴影 设置为 no 没有好处, 始终为 yes
    pub scaled_border_and_shadow: bool,
    // 色彩空间
    pub ycbcr_matrix: YCbCrMatrix,
    // 字幕作者视频宽度
    pub layout_res_x: u32,
    // 字幕作者视频高度
    pub layout_res_y: u32,
    // WARN: 需要写入文件
    // NOTE: 需要和 layout 一致, 仅用于兼容
    // pub play_res_x: u32,
    // pub play_res_y: u32,
    // 换行样式
    pub wrap_style: WrapStyle,

    // 渲染无关
    // 描述
    pub title: Option<String>,
    // 原始作者
    pub original_script: Option<String>,
    // 原始翻译
    pub original_translation: Option<String>,
    // 原始校对
    pub original_editing: Option<String>,
    // 原始时间轴
    pub original_timing: Option<String>,
    // 字幕更新者
    pub script_updated_by: Option<String>,
    // 更新描述
    pub update_details: Option<String>,
    // 自定义字段 !开头字段  例如 !: Release Date: 2009-08-07
    pub other_fields: HashMap<String, String>,
}
