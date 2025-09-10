use super::WrapStyle;
use isolang::Language;
use log::{debug, warn};
use nom::{
    branch::alt,
    bytes::complete::{tag, take_until},
    character::complete::{char, not_line_ending, space0},
    combinator::{map, map_res},
    sequence::{preceded, separated_pair},
    IResult, Parser,
};
use std::{
    collections::HashMap,
    fmt::{self, Display},
    str::FromStr,
};

#[derive(Debug, PartialEq, Eq, Clone)]
pub enum ScriptType {
    SSAv4,
    ASS,
    ASS2,
    Unknown(String),
}
impl FromStr for ScriptType {
    type Err = String;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        match s {
            "v4.00" => {
                eprintln!("Now is only implementation ASS");
                Ok(ScriptType::SSAv4)
            }
            "v4.00+" => Ok(ScriptType::ASS),
            "v4.00++" => {
                eprintln!("Now is only implementation ASS");
                Ok(ScriptType::ASS2)
            }
            _ => Err(format!("Invalid value for ScriptType: {}", s)),
        }
    }
}
impl Display for ScriptType {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let s = match &self {
            ScriptType::SSAv4 => "v4.00",
            ScriptType::ASS => "v4.00+",
            ScriptType::ASS2 => "v4.00++",
            ScriptType::Unknown(s) => s,
        };
        write!(f, "{}", s)
    }
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
impl FromStr for YCbCrMatrix {
    type Err = String;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        match s.to_lowercase().as_str() {
            "none" => Ok(Self::None),
            "tv.601" => Ok(Self::BT601TV),
            "pc.601" => Ok(Self::BT601PC),
            "tv.709" => Ok(Self::BT709TV),
            "pc.709" => Ok(Self::BT709PC),
            "tv.240m" => Ok(Self::SMPTE240MTV),
            "pc.240m" => Ok(Self::SMPTE240MPC),
            "tv.fcc" => Ok(Self::FCCTV),
            "pc.fcc" => Ok(Self::FCCPC),
            _ => Ok(Self::Unknown),
        }
    }
}
impl Display for YCbCrMatrix {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let s = match self {
            Self::None => "None",
            Self::BT601TV => "TV.601",
            Self::BT601PC => "PC.601",
            Self::BT709TV => "TV.709",
            Self::BT709PC => "PC.709",
            Self::SMPTE240MTV => "TV.240m",
            Self::SMPTE240MPC => "PC.240m",
            Self::FCCTV => "TV.fcc",
            Self::FCCPC => "PC.fcc",
            // https://github.com/libass/libass/blob/4d6e9ef3077fb279cab051cc00b216c5f08efb34/fuzz/writeout.c#L35-L52
            Self::Unknown => "InvalidUnknownValue",
        };
        write!(f, "{}", s)
    }
}

#[derive(Debug, Clone, PartialEq)]
pub enum ScriptInfoEntry {
    ScriptType(ScriptType),
    ScaledBorderAndShadow(bool),
    YCbCrMatrix(YCbCrMatrix),
    LayoutResX(u32),
    LayoutResY(u32),
    WrapStyle(WrapStyle),
    Timer(f64),
    Kerning(bool),
    Title(String),
    Language(Language),
    OriginalScript(String),
    OriginalTranslation(String),
    OriginalEditing(String),
    OriginalTiming(String),
    ScriptUpdatedBy(String),
    UpdateDetails(String),
    // 用于处理所有 !: 开头的自定义字段
    Custom(String, String),
}
#[derive(Debug, Default)]
pub struct ScriptInfoBuilder {
    script_type: Option<ScriptType>,
    scaled_border_and_shadow: Option<bool>,
    ycbcr_matrix: Option<YCbCrMatrix>,
    layout_res_x: Option<u32>,
    layout_res_y: Option<u32>,
    wrap_style: Option<WrapStyle>,
    timer: Option<f64>,
    kerning: Option<bool>,
    title: Option<String>,
    language: Option<Language>,
    original_script: Option<String>,
    original_translation: Option<String>,
    original_editing: Option<String>,
    original_timing: Option<String>,
    script_updated_by: Option<String>,
    update_details: Option<String>,
    // 用于收集自定义字段
    other_fields: HashMap<String, String>,
}

impl ScriptInfoBuilder {
    pub fn new() -> Self {
        Self::default()
    }
    pub fn apply_entry(&mut self, entry: ScriptInfoEntry) {
        match entry {
            ScriptInfoEntry::ScriptType(v) => self.script_type = Some(v),
            ScriptInfoEntry::ScaledBorderAndShadow(v) => self.scaled_border_and_shadow = Some(v),
            ScriptInfoEntry::YCbCrMatrix(v) => self.ycbcr_matrix = Some(v),
            ScriptInfoEntry::LayoutResX(v) => self.layout_res_x = Some(v),
            ScriptInfoEntry::LayoutResY(v) => self.layout_res_y = Some(v),
            ScriptInfoEntry::WrapStyle(v) => self.wrap_style = Some(v),
            ScriptInfoEntry::Timer(v) => self.timer = Some(v),
            ScriptInfoEntry::Kerning(v) => self.kerning = Some(v),
            ScriptInfoEntry::Title(v) => self.title = Some(v),
            ScriptInfoEntry::Language(v) => self.language = Some(v),
            ScriptInfoEntry::OriginalScript(v) => self.original_script = Some(v),
            ScriptInfoEntry::OriginalTranslation(v) => self.original_translation = Some(v),
            ScriptInfoEntry::OriginalEditing(v) => self.original_editing = Some(v),
            ScriptInfoEntry::OriginalTiming(v) => self.original_timing = Some(v),
            ScriptInfoEntry::ScriptUpdatedBy(v) => self.script_updated_by = Some(v),
            ScriptInfoEntry::UpdateDetails(v) => self.update_details = Some(v),
            // 将自定义字段存入 HashMap
            ScriptInfoEntry::Custom(key, value) => {
                self.other_fields.insert(key, value);
            }
        }
    }

    pub fn build(self) -> ScriptInfo {
        let script_type = match self.script_type {
            Some(st) => st,
            None => {
                warn!("未知的 ASS 版本, 使用默认值(ASS)");
                ScriptType::ASS
            }
        };

        let scaled_border_and_shadow = match self.scaled_border_and_shadow {
            Some(sbas) => sbas,
            None => {
                debug!("未知的 ScaledBorderAndShadow 属性值，默认开启");
                true
            }
        };

        let ycbcr_matrix = match self.ycbcr_matrix {
            Some(ym) => ym,
            None => {
                debug!("未知的 YCbCrMatrix 属性值，默认不使用转换");
                YCbCrMatrix::None
            }
        };

        let layout_res_x = match self.layout_res_x {
            Some(lrx) => lrx,
            None => {
                warn!("未知的 LayoutResX 属性值，默认 1920");
                1920
            }
        };
        let layout_res_y = match self.layout_res_y {
            Some(lry) => lry,
            None => {
                warn!("未知的 LayoutResy 属性值，默认 1080");
                1080
            }
        };

        let wrap_style = match self.wrap_style {
            Some(ws) => ws,
            None => {
                debug!("未知的 WrapStyle 属性值，默认使用智能换行");
                WrapStyle::Smart
            }
        };
        ScriptInfo {
            script_type,
            scaled_border_and_shadow,
            ycbcr_matrix,
            layout_res_x,
            layout_res_y,
            wrap_style,
            timer: self.timer,
            kerning: self.kerning,
            language: self.language,
            title: self.title,
            original_script: self.original_script,
            original_translation: self.original_translation,
            original_editing: self.original_editing,
            original_timing: self.original_timing,
            script_updated_by: self.script_updated_by,
            update_details: self.update_details,
            other_fields: self.other_fields,
        }
    }
}

#[derive(Debug, Clone, PartialEq)]
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
    // 时间缩放因子 >100 会导致字幕时间轴拉长，字幕出现变慢。<100 相反 (使用百分比的)
    pub timer: Option<f64>,
    // 是否启用字距调整
    pub kerning: Option<bool>,
    // 字幕主要语言
    pub language: Option<Language>,

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
const VERSION: &str = env!("CARGO_PKG_VERSION");
const PKG_NAME: &str = env!("CARGO_PKG_NAME");

// NOTE: 在 ASS 规范当中是必须要有一个空格的
impl Display for ScriptInfo {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        writeln!(f, "; Script generated by {} version {}", PKG_NAME, VERSION)?;
        writeln!(f, "ScriptType: {}", self.script_type)?;
        match self.scaled_border_and_shadow {
            true => {
                writeln!(f, "ScaledBorderAndShadow: yes")?;
            }
            false => {
                writeln!(f, "ScaledBorderAndShadow: no")?;
            }
        };
        writeln!(f, "YCbCr Matrix: {}", self.ycbcr_matrix)?;
        writeln!(f, "LayoutResX: {}", self.layout_res_x)?;
        writeln!(f, "PlayResX: {}", self.layout_res_x)?;
        writeln!(f, "LayoutResY: {}", self.layout_res_y)?;
        writeln!(f, "PlayResY: {}", self.layout_res_y)?;
        writeln!(f, "WrapStyle: {}", self.wrap_style)?;
        if let Some(timer) = &self.timer {
            writeln!(f, "Timer: {:.3}", timer)?;
        }
        if let Some(kerning) = &self.kerning {
            writeln!(f, "Kerning: {}", if *kerning { "yes" } else { "no" })?;
        }

        if let Some(language) = &self.language {
            writeln!(f, "Language: {}", language.to_639_1().unwrap())?;
        }

        if let Some(title) = &self.title {
            writeln!(f, "Title: {}", title)?;
        }
        if let Some(original_script) = &self.original_script {
            writeln!(f, "Original Script: {}", original_script)?;
        }
        if let Some(original_translation) = &self.original_translation {
            writeln!(f, "Original Translation: {}", original_translation)?;
        }
        if let Some(original_editing) = &self.original_editing {
            writeln!(f, "Original Editing: {}", original_editing)?;
        }
        if let Some(original_timing) = &self.original_timing {
            writeln!(f, "Original Timing: {}", original_timing)?;
        }
        if let Some(script_updated_by) = &self.script_updated_by {
            writeln!(f, "Script Updated By: {}", script_updated_by)?;
        }
        if let Some(update_details) = &self.update_details {
            writeln!(f, "Update Details: {}", update_details)?;
        }

        for (key, value) in &self.other_fields {
            writeln!(f, "!: {}: {}", key, value)?;
        }

        Ok(())
    }
}

impl Default for ScriptInfo {
    fn default() -> Self {
        Self {
            // 必填部分，使用指定的默认值
            script_type: ScriptType::ASS,
            scaled_border_and_shadow: true,
            ycbcr_matrix: YCbCrMatrix::None,
            layout_res_x: 1920,
            layout_res_y: 1080,
            wrap_style: WrapStyle::Smart,
            timer: Some(100.0),
            kerning: Some(false),
            language: None,

            // 选填部分
            title: None,
            original_script: None,
            original_translation: None,
            original_editing: None,
            original_timing: None,
            script_updated_by: None,
            update_details: None,

            // 初始化其他字段
            other_fields: HashMap::new(),
        }
    }
}

impl ScriptInfo {
    pub fn new(
        script_type: ScriptType,
        scaled_border_and_shadow: bool,
        ycbcr_matrix: YCbCrMatrix,
        layout_res_x: u32,
        layout_res_y: u32,
        wrap_style: WrapStyle,
        timer: Option<f64>,
        kerning: Option<bool>,
        title: Option<String>,
        language: Option<Language>,
        original_script: Option<String>,
        original_translation: Option<String>,
        original_editing: Option<String>,
        original_timing: Option<String>,
        script_updated_by: Option<String>,
        update_details: Option<String>,
        other_fields: HashMap<String, String>,
    ) -> Self {
        Self {
            script_type,
            scaled_border_and_shadow,
            ycbcr_matrix,
            layout_res_x,
            layout_res_y,
            wrap_style,
            timer,
            kerning,
            language,
            title,
            original_script,
            original_translation,
            original_editing,
            original_timing,
            script_updated_by,
            update_details,
            other_fields,
        }
    }
    pub fn add_or_update_other_fields(&mut self, key: String, value: String) -> Result<(), String> {
        self.other_fields
            .entry(key)
            .and_modify(|e| *e = value.clone())
            .or_insert(value);
        Ok(())
    }
}

pub fn parse_script_info_line(input: &str) -> IResult<&str, ScriptInfoEntry> {
    alt((
        // 匹配 "!:" 开头，然后分离 键:值
        map(
            preceded(
                tag("!:"),
                separated_pair(
                    take_until(":"),             // Key 是从 "!:" 后到第一个 ":" 的所有内容
                    preceded(char(':'), space0), // 分隔符是 ":" 后面跟0或多个空格
                    not_line_ending,             // Value 是行内剩余的所有内容
                ),
            ),
            |(k, v): (&str, &str)| {
                ScriptInfoEntry::Custom(k.trim().to_string(), v.trim().to_string())
            },
        ),
        map(
            separated_pair(
                tag("PlayResX"),
                preceded(char(':'), space0),
                map_res(not_line_ending, |s: &str| s.trim().parse::<u32>()),
            ),
            |(_, v)| ScriptInfoEntry::LayoutResX(v),
        ),
        map(
            separated_pair(
                tag("PlayResY"),
                preceded(char(':'), space0),
                map_res(not_line_ending, |s: &str| s.trim().parse::<u32>()),
            ),
            |(_, v)| ScriptInfoEntry::LayoutResY(v),
        ),
        map(
            separated_pair(
                tag("LayoutResX"),
                preceded(char(':'), space0),
                map_res(not_line_ending, |s: &str| s.trim().parse::<u32>()),
            ),
            |(_, v)| ScriptInfoEntry::LayoutResX(v),
        ),
        map(
            separated_pair(
                tag("LayoutResY"),
                preceded(char(':'), space0),
                map_res(not_line_ending, |s: &str| s.trim().parse::<u32>()),
            ),
            |(_, v)| ScriptInfoEntry::LayoutResY(v),
        ),
        map(
            separated_pair(
                tag("Timer"),
                preceded(char(':'), space0),
                map_res(not_line_ending, |s: &str| s.trim().parse::<f64>()),
            ),
            |(_, v)| ScriptInfoEntry::Timer(v),
        ),
        map(
            separated_pair(
                tag("WrapStyle"),
                preceded(char::<&str, _>(':'), space0),
                map_res(not_line_ending, |s: &str| {
                    let trimmed = s.trim();
                    match WrapStyle::from_str(trimmed) {
                        Ok(ws) => Ok(ws),
                        Err(_) => {
                            warn!("未知的 WrapStyle: {}，回退到默认 Smart", trimmed);
                            Err("invalid WrapStyle".to_string())
                        }
                    }
                }),
            ),
            |(_, v)| ScriptInfoEntry::WrapStyle(v),
        ),
        map(
            separated_pair(
                tag("ScaledBorderAndShadow"),
                preceded(char(':'), space0),
                not_line_ending,
            ),
            |(_, v_str): (&str, &str)| {
                let trimmed_val = v_str.trim();

                let result = if trimmed_val.eq_ignore_ascii_case("yes") {
                    true
                } else if let Ok(num) = trimmed_val.parse::<i64>() {
                    num != 0
                } else {
                    false
                };

                ScriptInfoEntry::ScaledBorderAndShadow(result)
            },
        ),
        map(
            separated_pair(tag("Kerning"), preceded(char(':'), space0), not_line_ending),
            |(_, v_str): (&str, &str)| {
                let trimmed_val = v_str.trim();

                let result = if trimmed_val.eq_ignore_ascii_case("yes") {
                    true
                } else if let Ok(num) = trimmed_val.parse::<i64>() {
                    num != 0
                } else {
                    false
                };

                ScriptInfoEntry::Kerning(result)
            },
        ),
        map(
            separated_pair(
                tag("YCbCr Matrix"),
                preceded(char::<&str, _>(':'), space0),
                not_line_ending,
            ),
            |(_, v)| ScriptInfoEntry::YCbCrMatrix(YCbCrMatrix::from_str(v.trim()).unwrap()),
        ),
        map(
            separated_pair(
                tag("Language"),
                preceded(char(':'), space0),
                map_res(not_line_ending, |s: &str| Language::from_str(s)),
            ),
            |(_, v)| ScriptInfoEntry::Language(v),
        ),
        map(
            separated_pair(
                tag("ScriptType"),
                preceded(char::<&str, _>(':'), space0),
                map_res(not_line_ending, |s: &str| {
                    let trimmed = s.trim();
                    match ScriptType::from_str(trimmed) {
                        Ok(st) => Ok(st),
                        Err(_) => {
                            warn!("未知的 ScriptType: {}，回退到默认 ASS", trimmed);
                            Err("invalid ScriptType".to_string())
                        }
                    }
                }),
            ),
            |(_, v)| ScriptInfoEntry::ScriptType(v),
        ),
        map(
            separated_pair(
                tag("Title"),
                preceded(char::<&str, _>(':'), space0),
                not_line_ending,
            ),
            |(_, v)| ScriptInfoEntry::Title(v.trim().to_string()),
        ),
        map(
            separated_pair(
                tag("Original Script"),
                preceded(char::<&str, _>(':'), space0),
                not_line_ending,
            ),
            |(_, v)| ScriptInfoEntry::OriginalScript(v.trim().to_string()),
        ),
        map(
            separated_pair(
                tag("Original Translation"),
                preceded(char::<&str, _>(':'), space0),
                not_line_ending,
            ),
            |(_, v)| ScriptInfoEntry::OriginalTranslation(v.trim().to_string()),
        ),
        map(
            separated_pair(
                tag("Original Editing"),
                preceded(char::<&str, _>(':'), space0),
                not_line_ending,
            ),
            |(_, v)| ScriptInfoEntry::OriginalEditing(v.trim().to_string()),
        ),
        map(
            separated_pair(
                tag("Original Timing"),
                preceded(char::<&str, _>(':'), space0),
                not_line_ending,
            ),
            |(_, v)| ScriptInfoEntry::OriginalTiming(v.trim().to_string()),
        ),
        map(
            separated_pair(
                tag("Script Updated By"),
                preceded(char::<&str, _>(':'), space0),
                not_line_ending,
            ),
            |(_, v)| ScriptInfoEntry::ScriptUpdatedBy(v.trim().to_string()),
        ),
        map(
            separated_pair(
                tag("Update Details"),
                preceded(char::<&str, _>(':'), space0),
                not_line_ending,
            ),
            |(_, v)| ScriptInfoEntry::UpdateDetails(v.trim().to_string()),
        ),
    ))
    .parse(input.trim()) // 在解析前对整行进行 trim，可以忽略行首尾的空白
}
