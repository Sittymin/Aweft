use crate::types::{
    event::EventParser,
    script::{parse_script_info_line, ScriptInfoBuilder},
    style::StyleParser,
    *,
};
use anyhow::{anyhow, Context, Result};
use core::panic;
use std::{
    collections::HashMap,
    fs::File,
    io::{BufRead, BufReader, BufWriter, Write},
    path::{Path, PathBuf},
};

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

        let mut script_info_builder = ScriptInfoBuilder::new();

        let mut style_parser = StyleParser::default();
        let mut styles = HashMap::new();

        let mut event_parser = EventParser::default();
        let mut events = Vec::new();

        let mut current_section: Option<Section> = None;

        for line_result in reader.lines() {
            let line = line_result?;
            let line = line.trim();
            if line.is_empty() || line.starts_with(';') {
                // TODO: 注释可能在以后有用(PS: Event 当中也有注释，也可以自定义`[Aweft]`字段)
                continue;
            }

            if let Some(section_name) = line.strip_prefix('[').and_then(|s| s.strip_suffix(']')) {
                current_section = match section_name {
                    "Script Info" => Some(Section::ScriptInfo),
                    "V4+ Styles" => Some(Section::V4PStyles),
                    "Events" => Some(Section::Events),
                    unknown => {
                        eprintln!("Warning: Skipping unknown section: '{}'", unknown);
                        None
                    }
                };
                continue;
            } else if let Some(section) = current_section {
                match section {
                    Section::ScriptInfo => match parse_script_info_line(&line) {
                        Ok((_, entry)) => {
                            script_info_builder.apply_entry(entry);
                        }
                        Err(_) => {
                            eprintln!(
                                "Warning: Skipping unparsable line in [Script Info]: {}",
                                line
                            );
                        }
                    },
                    // TODO: impl SSA v4 and ASS2
                    Section::V4PStyles => {
                        if let Some(format_data) = line.strip_prefix("Format:") {
                            style_parser
                                .learn_format(format_data)
                                .context("Failed to parse 'Format:' line")?;
                        } else {
                            let (name, style) =
                                style_parser.parse_style_line(line).with_context(|| {
                                    format!("Failed to parse style line: '{}'", line)
                                })?;
                            styles.insert(name, style);
                        }
                    }
                    Section::Events => {
                        if let Some(format_data) = line.strip_prefix("Format:") {
                            event_parser
                                .learn_format(format_data)
                                .context("Failed to parse 'Format:' line")?;
                        } else {
                            let event = event_parser.parse_event_line(line).with_context(|| {
                                format!("Failed to parse style line: '{}'", line)
                            })?;
                            events.push(event);
                        }
                    }
                }
            }
        }
        Ok(ASSFile {
            script_info: script_info_builder
                .build() // HACK: 需要为 script_info_builder 实现错误类型
                .map_err(|e| anyhow!(e))
                .with_context(|| format!("Failed to build script_info",))?,
            styles,
            events,
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
