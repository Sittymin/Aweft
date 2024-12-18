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
                        // if empty at all should add a default style
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
    fn test_parse_valid_dialogue() {
        let mut events = Vec::new();
        let line = "Dialogue: 0,0:00:01.00,0:00:02.00,Default,John,0,0,0,,Hello world";

        parse_events_line(line, &mut events);

        assert_eq!(events.len(), 1);
        let event = &events[0];
        assert!(matches!(event.event_type, EventType::Dialogue));
        assert_eq!(event.layer, 0);
        assert_eq!(event.style, "Default");
        assert_eq!(event.name, Some("John".to_string()));
        assert_eq!(event.raw_text, "Hello world");
    }

    #[test]
    fn test_parse_dialogue_empty_name() {
        let mut events = Vec::new();
        let line = "Dialogue: 0,0:00:01.00,0:00:02.00,Default,,0,0,0,,Hello world";

        parse_events_line(line, &mut events);

        assert_eq!(events.len(), 1);
        let event = &events[0];
        assert_eq!(event.name, None);
    }

    #[test]
    fn test_parse_dialogue_with_whitespace_name() {
        let mut events = Vec::new();
        let line = "Dialogue: 0,0:00:01.00,0:00:02.00,Default,   ,0,0,0,,Hello world";

        parse_events_line(line, &mut events);

        assert_eq!(events.len(), 1);
        assert_eq!(events[0].name, None);
    }

    #[test]
    fn test_parse_dialogue_with_commas_in_text() {
        let mut events = Vec::new();
        let line = "Dialogue: 0,0:00:01.00,0:00:02.00,Default,John,0,0,0,,Hello, world, test";

        parse_events_line(line, &mut events);

        assert_eq!(events.len(), 1);
        assert_eq!(events[0].raw_text, "Hello, world, test");
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
    fn test_parse_invalid_dialogue_too_few_fields() {
        let mut events = Vec::new();
        let line = "Dialogue: 0,0:00:01.00,0:00:02.00";

        parse_events_line(line, &mut events);

        assert_eq!(events.len(), 0);
    }

    #[test]
    fn test_parse_dialogue_with_invalid_time() {
        let mut events = Vec::new();
        let line = "Dialogue: 0,invalid,0:00:02.00,Default,John,0,0,0,,Hello world";

        parse_events_line(line, &mut events);

        assert_eq!(events.len(), 1);
        let event = &events[0];
        assert_eq!(event.start, ASSTime::new()); // Should use default time
    }

    #[test]
    fn test_parse_dialogue_with_invalid_margins() {
        let mut events = Vec::new();
        let line =
            "Dialogue: 0,0:00:01.00,0:00:02.00,Default,John,invalid,invalid,invalid,,Hello world";

        parse_events_line(line, &mut events);

        assert_eq!(events.len(), 1);
        let event = &events[0];
        assert_eq!(event.margin_l, EventMargin(0));
        assert_eq!(event.margin_r, EventMargin(0));
        assert_eq!(event.margin_v, EventMargin(0));
    }

    #[test]
    fn test_parse_comment_line() {
        let mut events = Vec::new();
        let line = "Comment: 0,0:00:00.00,0:00:06.99,Default,,0,0,0,,Placeholder;";

        parse_events_line(line, &mut events);
        let event = &events[0];

        assert_eq!(event.event_type, EventType::Comment);
    }
}
