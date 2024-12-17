use super::types::*;
use std::collections::HashMap;
use std::fs::File;
use std::io::{self, BufRead, BufReader};
use std::str::FromStr;

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
                    // TODO: impl SSA v4 and ASS2
                    "V4+ Styles" => {
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
        // MUST first and MUST be:
        // Format: Name, Fontname, Fontsize, PrimaryColour, SecondaryColour, OutlineColour, BackColour, Bold, Italic, Underline, StrikeOut, ScaleX, ScaleY, Spacing, Angle, BorderStyle, Outline, Shadow, Alignment, MarginL, MarginR, MarginV, Encoding
        // parse function pass this
    } else
    // in this line can't have space
    // https://github.com/libass/libass/wiki/ASS-File-Format-Guide#v4-ass-style
    if let Some(style_data) = line.strip_prefix("Style: ") {
        let style_datas: Vec<&str> = style_data.split(",").collect();
        if style_datas.len() != 23 {
            return ();
        }

        let name = style_datas[0].to_string();
        let style_without_name = StyleWithoutName {
            fontname: style_datas[2].to_string(),
            fontsize: style_datas[3].parse::<f64>().unwrap_or(18.0),
            primary_color: ASSColor::from_str(style_datas[4]).unwrap_or(ASSColor(0xFFFFFF00)),
            secondary_color: ASSColor::from_str(style_datas[5]).unwrap_or(ASSColor(0x00FFFF00)),
            outline_color: ASSColor::from_str(style_datas[6]).unwrap_or(ASSColor(0x00000000)),
            back_color: ASSColor::from_str(style_datas[7]).unwrap_or(ASSColor(0x00000080)),
            bold: BoldType::from_str(style_datas[8]).unwrap_or(BoldType::NonBold),
            italic: StyleBool::from_str(style_datas[9]).unwrap_or(StyleBool::False),
            underline: StyleBool::from_str(style_datas[10]).unwrap_or(StyleBool::False),
            strikeout: StyleBool::from_str(style_datas[11]).unwrap_or(StyleBool::False),
            scale_x: style_datas[12].parse::<f64>().unwrap_or(1.0),
            scale_y: style_datas[13].parse::<f64>().unwrap_or(1.0),
            spacing: style_datas[14].parse::<f64>().unwrap_or(0.0),
            angle: style_datas[15].parse::<f64>().unwrap_or(0.0),
            border_style: BorderStyle::from_str(style_datas[16])
                .unwrap_or(BorderStyle::OutlineAndDropShadow),
            outline: style_datas[17].parse::<f64>().unwrap_or(2.0),
            shadow: style_datas[18].parse::<f64>().unwrap_or(3.0),
            alignment: Alignment::from_str(style_datas[19]).unwrap_or(Alignment::BottomCenter),
            margin_l: style_datas[20].parse::<i32>().unwrap_or(20),
            margin_r: style_datas[21].parse::<i32>().unwrap_or(20),
            margin_v: style_datas[22].parse::<i32>().unwrap_or(20),
            encoding: 1,
        };
        styles.entry(name).or_insert(style_without_name);
    };
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

    #[test]
    fn parse_styles_line_test() {
        use std::collections::HashMap;

        let mut styles: HashMap<String, StyleWithoutName> = HashMap::new();

        let input =
            "Format: Name, Fontname, Fontsize, PrimaryColour, SecondaryColour, OutlineColour, \
                 BackColour, Bold, Italic, Underline, StrikeOut, ScaleX, ScaleY, Spacing, Angle, \
                 BorderStyle, Outline, Shadow, Alignment, MarginL, MarginR, MarginV, Encoding";
        parse_styles_line(input, &mut styles);
        assert!(styles.is_empty());

        let input =
            "Style: STAFF,汉仪旗黑-75S,55,&H00FFFFFF,&H000000FF,&H00000000,&H00000000,0,0,0,0,\
                 100,100,0,0,1,2,0,8,10,10,10,1";
        parse_styles_line(input, &mut styles);

        assert_eq!(styles.len(), 1);

        let staff_style = styles.get("STAFF").unwrap();

        let expected_style = StyleWithoutName {
            fontname: "汉仪旗黑-75S".to_string(),
            fontsize: 55.0,
            primary_color: ASSColor(0x00FFFFFF),
            secondary_color: ASSColor(0x000000FF),
            outline_color: ASSColor(0x00000000),
            back_color: ASSColor(0x00000000),
            bold: BoldType::NonBold,
            italic: StyleBool::False,
            underline: StyleBool::False,
            strikeout: StyleBool::False,
            scale_x: 100.0,
            scale_y: 100.0,
            spacing: 0.0,
            angle: 0.0,
            border_style: BorderStyle::OutlineAndDropShadow,
            outline: 2.0,
            shadow: 0.0,
            alignment: Alignment::TopCenter,
            margin_l: 10,
            margin_r: 10,
            margin_v: 10,
            encoding: 1,
        };

        assert_eq!(staff_style, &expected_style);
    }
}
