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
