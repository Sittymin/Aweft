use super::types::*;
use std::collections::HashMap;
use std::fs::File;
use std::io::{self, BufRead, BufReader};

impl ASSFile {
    pub fn parse_from_file(path: &str) -> io::Result<Self> {
        let file = File::open(path)?;
        let reader = BufReader::new(file);

        let mut ass_file = ASSFile {
            script_info: ScriptInfo::new(),
            styles: HashMap::new(),
            events: Vec::new(),
        };

        let mut current_section = None;

        for line_result in reader.lines() {
            let line = line_result?;
            let line = line.trim();
            if line.is_empty() || line.starts_with(';') {
                continue; // 跳过空行和注释
            }

            if line.starts_with('[') && line.ends_with(']') {
                // 识别新的部分
                let section_name = &line[1..line.len() - 1];
                current_section = Some(section_name.to_string());
                continue;
            }

            if let Some(ref section_name) = current_section {
                match section_name.as_str() {
                    "Script Info" => {
                        parse_script_info_line(line, &mut ass_file.script_info);
                    }
                    "V4 Styles" | "V4+ Styles" => {
                        parse_styles_line(line, &mut ass_file.styles);
                    }
                    "Events" => {
                        parse_events_line(line, &mut ass_file.events);
                    }
                    _ => {
                        // 忽略未知的部分
                    }
                }
            }
        }

        Ok(ass_file)
    }
}

fn parse_script_info_line(line: &str, script_info: &mut ScriptInfo) {
    if let Some((key, value)) = line.split_once(':') {
        let key = key.trim();
        let value = value.trim();
        // 根据键更新 script_info，相应地处理键值对
        script_info.insert(key.to_string(), value.to_string());
    }
}

fn parse_styles_line(line: &str, styles: &mut HashMap<String, StyleWithoutName>) {
    if line.starts_with("Format:") {
        // 解析格式行，确定字段顺序
        // 存储格式信息以便后续解析
    } else if line.starts_with("Style:") {
        // 解析样式定义行
        let style_data = &line[6..].trim();
        // 使用之前解析的格式信息来解析 style_data
        // 创建 StyleWithoutName 对象并插入到 styles 中
        let style = parse_style(style_data);
        styles.insert(style.name.clone(), style);
    }
}

fn parse_events_line(line: &str, events: &mut Vec<Event>) {
    if line.starts_with("Format:") {
        // 解析格式行，确定字段顺序
        // 存储格式信息以便后续解析
    } else if line.starts_with("Dialogue:") {
        // 解析对话事件行
        let event_data = &line[9..].trim();
        // 使用之前解析的格式信息来解析 event_data
        let event = parse_event(event_data);
        events.push(event);
    }
}

fn parse_style(data: &str) -> StyleWithoutName {
    // 假设已经有字段顺序的信息
    // 将 data 按照 ',' 分割并映射到字段
    // 创建并返回 StyleWithoutName 对象
    // 注意这里应该避免不必要的拷贝
    StyleWithoutName {
        // 初始化字段
    }
}

fn parse_event(data: &str) -> Event {
    // 类似地，解析 event_data 并创建 Event 对象
    Event {
        // 初始化字段
    }
}
