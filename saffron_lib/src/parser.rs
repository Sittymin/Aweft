use super::types::*;
use std::collections::HashMap;
use std::fs::File;
use std::io::{self, BufRead, BufReader};

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_parse_script_info_line_normal_field() {
        let mut script_info = ScriptInfo::new();

        let result = parse_script_info_line("Title: Test Title", &mut script_info);
        assert!(result.is_ok());
        assert_eq!(script_info.title, Some("Test Title".to_string()));

        let result = parse_script_info_line("ScriptType: v4.00+", &mut script_info);
        assert!(result.is_ok());
        assert_eq!(script_info.script_type, ScriptType::ASS);

        let result = parse_script_info_line("PlayResX: 1920", &mut script_info);
        assert!(result.is_ok());
        assert_eq!(script_info.layout_res_x, 1920);
    }

    #[test]
    fn test_parse_script_info_line_custom_field() {
        let mut script_info = ScriptInfo::new();

        let result = parse_script_info_line("!Custom: Value", &mut script_info);
        assert!(result.is_ok());
        assert_eq!(
            script_info.other_fields.get("Custom"),
            Some(&"Value".to_string())
        );
    }

    #[test]
    fn test_parse_script_info_line_invalid_input() {
        let mut script_info = ScriptInfo::new();

        let result = parse_script_info_line("PlayResX: invalid", &mut script_info);
        assert!(result.is_err());
        let result = parse_script_info_line("ScriptType: invalid", &mut script_info);
        assert!(result.is_err());
    }

    #[test]
    fn test_parse_script_info_line_malformed() {
        let mut script_info = ScriptInfo::new();

        let result = parse_script_info_line("Invalid Line", &mut script_info);
        assert!(result.is_ok());

        let result = parse_script_info_line("Invalid Line:", &mut script_info);
        assert!(result.is_ok());

        let result = parse_script_info_line(":Invalid Line", &mut script_info);
        assert!(result.is_ok());

        let result = parse_script_info_line(":", &mut script_info);
        assert!(result.is_ok());

        let result = parse_script_info_line("", &mut script_info);
        assert!(result.is_ok());
    }
}

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
                        if let Err(e) = parse_script_info_line(line, &mut ass_file.script_info) {
                            eprintln!("ScriptInfo line error: {}", e)
                        };
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

fn parse_script_info_line(line: &str, script_info: &mut ScriptInfo) -> Result<(), String> {
    if let Some((key, value)) = line.split_once(':') {
        let key = key.trim();
        if key.is_empty() {
            return Ok(());
        }

        let value = value.trim();
        if value.is_empty() {
            return Ok(());
        }

        if key.starts_with("!") {
            script_info.add_or_update_other_fields(key[1..].to_string(), value.to_string())?;
        } else {
            script_info.set_field(key, value)?;
        }
    }
    Ok(())
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
        // TODO: let style = parse_style(style_data);
        // TODO: styles.insert(style.name.clone(), style);
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
        // TODO: let event = parse_event(event_data);
        // events.push(event);
    }
}

/* fn parse_style(data: &str) -> StyleWithoutName {
    // 假设已经有字段顺序的信息
    // 将 data 按照 ',' 分割并映射到字段
    // 创建并返回 StyleWithoutName 对象
    // 注意这里应该避免不必要的拷贝
    StyleWithoutName {
        // 初始化字段
    }
} */

/* fn parse_event(data: &str) -> Event {
    // 类似地，解析 event_data 并创建 Event 对象
    Event {
        // 初始化字段
    }
} */
