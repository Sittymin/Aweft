use super::types::*;
use core::panic;
use std::collections::HashMap;
use std::fs::File;
use std::io::{self, BufRead, BufReader, BufWriter, Write};
use std::path::PathBuf;
use std::str::FromStr;

impl ASSFile {
    pub fn parse_from_file(path: &PathBuf) -> io::Result<Self> {
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
                        // if empty at all should add a default style
                        parse_styles_line(line, &mut ass_file.styles);
                    }
                    "Events" => {
                        parse_events_line(line, &mut ass_file.events);
                    }
                    _ => {
                        // Ignore other part
                    }
                }
            }
        }
        Ok(ass_file)
    }
    pub fn write_to_file_buffered(
        &self,
        path: &PathBuf,
        events_flush_interval: u32,
    ) -> io::Result<()> {
        let file = File::create(path)?;
        let mut writer = BufWriter::new(file);
        writeln!(writer, "[Script Info]")?;
        writeln!(writer, "{}", self.script_info)?;
        writer.flush()?;

        if self.script_info.script_type == ScriptType::ASS {
            writeln!(writer, "[V4+ Styles]")?;
            writeln!(writer, "Format: Name, Fontname, Fontsize, PrimaryColour, SecondaryColour, OutlineColour, BackColour, Bold, Italic, Underline, StrikeOut, ScaleX, ScaleY, Spacing, Angle, BorderStyle, Outline, Shadow, Alignment, MarginL, MarginR, MarginV, Encoding")?;
            for (key, value) in &self.styles {
                writeln!(writer, "Style: {},{}", key, value)?;
            }
        } else {
            panic!("No script type other than ASS has been implemented")
        }

        // TODO: Event
        // writeln!(writer, "\n[Events]")?;
        // for (index, event) in self.events.iter().enumerate() {
        //     writeln!(writer, "{}", event)?;
        //     if (index + 1) % events_flush_interval == 0 {
        //         writer.flush()?;
        //     }
        // }

        Ok(())
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

        // example: !: Release Date: 2009-08-07
        if key.starts_with("!") {
            script_info.add_or_update_other_fields(key[3..].to_string(), value.to_string())?;
        } else {
            script_info.set_field(key, value)?;
        }
    }
    Ok(())
}

fn parse_styles_line(line: &str, styles: &mut HashMap<String, StyleWithoutName>) {
    let want_data: Vec<&str>  = "Name, Fontname, Fontsize, PrimaryColour, SecondaryColour, OutlineColour, BackColour, Bold, Italic, Underline, StrikeOut, ScaleX, ScaleY, Spacing, Angle, BorderStyle, Outline, Shadow, Alignment, MarginL, MarginR, MarginV, Encoding".split(",").collect();

    if line.starts_with("Format:") {
        // MUST first and MUST be:
        // Format: Name, Fontname, Fontsize, PrimaryColour, SecondaryColour, OutlineColour, BackColour, Bold, Italic, Underline, StrikeOut, ScaleX, ScaleY, Spacing, Angle, BorderStyle, Outline, Shadow, Alignment, MarginL, MarginR, MarginV, Encoding
        // parse function pass this
    } else
    // in this line can't have space
    // https://github.com/libass/libass/wiki/ASS-File-Format-Guide#v4-ass-style
    if let Some(style_data) = line.strip_prefix("Style: ") {
        let style_datas: Vec<&str> = style_data.split(",").collect();
        if style_datas.len() != want_data.len() {
            return ();
        }

        let name = style_datas[0].to_string();
        let style_without_name = StyleWithoutName {
            fontname: style_datas[1].to_string(),
            fontsize: style_datas[2].parse::<f64>().unwrap_or(18.0),
            primary_color: ASSColor::from_str(style_datas[3]).unwrap_or(ASSColor(0xFFFFFF00)),
            secondary_color: ASSColor::from_str(style_datas[4]).unwrap_or(ASSColor(0x00FFFF00)),
            outline_color: ASSColor::from_str(style_datas[5]).unwrap_or(ASSColor(0x00000000)),
            back_color: ASSColor::from_str(style_datas[6]).unwrap_or(ASSColor(0x00000080)),
            bold: BoldType::from_str(style_datas[7]).unwrap_or(BoldType::NonBold),
            italic: StyleBool::from_str(style_datas[8]).unwrap_or(StyleBool::False),
            underline: StyleBool::from_str(style_datas[9]).unwrap_or(StyleBool::False),
            strikeout: StyleBool::from_str(style_datas[10]).unwrap_or(StyleBool::False),
            scale_x: style_datas[11].parse::<f64>().unwrap_or(1.0),
            scale_y: style_datas[12].parse::<f64>().unwrap_or(1.0),
            spacing: style_datas[13].parse::<f64>().unwrap_or(0.0),
            angle: style_datas[14].parse::<f64>().unwrap_or(0.0),
            border_style: BorderStyle::from_str(style_datas[15])
                .unwrap_or(BorderStyle::OutlineAndDropShadow),
            outline: style_datas[16].parse::<f64>().unwrap_or(2.0),
            shadow: style_datas[17].parse::<f64>().unwrap_or(3.0),
            alignment: Alignment::from_str(style_datas[18]).unwrap_or(Alignment::BottomCenter),
            margin_l: style_datas[19].parse::<i32>().unwrap_or(20),
            margin_r: style_datas[20].parse::<i32>().unwrap_or(20),
            margin_v: style_datas[21].parse::<i32>().unwrap_or(20),
            encoding: 1,
        };
        styles.entry(name).or_insert(style_without_name);
    };
}

fn parse_events_line(line: &str, events: &mut Vec<Event>) {
    let want_data: Vec<&str> =
        "Layer, Start, End, Style, Name, MarginL, MarginR, MarginV, Effect, Text"
            .split(",")
            .collect();

    if line.starts_with("Format:") {
        // MUST be
        // Format: Layer, Start, End, Style, Name, MarginL, MarginR, MarginV, Effect, Text
    } else if let Some(dialogue_data) = line.strip_prefix("Dialogue: ") {
        let dialogue_datas: Vec<&str> = dialogue_data.split(",").collect();
        if dialogue_datas.len() < want_data.len() {
            return ();
        }

        events.push(parse_event(EventType::Dialogue, dialogue_datas));
    } else if let Some(comment_data) = line.strip_prefix("Comment: ") {
        let comment_datas: Vec<&str> = comment_data.split(",").collect();
        if comment_datas.len() < want_data.len() {
            return ();
        }

        events.push(parse_event(EventType::Comment, comment_datas));
    }
}

fn parse_event(event_type: EventType, line_datas: Vec<&str>) -> Event {
    Event {
        event_type,
        layer: line_datas[0].parse::<u32>().unwrap_or(0),
        start: ASSTime::from_str(line_datas[1]).unwrap_or(ASSTime::new()),
        end: ASSTime::from_str(line_datas[2]).unwrap_or(ASSTime::new()),
        style: line_datas[3].to_string(),
        name: Some(line_datas[4].trim().to_string()).filter(|s| !s.is_empty()),
        margin_l: EventMargin::from_str(line_datas[5]).unwrap_or(EventMargin(0)),
        margin_r: EventMargin::from_str(line_datas[6]).unwrap_or(EventMargin(0)),
        margin_v: EventMargin::from_str(line_datas[7]).unwrap_or(EventMargin(0)),
        effect: Effect::from_str(line_datas[8]).ok(),
        raw_text: line_datas[9..].join(","),
        parsed_text: None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use io::Write;
    use std::{env, path::PathBuf};

    fn create_test_file(content: &str) -> io::Result<PathBuf> {
        // 获取系统临时目录
        let mut test_path = env::temp_dir();
        // 使用时间戳创建唯一文件名
        let timestamp = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        test_path.push(format!("test_subtitle_{}.ass", timestamp));

        let mut file = File::create(&test_path)?;
        file.write_all(content.as_bytes())?;
        file.flush()?;
        Ok(test_path)
    }

    #[test]
    fn test_parse_script_info_line() {
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

        let result = parse_script_info_line("!Custom: Value", &mut script_info);
        assert!(result.is_ok());
        assert_eq!(
            script_info.other_fields.get("Custom"),
            Some(&"Value".to_string())
        );

        let result = parse_script_info_line("PlayResX: invalid", &mut script_info);
        assert!(result.is_err());
        let result = parse_script_info_line("ScriptType: invalid", &mut script_info);
        assert!(result.is_err());

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
    fn test_parse_styles_line() {
        use std::collections::HashMap;

        let mut styles: HashMap<String, StyleWithoutName> = HashMap::new();

        let input =
            "Format: Name, Fontname, Fontsize, PrimaryColour, SecondaryColour, OutlineColour, \
                 BackColour, Bold, Italic, Underline, StrikeOut, ScaleX, ScaleY, Spacing, Angle, \
                 BorderStyle, Outline, Shadow, Alignment, MarginL, MarginR, MarginV, Encoding";
        parse_styles_line(input, &mut styles);
        assert!(styles.is_empty());

        let input =
            "Style: Default,DejaVu Sans,65,&H007F67D0,&H00187DC1,&H00000000,&H00D4AA86,-1,0,0,0,100,100,0,0,1,0.4,0,7,10,10,10,1";
        parse_styles_line(input, &mut styles);

        assert_eq!(styles.len(), 1);

        let staff_style = styles.get("Default").unwrap();

        let expected_style = StyleWithoutName {
            fontname: "DejaVu Sans".to_string(),
            fontsize: 65.0,
            primary_color: ASSColor(0x007F67D0),
            secondary_color: ASSColor(0x00187DC1),
            outline_color: ASSColor(0x00000000),
            back_color: ASSColor(0x00D4AA86),
            bold: BoldType::Bold,
            italic: StyleBool::False,
            underline: StyleBool::False,
            strikeout: StyleBool::False,
            scale_x: 100.0,
            scale_y: 100.0,
            spacing: 0.0,
            angle: 0.0,
            border_style: BorderStyle::OutlineAndDropShadow,
            outline: 0.4,
            shadow: 0.0,
            alignment: Alignment::TopLeft,
            margin_l: 10,
            margin_r: 10,
            margin_v: 10,
            encoding: 1,
        };

        assert_eq!(staff_style, &expected_style);
    }

    #[test]
    fn test_parse_format_line() {
        let mut events = Vec::new();
        let line =
            "Format: Layer, Start, End, Style, Name, MarginL, MarginR, MarginV, Effect, Text";

        parse_events_line(line, &mut events);

        // Format line should not create any events
        assert_eq!(events.len(), 0);
    }

    #[test]
    fn test_parse_comment_line() {
        let mut events = Vec::new();
        let line = "Comment: 0,0:00:00.00,0:00:06.99,Default,,0,0,0,,Placeholder;";

        parse_events_line(line, &mut events);
        let event = &events[0];

        assert_eq!(event.event_type, EventType::Comment);
    }

    #[test]
    fn test_parse_from_file() -> io::Result<()> {
        let test_content = r#"[Script Info]
; 脚本信息
Title: Test Subtitle
ScriptType: v4.00+
PlayResX: 1920
PlayResY: 1080

[V4+ Styles]
Format: Name, Fontname, Fontsize, PrimaryColour, SecondaryColour, OutlineColour, BackColour, Bold, Italic, Underline, StrikeOut, ScaleX, ScaleY, Spacing, Angle, BorderStyle, Outline, Shadow, Alignment, MarginL, MarginR, MarginV, Encoding
Style: Default,Arial,20,&H00FFFFFF,&H000000FF,&H00000000,&H00000000,0,0,0,0,100,100,0,0,1,2,2,2,10,10,10,1

[Events]
Format: Layer, Start, End, Style, Name, MarginL, MarginR, MarginV, Effect, Text
Dialogue: 0,0:00:01.00,0:00:04.00,Default,,0,0,0,,这是一个测试字幕"#;

        let temp_path = create_test_file(test_content)?;
        println!("Created test file at: {:?}", temp_path);

        let ass_file = ASSFile::parse_from_file(&temp_path)?;

        // 1. Script Info
        {
            let script_info = &ass_file.script_info;
            assert_eq!(script_info.script_type, ScriptType::ASS);
            assert_eq!(script_info.scaled_border_and_shadow, true);
            assert_eq!(script_info.ycbcr_matrix, YCbCrMatrix::Default);
            assert_eq!(script_info.layout_res_x, 1920);
            assert_eq!(script_info.layout_res_y, 1080);
            assert_eq!(script_info.wrap_style, WrapStyle::Smart);
            assert_eq!(script_info.title, Some("Test Subtitle".to_string()));
            assert_eq!(script_info.original_script, None);
            assert_eq!(script_info.original_translation, None);
            assert_eq!(script_info.original_editing, None);
            assert_eq!(script_info.original_timing, None);
            assert_eq!(script_info.script_updated_by, None);
            assert_eq!(script_info.update_details, None);
            assert!(script_info.other_fields.is_empty());
        }

        // 2. Styles
        {
            assert_eq!(ass_file.styles.len(), 1);
            let style = ass_file
                .styles
                .get("Default")
                .expect("Default style should exist");
            assert_eq!(style.fontname, "Arial");
            assert_eq!(style.fontsize, 20.0);
            assert_eq!(style.primary_color, ASSColor(16777215));
            assert_eq!(style.secondary_color, ASSColor(255));
            assert_eq!(style.outline_color, ASSColor(0));
            assert_eq!(style.back_color, ASSColor(0));
            assert_eq!(style.bold, BoldType::NonBold);
            assert_eq!(style.italic, StyleBool::False);
            assert_eq!(style.underline, StyleBool::False);
            assert_eq!(style.strikeout, StyleBool::False);
            assert_eq!(style.scale_x, 100.0);
            assert_eq!(style.scale_y, 100.0);
            assert_eq!(style.spacing, 0.0);
            assert_eq!(style.angle, 0.0);
            assert_eq!(style.border_style, BorderStyle::OutlineAndDropShadow);
            assert_eq!(style.outline, 2.0);
            assert_eq!(style.shadow, 2.0);
            assert_eq!(style.alignment, Alignment::BottomCenter);
            assert_eq!(style.margin_l, 10);
            assert_eq!(style.margin_r, 10);
            assert_eq!(style.margin_v, 10);
            assert_eq!(style.encoding, 1);
        }

        // 3. Events
        {
            assert_eq!(ass_file.events.len(), 1);
            let event = &ass_file.events[0];
            assert_eq!(event.event_type, EventType::Dialogue);
            assert_eq!(event.layer, 0);
            assert_eq!(
                event.start,
                ASSTime {
                    hours: 0,
                    minutes: 0,
                    seconds: 1,
                    centiseconds: 0
                }
            );
            assert_eq!(
                event.end,
                ASSTime {
                    hours: 0,
                    minutes: 0,
                    seconds: 4,
                    centiseconds: 0
                }
            );
            assert_eq!(event.style, "Default");
            assert_eq!(event.name, None);
            assert_eq!(event.margin_l, EventMargin(0));
            assert_eq!(event.margin_r, EventMargin(0));
            assert_eq!(event.margin_v, EventMargin(0));
            assert_eq!(event.effect, None);
            assert_eq!(event.raw_text, "这是一个测试字幕");
            assert_eq!(event.parsed_text, None);
        }

        // 4. Clear
        std::fs::remove_file(temp_path)?;

        Ok(())
    }
}
