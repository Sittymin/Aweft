use crate::types::{
    event::{Event, EventParser},
    script::{parse_script_info_line, ScriptInfo, ScriptInfoBuilder},
    style::{StyleParser, StyleWithoutName},
    *,
};
use anyhow::{anyhow, Result};
use core::panic;
use crossbeam_channel::bounded;
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
        if let Err(e) = parser.learn_format(format_line.strip_prefix("Format:").unwrap().trim()) {
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
        if let Err(e) = parser.learn_format(format_line.strip_prefix("Format:").unwrap().trim()) {
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
    type Item = anyhow::Result<SectionContent>;

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
        let (work_sender, work_receiver) =
            bounded::<anyhow::Result<SectionContent>>(num_cpus::get() * 2);

        // `result_channel`: 消费者 -> 主线程，发送处理完的结果
        let (result_sender, result_receiver) = bounded::<ParsedSection>(num_cpus::get() * 2);

        // 使用 Rayon 的 `scope` 可以更安全地处理线程和错误
        rayon::scope(|s| {
            // 在 Rayon scope 内启动一个生产者
            s.spawn(move |_| {
                for section_result in section_iterator {
                    if work_sender.send(section_result).is_err() {
                        break; // 消费者已关闭
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
                            eprintln!("Producer IO Error: {:?}", e);
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
                            eprintln!("Parsing Error: {}", e);
                        }
                        parsed => {
                            if result_sender.send(parsed).is_err() {
                                eprintln!("Result receiver hung up, could not send parsed chunk.");
                            }
                        }
                    }
                });
        });

        let mut script_info = None;
        let mut styles = None;
        let mut events = None;

        for parsed_result in result_receiver {
            match parsed_result {
                ParsedSection::ScriptInfo(si) => script_info = Some(si),
                ParsedSection::Styles(s) => styles = Some(s),
                ParsedSection::Events(e) => events = Some(e),
                ParsedSection::Error(_) => unreachable!("不会发送错误到最终输出"),
            }
        }

        Ok(ASSFile {
            script_info: script_info.ok_or_else(|| {
                anyhow!("Parsing failed: [Script Info] section not found or failed to parse")
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
            panic!("No script type other than ASS has been implemented")
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
    use isolang::languages;
    use proptest::prelude::*;

    use crate::parser::process_script_info_chunk;
    use crate::types::{
        script::{ScriptType, YCbCrMatrix},
        WrapStyle,
    };

    const SCRIPTTYPES: &[&str] = &[
        "v4.00", "v4.00+", "v4.00++"
    ];

    const YCBCRMATRIXS: &[&str] = &[
        "None", "TV.601", "PC.601", "TV.709", "PC.709", "TV.240m", "PC.240m", "TV.fcc", "PC.fcc",
    ];

    fn comment_lines_strategy() -> impl Strategy<Value = Vec<String>> {
        prop::collection::vec("[^\r\n]*".prop_map(|s| format!("; {}", s)), 0..=3)
    }

    fn other_fields_strategy() -> impl Strategy<Value = Vec<String>> {
        prop::collection::vec(
            ("[^:\r\n]*", "[^\r\n]*").prop_map(|(s1, s2)| format!("!: {}: {}", s1, s2)),
            0..=5,
        )
    }

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
                super::ParsedSection::ScriptInfo(si) => {
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
                    assert_eq!(si.wrap_style, match wrap_style { 0 => WrapStyle::Smart, 1 => WrapStyle::EndWord, _ => WrapStyle::NoWrap });
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
                super::ParsedSection::ScriptInfo(si) => {
                    assert_eq!(si.script_type, ScriptType::ASS);
                    assert_eq!(si.layout_res_x, layout_res_x);
                    assert_eq!(si.layout_res_y, layout_res_y);
                    assert_eq!(si.wrap_style, match wrap_style { 0 => WrapStyle::Smart, 1 => WrapStyle::EndWord, _ => WrapStyle::NoWrap });
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
                super::ParsedSection::ScriptInfo(si) => {
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
