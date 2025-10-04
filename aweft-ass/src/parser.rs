use crate::error::{ParseError, Result};
use crate::types::{
    event::{Event, EventParser},
    script::{parse_script_info_line, ScriptInfo, ScriptInfoBuilder},
    style::{StyleParser, StyleWithoutName},
    *,
};
use crossbeam_channel::bounded;
use log::{error, warn};
use rayon::prelude::*;
use std::{
    collections::HashMap,
    fs::File,
    io::{BufRead, BufReader, BufWriter, Write},
    path::{Path, PathBuf},
    sync::Arc,
};

enum SectionContent {
    ScriptInfo(Vec<String>),
    V4PStyles(Vec<String>),
    Events(Vec<String>),
}

// 定义解析完成后的结果块
#[derive(Debug)]
enum ParsedSection {
    ScriptInfo(ScriptInfo),
    Styles(HashMap<String, StyleWithoutName>),
    Events(Vec<Event>),
    // NOTE: 仅有Event的时间与文本字段可能存在不可自动修复的错误
    Error(String),
}

fn process_script_info_chunk(lines: Vec<String>) -> ParsedSection {
    let mut builder = ScriptInfoBuilder::new();
    for line in lines {
        if let Ok((_, entry)) = parse_script_info_line(&line) {
            builder.apply_entry(entry);
        }
    }
    ParsedSection::ScriptInfo(builder.build())
}

fn process_styles_chunk(lines: Vec<String>) -> ParsedSection {
    let mut parser = StyleParser::default();
    if let Some(format_line) = lines.iter().find(|l| l.starts_with("Format:")) {
        let format_data = format_line.strip_prefix("Format:").unwrap().trim();
        if let Err(e) = parser.learn_format(format_data) {
            return ParsedSection::Error(e.to_string());
        }
    }
    let styles = lines
        .iter()
        .filter(|l| !l.starts_with("Format:"))
        .filter_map(|line| parser.parse_style_line(line).ok())
        .collect();
    ParsedSection::Styles(styles)
}

fn parse_events_section(lines: Vec<String>) -> ParsedSection {
    let mut parser = EventParser::default();
    if let Some(format_line) = lines.iter().find(|l| l.starts_with("Format:")) {
        let format_data = format_line.strip_prefix("Format:").unwrap().trim();
        if let Err(e) = parser.learn_format(format_data) {
            return ParsedSection::Error(e.to_string());
        }
    }
    let shared_parser = Arc::new(parser);
    let events = lines
        .par_iter()
        .filter(|line| !line.starts_with("Format:"))
        .map_with(shared_parser, |p, l| p.parse_event_line(l))
        .filter_map(|res| res.ok())
        .collect();
    ParsedSection::Events(events)
}

struct SectionIterator<I>
where
    I: Iterator<Item = std::io::Result<String>>,
{
    lines: std::iter::Peekable<I>,
}

impl<I> SectionIterator<I>
where
    I: Iterator<Item = std::io::Result<String>>,
{
    fn new(lines: I) -> Self {
        Self {
            lines: lines.peekable(),
        }
    }
}

// 为我们的结构体实现 Iterator Trait
impl<I> Iterator for SectionIterator<I>
where
    I: Iterator<Item = std::io::Result<String>>,
{
    type Item = Result<SectionContent>;

    fn next(&mut self) -> Option<Self::Item> {
        loop {
            // 使用循环代替递归
            // 跳过空行和注释
            while let Some(Ok(line)) = self.lines.peek() {
                let trimmed = line.trim();
                if trimmed.is_empty() || trimmed.starts_with(';') {
                    self.lines.next();
                } else {
                    break;
                }
            }

            // 读取 Section Header
            let first_line = match self.lines.next()? {
                Ok(line) => line,
                Err(e) => return Some(Err(e.into())),
            };

            let trimmed_header = first_line.trim();
            let section_name = match trimmed_header
                .strip_prefix('[')
                .and_then(|s| s.strip_suffix(']'))
            {
                Some(name) => name,
                None => continue, // 如果不是合法的 [Header]，继续找下一个
            };

            let section = match section_name {
                "Script Info" => Section::ScriptInfo,
                "V4+ Styles" => Section::V4PStyles,
                "Events" => Section::Events,
                _ => continue, // 跳过不认识的 Section
            };

            // 收集 Section 内容
            let mut chunk_lines = Vec::new();
            while let Some(Ok(line)) = self.lines.peek() {
                if line.trim().starts_with('[') {
                    break;
                }
                match self.lines.next().unwrap() {
                    Ok(line_str) => chunk_lines.push(line_str),
                    Err(e) => return Some(Err(e.into())),
                }
            }

            let work_chunk = match section {
                Section::ScriptInfo => SectionContent::ScriptInfo(chunk_lines),
                Section::V4PStyles => SectionContent::V4PStyles(chunk_lines),
                Section::Events => SectionContent::Events(chunk_lines),
            };

            return Some(Ok(work_chunk)); // 成功找到一个块，返回它
        }
    }
}

impl ASSFile {
    pub fn new() -> Self {
        Self {
            script_info: script::ScriptInfo::default(),
            styles: HashMap::new(),
            events: Vec::new(),
        }
    }
    pub fn parse_from_file(path: impl AsRef<Path>) -> Result<Self> {
        let file = File::open(path)?;
        let reader = BufReader::new(file);

        // 使用我们的自定义迭代器，代码意图非常清晰
        let section_iterator = SectionIterator::new(reader.lines());

        // `work_channel`: 主线程 -> 消费者，发送要处理的工作块
        let (work_sender, work_receiver) = bounded::<Result<SectionContent>>(num_cpus::get() * 2);

        // `result_channel`: 消费者 -> 主线程，发送处理完的结果
        let (result_sender, result_receiver) = bounded::<ParsedSection>(num_cpus::get() * 2);

        // 用于收集解析过程中的错误
        let (error_sender, error_receiver) = bounded::<String>(10);

        // 使用 Rayon 的 `scope` 可以更安全地处理线程和错误
        rayon::scope(|s| {
            // 在 Rayon scope 内启动一个生产者
            let error_sender_clone = error_sender.clone();
            s.spawn(move |_| {
                for section_result in section_iterator {
                    if work_sender.send(section_result).is_err() {
                        let _ = error_sender_clone.send("工作接收器已关闭".to_string());
                        break;
                    }
                }
            });

            // Rayon 的主线程池成为消费者
            work_receiver
                .into_iter()
                .par_bridge()
                .for_each(|chunk_result| {
                    // 首先处理IO错误
                    let chunk = match chunk_result {
                        Ok(c) => c,
                        Err(e) => {
                            error!("读取文件时发生 IO 错误: {:?}", e);
                            let _ = error_sender.send(format!("IO 错误: {}", e));
                            return;
                        }
                    };

                    // 然后分派解析任务
                    let parsed_chunk = match chunk {
                        SectionContent::ScriptInfo(lines) => process_script_info_chunk(lines),
                        SectionContent::V4PStyles(lines) => process_styles_chunk(lines),
                        SectionContent::Events(lines) => parse_events_section(lines),
                    };

                    match parsed_chunk {
                        ParsedSection::Error(e) => {
                            warn!("解析区块时出现错误: {}", e);
                            let _ = error_sender.send(e);
                        }
                        parsed => {
                            if result_sender.send(parsed).is_err() {
                                error!("结果接收器已关闭，无法发送解析结果");
                            }
                        }
                    }
                });
        });

        // 关闭错误发送器，以便后续可以迭代错误
        drop(error_sender);

        // 收集所有错误信息
        let errors: Vec<String> = error_receiver.into_iter().collect();

        let mut script_info = None;
        let mut styles = None;
        let mut events = None;

        for parsed_result in result_receiver {
            match parsed_result {
                ParsedSection::ScriptInfo(si) => script_info = Some(si),
                ParsedSection::Styles(s) => styles = Some(s),
                ParsedSection::Events(e) => events = Some(e),
                ParsedSection::Error(_) => {
                    // 错误已经记录到 errors 向量中
                }
            }
        }

        // 如果有严重错误且缺少必要的 section，返回错误
        if !errors.is_empty() && script_info.is_none() {
            return Err(ParseError::Section {
                message: format!(
                    "解析失败，遇到 {} 个错误。首个错误: {}",
                    errors.len(),
                    errors.first().unwrap_or(&"未知错误".to_string())
                ),
            });
        }

        // 如果有非致命错误，记录警告
        if !errors.is_empty() {
            warn!("解析过程中遇到 {} 个非致命错误", errors.len());
            for (i, err) in errors.iter().enumerate().take(5) {
                warn!("  错误 {}: {}", i + 1, err);
            }
        }

        Ok(ASSFile {
            script_info: script_info.ok_or_else(|| ParseError::Section {
                message: "[Script Info] 区块未找到或解析失败".to_string(),
            })?,

            styles: styles.unwrap_or_default(),
            events: events.unwrap_or_default(),
        })
    }
    pub fn merge_styles(&mut self, other: &ASSFile) {
        for (key, val) in &other.styles {
            if !self.styles.contains_key(key) {
                self.styles.insert(key.clone(), val.clone());
            }
        }
    }
    pub fn write_to_file_buffered(
        &self,
        path: &PathBuf,
        // HACK: 看看这个是否可以自适应
        events_flush_interval: usize,
    ) -> Result<()> {
        let file = File::create(path)?;
        let mut writer = BufWriter::new(file);
        writeln!(writer, "[Script Info]")?;
        writeln!(writer, "{}", self.script_info)?;
        writer.flush()?;

        if self.script_info.script_type == script::ScriptType::ASS {
            writeln!(writer, "[V4+ Styles]")?;
            writeln!(writer, "Format: Name, Fontname, Fontsize, PrimaryColour, SecondaryColour, OutlineColour, BackColour, Bold, Italic, Underline, StrikeOut, ScaleX, ScaleY, Spacing, Angle, BorderStyle, Outline, Shadow, Alignment, MarginL, MarginR, MarginV, Encoding")?;
            for (key, value) in &self.styles {
                writeln!(writer, "Style: {},{}", key, value)?;
            }
        } else {
            return Err(ParseError::Generic {
                message: format!(
                    "不支持的脚本类型: {:?}，当前仅支持 ASS (v4.00+)",
                    self.script_info.script_type
                ),
            }
            .into());
        }

        writeln!(writer, "\n[Events]")?;
        writeln!(
            writer,
            "Format: Layer, Start, End, Style, Name, MarginL, MarginR, MarginV, Effect, Text"
        )?;
        for (index, event) in self.events.iter().enumerate() {
            writeln!(writer, "{}", event)?;
            if (index + 1) % events_flush_interval == 0 {
                writer.flush()?;
            }
        }

        writer.flush()?;
        Ok(())
    }
    fn merge_events_with(mut self, other: &ASSFile) {
        self.events = [&self.events[..], &other.events[..]].concat();
    }
    fn merge_styles_with(mut self, other: &ASSFile) {
        for (key, value) in other.styles.clone() {
            if !self.styles.contains_key(&key) {
                self.styles.insert(key, value);
            }
        }
    }
}

#[cfg(test)]
mod tests {
    mod support {
        use log::warn;
        use proptest::prelude::*;

        use crate::types::event::{parse_dialogue_line, Comment, Event};
        use crate::types::style::{BorderStyle, StyleWithoutName};
        use crate::types::BoldType;
        use aweft_libass::types::{Alignment, AssColorRgba};
        pub const SCRIPTTYPES: &[&str] = &["v4.00", "v4.00+", "v4.00++"];

        pub const YCBCRMATRIXS: &[&str] = &[
            "None", "TV.601", "PC.601", "TV.709", "PC.709", "TV.240m", "PC.240m", "TV.fcc",
            "PC.fcc",
        ];

        pub const STYLEFIELDS: &[&str] = &[
            "Name",
            "Fontname",
            "Fontsize",
            "PrimaryColour",
            "SecondaryColour",
            "OutlineColour",
            "BackColour",
            "Bold",
            "Italic",
            "Underline",
            "StrikeOut",
            "ScaleX",
            "ScaleY",
            "Spacing",
            "Angle",
            "BorderStyle",
            "Outline",
            "Shadow",
            "Alignment",
            "MarginL",
            "MarginR",
            "MarginV",
            "Encoding",
        ];

        // 生成随机的 Format 字段顺序（可为空）与其字符串化形式
        #[derive(Clone, Debug)]
        pub struct StyleFormatSpec {
            pub fields: Vec<&'static str>,
        }

        impl StyleFormatSpec {
            pub fn to_format_line(&self) -> String {
                match self.fields.is_empty() {
                    true => "Format:".to_string(),
                    false => ["Format: ", &self.fields.join(", ")].concat(),
                }
            }
        }

        // 返回结构化对象，便于基于字段顺序生成对应的 Style 行并做断言
        // 为贴近实际，增加 Name 出现的概率：在空或缺 Name 时，50% 概率插入 Name
        pub fn style_format_spec_strategy() -> impl Strategy<Value = StyleFormatSpec> {
            prop::sample::subsequence(STYLEFIELDS, 0..=STYLEFIELDS.len())
                .prop_shuffle()
                .prop_flat_map(|mut fields: Vec<&'static str>| {
                    let has_name = fields.iter().any(|&f| f == "Name");
                    let should_insert = !has_name || fields.is_empty();
                    prop_oneof![
                        Just(fields.clone()),
                        Just({
                            if should_insert {
                                fields.insert(0, "Name");
                            }
                            fields
                        })
                    ]
                })
                .prop_map(|fields| StyleFormatSpec { fields })
        }

        pub fn comment_lines_strategy() -> impl Strategy<Value = Vec<String>> {
            prop::collection::vec("[^\r\n]*".prop_map(|s| format!("; {}", s)), 0..=3)
        }

        pub fn other_fields_strategy() -> impl Strategy<Value = Vec<String>> {
            prop::collection::vec(
                ("[^:\r\n]*", "[^\r\n]*").prop_map(|(s1, s2)| format!("!: {}: {}", s1, s2)),
                0..=5,
            )
        }
        // 构建期望的 StyleWithoutName，按照解析器的默认值与字段解析规则
        pub fn build_expected_style(
            name_str: &str,
            fields: &[&'static str],
            values: &[String],
        ) -> (String, StyleWithoutName) {
            // 解析辅助
            let mut fontname = "Arial".to_string();
            let mut fontsize: f64 = 18.0;
            let mut primary = AssColorRgba::new(255, 255, 255, 255);
            let mut secondary = AssColorRgba::new(0, 255, 255, 255);
            let mut outline_color = AssColorRgba::new(0, 0, 0, 0);
            let mut back_color = AssColorRgba::new(0, 0, 0, 128);
            let mut bold = BoldType::NonBold;
            let mut italic = false;
            let mut underline = false;
            let mut strikeout = false;
            let mut scale_x: f64 = 100.0;
            let mut scale_y: f64 = 100.0;
            let mut spacing: f64 = 0.0;
            let mut angle: f64 = 0.0;
            let mut border_style = BorderStyle::OutlineAndDropShadow;
            let mut outline: f64 = 2.0;
            let mut shadow: f64 = 2.0;
            let mut alignment = Alignment::BottomCenter;
            let mut margin_l: i32 = 10;
            let mut margin_r: i32 = 10;
            let mut margin_v: i32 = 10;
            let mut encoding: i32 = 1;

            for (idx, field) in fields.iter().enumerate() {
                let v = &values[idx];
                match *field {
                    "Fontname" => fontname = v.trim().to_string(),
                    "Fontsize" => {
                        // 如果解析成功，则更新值；否则，什么也不做，保留默认值
                        if let Ok(val) = v.parse::<f64>() {
                            fontsize = val;
                        }
                    }
                    "PrimaryColour" => {
                        if let Ok(val) = u32::from_str_radix(&v.trim_start_matches("&H"), 16) {
                            primary = AssColorRgba::from_ass_style_u32(val);
                        }
                    }
                    "SecondaryColour" => {
                        if let Ok(val) = u32::from_str_radix(&v.trim_start_matches("&H"), 16) {
                            secondary = AssColorRgba::from_ass_style_u32(val);
                        }
                    }
                    "OutlineColour" => {
                        if let Ok(val) = u32::from_str_radix(&v.trim_start_matches("&H"), 16) {
                            outline_color = AssColorRgba::from_ass_style_u32(val);
                        }
                    }
                    "BackColour" => {
                        if let Ok(val) = u32::from_str_radix(&v.trim_start_matches("&H"), 16) {
                            back_color = AssColorRgba::from_ass_style_u32(val);
                        }
                    }
                    "Bold" => {
                        bold = if v.trim() == "0" {
                            BoldType::NonBold
                        } else {
                            BoldType::Bold
                        }
                    }
                    "Italic" => {
                        if let Ok(val) = v.parse::<i32>() {
                            italic = val != 0;
                        }
                    }
                    "Underline" => {
                        if let Ok(val) = v.parse::<i32>() {
                            underline = val != 0;
                        }
                    }
                    "StrikeOut" => {
                        if let Ok(val) = v.parse::<i32>() {
                            strikeout = val != 0;
                        }
                    }
                    "ScaleX" => {
                        if let Ok(val) = v.parse::<f64>() {
                            scale_x = val;
                        }
                    }
                    "ScaleY" => {
                        if let Ok(val) = v.parse::<f64>() {
                            scale_y = val;
                        }
                    }
                    "Spacing" => {
                        if let Ok(val) = v.parse::<f64>() {
                            spacing = val;
                        }
                    }
                    "Angle" => {
                        if let Ok(val) = v.parse::<f64>() {
                            angle = val;
                        }
                    }
                    "BorderStyle" => {
                        if let Ok(val) = v.parse::<BorderStyle>() {
                            border_style = val;
                        }
                    }
                    "Outline" => {
                        if let Ok(val) = v.parse::<f64>() {
                            outline = val;
                        }
                    }
                    "Shadow" => {
                        if let Ok(val) = v.parse::<f64>() {
                            shadow = val;
                        }
                    }
                    "Alignment" => {
                        if let Ok(val) = v.parse::<Alignment>() {
                            alignment = val;
                        }
                    }
                    "MarginL" => {
                        if let Ok(val) = v.parse::<i32>() {
                            margin_l = val;
                        }
                    }
                    "MarginR" => {
                        if let Ok(val) = v.parse::<i32>() {
                            margin_r = val;
                        }
                    }
                    "MarginV" => {
                        if let Ok(val) = v.parse::<i32>() {
                            margin_v = val;
                        }
                    }
                    "Encoding" => {
                        if let Ok(val) = v.parse::<i32>() {
                            encoding = val;
                        }
                    }
                    // Name 在样式外层作为 key，不进入 StyleWithoutName
                    "Name" => {}
                    other => panic!("unexpected field in build_expected_style: {}", other),
                }
            }

            let style = StyleWithoutName {
                fontname,
                fontsize,
                primary_color: primary,
                secondary_color: secondary,
                outline_color,
                back_color,
                bold,
                italic,
                underline,
                strikeout,
                scale_x,
                scale_y,
                spacing,
                angle,
                border_style,
                outline,
                shadow,
                alignment,
                margin_l,
                margin_r,
                margin_v,
                encoding,
            };
            (name_str.to_string(), style)
        }

        pub const EVENTFIELDS: &[&str] = &[
            // NOTE: 对于旧版 Marked 字段需要直接忽略
            "Layer", "Start", "End", "Style", "Name", "MarginL", "MarginR", "MarginV", "Effect",
            "Text",
        ];

        #[derive(Clone, Debug)]
        pub struct EventFormatSpec {
            pub fields: Vec<&'static str>,
        }

        impl EventFormatSpec {
            pub fn to_format_line(&self) -> String {
                match self.fields.is_empty() {
                    true => "Format:".to_string(),
                    false => ["Format: ", &self.fields.join(", ")].concat(),
                }
            }
        }

        pub fn eventfields_format_spec_strategy() -> impl Strategy<Value = EventFormatSpec> {
            prop::sample::subsequence(EVENTFIELDS, 0..=EVENTFIELDS.len())
                .prop_shuffle()
                .prop_flat_map(|mut fields: Vec<&'static str>| {
                    // 确保至少有一个 "Text"
                    if !fields.iter().any(|&f| f == "Text") {
                        fields.push("Text");
                    }
                    // 将 "Text" 移动到末尾
                    if let Some(pos) = fields.iter().position(|&f| f == "Text") {
                        let text = fields.remove(pos);
                        fields.push(text);
                    }
                    Just(fields).boxed()
                })
                .prop_map(|fields| EventFormatSpec { fields })
        }
        pub fn build_expected_event(
            is_comment: bool,
            fields: &[&'static str],
            values: &[String],
        ) -> Event {
            use crate::types::event::{Dialogue, Effect, Event, TextSegment};
            use crate::types::time::AssTime;

            let mut layer = 0;
            let mut start = AssTime::from_centiseconds(0);
            let mut end = AssTime::from_centiseconds(0);
            let mut style = ("Default").to_string();
            let mut name = None;
            let mut margin_l = 0;
            let mut margin_r = 0;
            let mut margin_v = 0;
            let mut effect = None;
            let mut content = vec![TextSegment::Text("".to_string())];
            let mut text = String::new();

            for (idx, field) in fields.iter().enumerate() {
                let v = &values[idx];
                match *field {
                    "Layer" => {
                        if let Ok(val) = v.parse::<i32>() {
                            layer = val
                        }
                    }
                    "Start" => {
                        if let Ok(val) = v.parse::<AssTime>() {
                            start = val
                        }
                    }
                    "End" => {
                        if let Ok(val) = v.parse::<AssTime>() {
                            end = val
                        }
                    }
                    "Style" => style = v.trim().to_string(),
                    "Name" => {
                        let val = v.trim();
                        if !val.is_empty() {
                            name = Some(val.to_string())
                        }
                    }
                    "MarginL" => {
                        if let Ok(val) = v.parse::<i32>() {
                            margin_l = val
                        }
                    }
                    "MarginR" => {
                        if let Ok(val) = v.parse::<i32>() {
                            margin_r = val
                        }
                    }
                    "MarginV" => {
                        if let Ok(val) = v.parse::<i32>() {
                            margin_v = val
                        }
                    }
                    "Effect" => {
                        if let Ok(val) = v.parse::<Effect>() {
                            effect = Some(val)
                        }
                    }
                    "Text" => {
                        if is_comment {
                            text = v.to_string()
                        } else {
                            if let Ok((rest, val)) = parse_dialogue_line(v) {
                                if !rest.is_empty() {
                                    warn!("未完全解析 Text，剩余: {rest}")
                                }
                                content = val
                            }
                        }
                    }
                    other => panic!("unexpected field in build_expected_event: {}", other),
                }
            }

            if is_comment {
                Event::Comment(Comment {
                    layer,
                    start,
                    end,
                    style,
                    name,
                    margin_l,
                    margin_r,
                    margin_v,
                    effect,
                    text,
                })
            } else {
                Event::Dialogue(Dialogue {
                    layer,
                    start,
                    end,
                    style,
                    name,
                    margin_l,
                    margin_r,
                    margin_v,
                    effect,
                    content,
                })
            }
        }
    }

    mod script_info_prop {
        use super::support::*;
        use crate::parser::process_script_info_chunk;
        use crate::parser::ParsedSection;
        use crate::types::{
            script::{ScriptType, YCbCrMatrix},
            WrapStyle,
        };
        use isolang::languages;
        use proptest::prelude::*;

        proptest! {
            #[test]
            fn test_script_info_all_fields_valid_prop(
                comment_lines in comment_lines_strategy(),
                script_type in prop::sample::select(SCRIPTTYPES).prop_map(|s| s.to_string()),
                layout_res_x in any::<u32>(),
                layout_res_y in any::<u32>(),
                wrap_style in prop::sample::select(vec![0, 1, 2]),
                timer in any::<f64>(),
                scaled in prop_oneof![
                    Just("yes".to_string()),
                    Just("no".to_string()),
                    any::<i32>().prop_map(|n| n.to_string()),
                ],
                kerning in prop_oneof![
                    Just("yes".to_string()),
                    Just("no".to_string()),
                    any::<i32>().prop_map(|n| n.to_string()),
                ],
                language in prop::sample::select(
                    languages().filter_map(|l| l.to_639_1()).collect::<Vec<&str>>()
                ),
                ycbcr in prop::sample::select(YCBCRMATRIXS).prop_map(|s| s.to_string()),
                title in "[^\r\n]*",
                original_script in "[^\r\n]*",
                original_translation in "[^\r\n]*",
                original_editing in "[^\r\n]*",
                original_timing in "[^\r\n]*",
                script_updated_by in "[^\r\n]*",
                update_details in "[^\r\n]*",
                other_fields in other_fields_strategy()
            ) {
                let mut lines = Vec::new();
                lines.extend(comment_lines);
                lines.push(format!("ScriptType: {}", script_type));
                lines.push(format!("LayoutResX: {}", layout_res_x));
                lines.push(format!("LayoutResY: {}", layout_res_y));
                // 同步 PlayRes，避免覆盖造成歧义
                lines.push(format!("PlayResX: {}", layout_res_x));
                lines.push(format!("PlayResY: {}", layout_res_y));
                lines.push(format!("WrapStyle: {}", wrap_style));
                lines.push(format!("Timer: {}", timer));
                lines.push(format!("ScaledBorderAndShadow: {}", scaled));
                lines.push(format!("Kerning: {}", kerning));
                lines.push(format!("Language: {}", language));
                lines.push(format!("YCbCr Matrix: {}", ycbcr));
                lines.push(format!("Title: {}", title));
                lines.push(format!("Original Script: {}", original_script));
                lines.push(format!("Original Translation: {}", original_translation));
                lines.push(format!("Original Editing: {}", original_editing));
                lines.push(format!("Original Timing: {}", original_timing));
                lines.push(format!("Script Updated By: {}", script_updated_by));
                lines.push(format!("Update Details: {}", update_details));
                lines.extend(other_fields);

                match process_script_info_chunk(lines) {
                    ParsedSection::ScriptInfo(si) => {
                        // 基本断言
                        let expected_script_type = match script_type.as_str() {
                            "v4.00" => ScriptType::SSAv4,
                            "v4.00+" => ScriptType::ASS,
                            "v4.00++" => ScriptType::ASS2,
                            _ => unreachable!(),
                        };
                        assert_eq!(si.script_type, expected_script_type);
                        assert_eq!(si.layout_res_x, layout_res_x);
                        assert_eq!(si.layout_res_y, layout_res_y);
                        assert_eq!(si.wrap_style, match wrap_style { 0 => WrapStyle::Smart, 1 => WrapStyle::EndOfWord, 2 => WrapStyle::NoWrap, 3 => WrapStyle::Smart, _ => WrapStyle::Smart });
                        // YCbCr 映射
                        let expected_ycbcr = match ycbcr.as_str() {
                            "None" => YCbCrMatrix::None,
                            "TV.601" => YCbCrMatrix::BT601TV,
                            "PC.601" => YCbCrMatrix::BT601PC,
                            "TV.709" => YCbCrMatrix::BT709TV,
                            "PC.709" => YCbCrMatrix::BT709PC,
                            "TV.240m" => YCbCrMatrix::SMPTE240MTV,
                            "PC.240m" => YCbCrMatrix::SMPTE240MPC,
                            "TV.fcc" => YCbCrMatrix::FCCTV,
                            "PC.fcc" => YCbCrMatrix::FCCPC,
                            _ => unreachable!(),
                        };
                        assert_eq!(si.ycbcr_matrix, expected_ycbcr);
                        // Timer
                        assert_eq!(si.timer, Some(timer));
                        // 验证 yes 或非0整数 => true；no 或 0 => false；其他 => false
                        let expected_scaled = {
                            let t = scaled.trim().to_ascii_lowercase();
                            if t == "yes" { true }
                            else if let Ok(n) = t.parse::<i64>() { n != 0 }
                            else { false }
                        };
                        let expected_kerning = {
                            let t = kerning.trim().to_ascii_lowercase();
                            if t == "yes" { true }
                            else if let Ok(n) = t.parse::<i64>() { n != 0 }
                            else { false }
                        };
                        assert_eq!(si.scaled_border_and_shadow, expected_scaled);
                        assert_eq!(si.kerning, Some(expected_kerning));
                        // Language
                        assert_eq!(si.language.unwrap().to_639_1(), Some(language));
                    }
                    other => panic!("Expected ScriptInfo, got: {:?}", other),
                }
            }

            #[test]
            fn test_script_info_all_fields_error_tolerant_prop(
                comment_lines in comment_lines_strategy(),
                layout_res_x in any::<u32>(),
                layout_res_y in any::<u32>(),
                wrap_style in prop::sample::select(vec![0, 1, 2]),
                timer in any::<f64>(),
                // 非法 ScriptType（将被映射为 ASS）
                bad_script_type in "[^\r\n]*".prop_filter("必须是非法 ScriptType 值", |s| !SCRIPTTYPES.contains(&s.as_str())),
                // 非法但可容错的布尔类
                bad_bool1 in "[^\r\n]*".prop_filter("trim 后既非 yes/no 也非整数", |s| {
                    let t = s.trim();
                    let lower = t.to_ascii_lowercase();
                    if lower == "yes" || lower == "no" { return false; }
                    t.parse::<i64>().is_err()
                }),
                bad_bool2 in "[^\r\n]*".prop_filter("trim 后既非 yes/no 也非整数", |s| {
                    let t = s.trim();
                    let lower = t.to_ascii_lowercase();
                    if lower == "yes" || lower == "no" { return false; }
                    t.parse::<i64>().is_err()
                }),
                language in prop::sample::select(
                    languages().filter_map(|l| l.to_639_1()).collect::<Vec<&str>>()
                ),
                // 非法 YCbCr（将被映射为 Unknown）
                bad_ycbcr in "[^\r\n]*".prop_filter("必须是非法 YCbCr 值", |s| !YCBCRMATRIXS.contains(&s.as_str())),
                title in "[^\r\n]*",
                original_script in "[^\r\n]*",
                original_translation in "[^\r\n]*",
                original_editing in "[^\r\n]*",
                original_timing in "[^\r\n]*",
                script_updated_by in "[^\r\n]*",
                update_details in "[^\r\n]*",
                other_fields in other_fields_strategy()
            ) {
                let mut lines = Vec::new();
                lines.extend(comment_lines);
                lines.push(format!("ScriptType: {}", bad_script_type));
                lines.push(format!("LayoutResX: {}", layout_res_x));
                lines.push(format!("LayoutResY: {}", layout_res_y));
                lines.push(format!("PlayResX: {}", layout_res_x));
                lines.push(format!("PlayResY: {}", layout_res_y));
                lines.push(format!("WrapStyle: {}", wrap_style));
                lines.push(format!("Timer: {}", timer));
                // 错误但可容错的值（不触发 panic，不导致缺失）
                lines.push(format!("ScaledBorderAndShadow: {}", bad_bool1));
                lines.push(format!("Kerning: {}", bad_bool2));
                lines.push(format!("Language: {}", language));
                lines.push(format!("YCbCr Matrix: {}", bad_ycbcr));
                lines.push(format!("Title: {}", title));
                lines.push(format!("Original Script: {}", original_script));
                lines.push(format!("Original Translation: {}", original_translation));
                lines.push(format!("Original Editing: {}", original_editing));
                lines.push(format!("Original Timing: {}", original_timing));
                lines.push(format!("Script Updated By: {}", script_updated_by));
                lines.push(format!("Update Details: {}", update_details));
                lines.extend(other_fields);

                match process_script_info_chunk(lines) {
                    ParsedSection::ScriptInfo(si) => {
                        assert_eq!(si.script_type, ScriptType::ASS);
                        assert_eq!(si.layout_res_x, layout_res_x);
                        assert_eq!(si.layout_res_y, layout_res_y);
                        assert_eq!(si.wrap_style, match wrap_style { 0 => WrapStyle::Smart, 1 => WrapStyle::EndOfWord, 2 => WrapStyle::NoWrap, 3 => WrapStyle::Smart, _ => WrapStyle::Smart });
                        // 非法 YCbCr 应被映射为 Unknown
                        assert!(matches!(si.ycbcr_matrix, YCbCrMatrix::Unknown));
                        // 非法布尔应容错为 false
                        assert_eq!(si.scaled_border_and_shadow, false);
                        assert_eq!(si.kerning, Some(false));
                        // 其他关键字段存在
                        assert!(si.timer.is_some());
                        assert_eq!(si.language.unwrap().to_639_1(), Some(language));
                    }
                    other => panic!("Expected ScriptInfo, got: {:?}", other),
                }
            }
        }
        proptest! {
            #[test]
            fn test_script_info_missing_fields_defaults_prop(
                // 生成不包含换行的非法 YCbCr 值
                bad_ycbcr in "[^\r\n]*".prop_filter(
                    "必须是非法 YCbCr 值",
                    |s| !YCBCRMATRIXS.contains(&s.as_str())
                ),
                // ScaledBorderAndShadow 支持 yes/no 或整数
                scaled in prop_oneof![
                    Just("yes".to_string()),
                    Just("no".to_string()),
                    any::<i32>().prop_map(|n| n.to_string()),
                ],
            ) {
                let lines = vec![
                    format!("ScaledBorderAndShadow: {}", scaled),
                    format!("YCbCr Matrix: {}", bad_ycbcr),
                ];
                let result = process_script_info_chunk(lines);
                match result {
                    ParsedSection::ScriptInfo(si) => {
                        assert_eq!(si.script_type, ScriptType::ASS);
                        // 非法 YCbCr 应被折叠为 Unknown
                        assert!(matches!(si.ycbcr_matrix, YCbCrMatrix::Unknown));
                        // 未提供分辨率与 WrapStyle，应回退到默认
                        assert_eq!(si.layout_res_x, 1920);
                        assert_eq!(si.layout_res_y, 1080);
                        assert_eq!(si.wrap_style, WrapStyle::Smart);
                    }
                    other => panic!("Expected ScriptInfo, got: {:?}", other),
                }
            }
        }
    }

    mod styles_prop {
        use super::support::*;
        use crate::parser::{process_styles_chunk, ParsedSection};
        use crate::types::style::StyleWithoutName;
        use proptest::prelude::*;

        #[test]
        fn test_style_format_absent_ok() {
            // 完全没有 Format 行与 Style 行，应当回退并成功（得到空样式）
            let lines: Vec<String> = vec![];
            match process_styles_chunk(lines) {
                ParsedSection::Styles(styles) => assert!(styles.is_empty()),
                other => panic!("Expected Styles, got: {:?}", other),
            }
        }

        proptest! {
            #[test]
            fn test_style_format_random_subset_and_order_prop(
                spec in style_format_spec_strategy()
            ) {
                // 仅包含一个随机 Format 行（可为空）的样本 [V4+ Styles] 内容
                let lines = vec![spec.to_format_line()];
                match process_styles_chunk(lines) {
                    ParsedSection::Styles(styles) => {
                        // 没有 Style: 行，解析应成功且样式表为空
                        assert!(styles.is_empty());
                    }
                    other => panic!("Expected Styles, got: {:?}", other),
                }
            }
        }

        // 当 Format 含 Name 或为空（默认顺序含 Name），生成 Style 并断言字段映射
        proptest! {
            #[test]
            fn test_style_line_matches_format_and_values_prop(
                spec in style_format_spec_strategy(),
                // 字符串：不含换行与逗号
                name in "[^,\r\n]*",
                fontname in "[^,\r\n]*",
                // 浮点
                fsz in any::<f64>(),
                // 颜色：使用 &H 十六进制
                c1 in any::<u32>(), c2 in any::<u32>(), c3 in any::<u32>(), c4 in any::<u32>(),
                // 布尔类（整数）
                bold in any::<i32>(), italic in any::<i32>(), underline in any::<i32>(), strike in any::<i32>(),
                // 比例/间距/角度
                sx in any::<f64>(), sy in any::<f64>(),
                sp in any::<f64>(), ang in any::<f64>(),
                // 边框样式、描边、阴影
                bstyle in prop::sample::select(vec![1,3]), out in any::<f64>(), sh in any::<f64>(),
                // 对齐与边距、编码
                align in 1i32..=9, ml in any::<i32>(), mr in any::<i32>(), mv in any::<i32>(), enc in any::<i32>(),
                // 随机数量（u8），限制为 0..=8 避免过大
                count_u8 in any::<u8>()
            ) {
                let effective_fields: Vec<&'static str> = if spec.fields.is_empty() { STYLEFIELDS.to_vec() } else { spec.fields.clone() };
                prop_assume!(effective_fields.iter().any(|&f| f == "Name"));
                let name_idx = effective_fields.iter().position(|&f| f == "Name").unwrap();
                let format_line = StyleFormatSpec { fields: spec.fields.clone() }.to_format_line();
                let mut lines = vec![format_line];
                let mut expected: Vec<(String, StyleWithoutName)> = Vec::new();

                for i in 0..count_u8 {
                    let name_i = format!("{}-{}", name.trim(), i);
                    let fontname_i = format!("{}-{}", fontname, i);
                    let fsz_i = fsz + i as f64 ;
                    let sx_i = sx + i as f64;
                    let sy_i = sy - i as f64;
                    let sp_i = sp.abs() + i as f64;
                    let ang_i = ang - i as f64;
                    let out_i = out.abs() + i as f64;
                    let sh_i = sh.abs() + i as f64;
                    let c1_i = c1.wrapping_add(i as u32);
                    let c2_i = c2.wrapping_add(i as u32);
                    let c3_i = c3.wrapping_add(i as u32);
                    let c4_i = c4.wrapping_add(i as u32);
                    let bold_i = bold.wrapping_add(i as i32) as i32;
                    let italic_i = italic + i as i32;
                    let underline_i = underline + i as i32;
                    let strike_i = strike + i as i32;
                    let bstyle_i = if i % 2 == 0 { bstyle } else { if bstyle == 1 { 3 } else { 1 } };
                    let align_i = ((align + i as i32) % 9) + 1;
                    let ml_i = ml.wrapping_add(i as i32);
                    let mr_i = mr.wrapping_add(i as i32);
                    let mv_i = mv.wrapping_add(i as i32);
                    let enc_i = enc.wrapping_add(i as i32);

                    let values = effective_fields.iter().map(|&f| match f {
                        "Name" => name_i.clone(),
                        "Fontname" => fontname_i.clone(),
                        "Fontsize" => format!("{:.3}", fsz_i),
                        "PrimaryColour" => format!("&H{:08X}", c1_i),
                        "SecondaryColour" => format!("&H{:08X}", c2_i),
                        "OutlineColour" => format!("&H{:08X}", c3_i),
                        "BackColour" => format!("&H{:08X}", c4_i),
                        "Bold" => bold_i.to_string(),
                        "Italic" => italic_i.to_string(),
                        "Underline" => underline_i.to_string(),
                        "StrikeOut" => strike_i.to_string(),
                        "ScaleX" => format!("{:.3}", sx_i),
                        "ScaleY" => format!("{:.3}", sy_i),
                        "Spacing" => format!("{:.3}", sp_i),
                        "Angle" => format!("{:.3}", ang_i),
                        "BorderStyle" => bstyle_i.to_string(),
                        "Outline" => format!("{:.3}", out_i),
                        "Shadow" => format!("{:.3}", sh_i),
                        "Alignment" => align_i.to_string(),
                        "MarginL" => ml_i.to_string(),
                        "MarginR" => mr_i.to_string(),
                        "MarginV" => mv_i.to_string(),
                        "Encoding" => enc_i.to_string(),
                        other => panic!("unexpected field {}", other),
                    }).collect::<Vec<_>>();

                    lines.push(["Style: ", &values.join(",")].concat());
                    let (exp_name, exp_style) = build_expected_style(&values[name_idx], &effective_fields, &values);
                    expected.push((exp_name, exp_style));
                }

                match process_styles_chunk(lines) {
                    ParsedSection::Styles(styles) => {
                        assert_eq!(styles.len(), expected.len());
                        for (n, s) in expected { assert_eq!(styles.get(&n), Some(&s)); }
                    }
                    other => panic!("Expected Styles, got: {:?}", other),
                }
            }
        }

        // 当 Format 不含 Name：应将样式名视为 "Default"，且后定义覆盖先定义
        proptest! {
            #[test]
            fn test_style_line_ignored_without_name_field_prop(
                spec in style_format_spec_strategy()
            ) {
                prop_assume!(!spec.fields.is_empty());
                prop_assume!(!spec.fields.iter().any(|&f| f == "Name"));

                // 为各字段生成占位值（不重要，因为应被忽略）
                let values: Vec<String> = spec.fields.iter().map(|&f| {
                    // 使用简单可解析的占位值
                    match f {
                        "Fontname" | "Name" => "x".to_string(),
                        "PrimaryColour" | "SecondaryColour" | "OutlineColour" | "BackColour" => "&H00000000".to_string(),
                        "Alignment" => "3".to_string(),
                        "BorderStyle" => "1".to_string(),
                        _ => "0".to_string(),
                    }
                }).collect();

                let format_line = StyleFormatSpec { fields: spec.fields.clone() }.to_format_line();
                // 生成多条，验证覆盖
                let style_line1 = ["Style: ", &values.join(",")].concat();
                let style_line2 = ["Style: ", &values.join(",")].concat();
                let lines = vec![format_line, style_line1, style_line2];
                match process_styles_chunk(lines) {
                    ParsedSection::Styles(styles) => {
                        assert_eq!(styles.len(), 1);
                        let (exp_name, exp_style) = build_expected_style(
                            "Default",
                            &spec.fields,
                            &values,
                        );
                        let got = styles.get(&exp_name).expect("Default style not found");
                        assert_eq!(got, &exp_style);
                    }
                    other => panic!("Expected Styles, got: {:?}", other),
                }
            }
        }
    }
    mod events_prop {
        use crate::{
            parser::{
                parse_events_section,
                tests::support::{build_expected_event, eventfields_format_spec_strategy},
                ParsedSection,
            },
            types::event::Event,
        };
        use proptest::prelude::*;

        proptest! {
            #[test]
            fn test_event_all_fields_valid_prop(
                format in eventfields_format_spec_strategy(),
                layer in any::<i32>(),
                // INFO: libass 支持负数时间
                start_time in any::<i32>(),
                end_time in any::<i32>(),
                style in "[^,\r\n]*",
                name in "[^,\r\n]*",
                ml in any::<i32>(),
                mr in any::<i32>(),
                mv in any::<i32>(),
                effect_y1 in any::<i32>(),
                effect_y2 in any::<i32>(),
                effect_delay in any::<i32>(),
                effect_direction in any::<i32>(),
                effect_type in 0u8..3,
                text in "[^\r\n]*",
                // 随机数量
                count_u32 in any::<u8>()
            ) {

                // HACK: 没有测试完全没有 Format 的可能性
                let format_fields: Vec<&'static str> = format.fields.clone();
                let format_line = format.to_format_line();
                let mut lines = vec![format_line];
                let mut expected: Vec<Event> = Vec::new();


                for i in 0..count_u32 {
                    let start_hour = start_time.saturating_sub(i as i32);
                    let start_minu = start_time.saturating_add(i as i32 * 60);
                    let start_sec = start_time.saturating_sub(i as i32);
                    let start_csec = start_time.saturating_sub(i as i32);
                    let end_hour = end_time.saturating_sub(i as i32);
                    let end_minu = end_time.saturating_add(i as i32 * 60);
                    let end_sec = end_time.saturating_add(i as i32);
                    let end_csec = end_time.saturating_add(i as i32);

                    let is_comment_i = i % 10 == 0;
                    let layer_i = layer.wrapping_add(i as i32);
                    let style_i = format!("{}-{}", style, i);
                    let name_i = format!("{}-{}", name, i);
                    let ml_i = ml.wrapping_add(i as i32);
                    let mr_i = mr.wrapping_add(i as i32);
                    let mv_i = mv.wrapping_add(i as i32);
                    let effect_str = match effect_type {
                        0 => {
                            let delay_i = effect_delay.wrapping_add(i as i32);
                            let direction_i = effect_direction.wrapping_add(i as i32);
                            format!("Banner;{};{}", delay_i, direction_i)

                        },
                        1 => {
                            let y1_i = effect_y1.wrapping_add(i as i32);
                            let y2_i = effect_y2.wrapping_add(i as i32);
                            let delay_i = effect_delay.wrapping_add(i as i32);
                            format!("Scroll up;{};{};{}", y1_i, y2_i, delay_i)
                        },
                        2 => {

                            let y1_i = effect_y1.wrapping_add(i as i32);
                            let y2_i = effect_y2.wrapping_add(i as i32);
                            let delay_i = effect_delay.wrapping_add(i as i32);
                            format!("Scroll down;{};{};{}", y1_i, y2_i, delay_i)
                        },
                        _ => unreachable!("范围错误")
                    };
                    let text_1 = format!("{}-{}", text, i);

                    let values = format_fields.iter().map(|&f| match f {
                        "Layer" => layer_i.to_string(),
                        "Start" => format!("{start_hour}:{start_minu:02}:{start_sec:02}.{start_csec:02}"),
                        "End" => format!("{end_hour}:{end_minu:02}:{end_sec:02}.{end_csec:02}"),
                        "Style" => style_i.to_string(),
                        "Name" => name_i.to_string(),
                        "MarginL" => ml_i.to_string(),
                        "MarginR" => mr_i.to_string(),
                        "MarginV" => mv_i.to_string(),
                        "Effect" => effect_str.to_string(),
                        "Text" => text_1.to_string(),
                        other => panic!("unexpected field {}", other),
                    }).collect::<Vec<_>>();

                    if is_comment_i {
                        lines.push(["Comment: ", &values.join(",")].concat());
                    } else {
                        lines.push(["Dialogue: ", &values.join(",")].concat());
                    }
                    expected.push(build_expected_event(is_comment_i, &format_fields, &values))
                }
                match parse_events_section(lines) {
                    ParsedSection::Events(events) => {
                        assert_eq!(events.len(), expected.len());
                        assert_eq!(events, expected);
                    },

                    other => panic!("Expected Events, got: {:?}", other),
                }
            }
        }
    }
}
