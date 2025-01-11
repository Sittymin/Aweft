use std::{
    io::Error,
    path::{Path, PathBuf},
};

use crate::types::*;

struct FileWithOffset<'a> {
    file_path: &'a Path,
    offset_hours: i32,
    offset_minutes: i32,
    offset_seconds: i32,
    offset_centiseconds: i32,
}
impl<'a> FileWithOffset<'a> {
    fn new(
        file_path: &'a Path,
        offset_hours: i32,
        offset_minutes: i32,
        offset_seconds: i32,
        offset_centiseconds: i32,
    ) -> Self {
        FileWithOffset {
            file_path,
            offset_hours,
            offset_minutes,
            offset_seconds,
            offset_centiseconds,
        }
    }
}

fn merge_ass_files_with_offsets(
    files: Vec<FileWithOffset>,
    mut output: PathBuf,
) -> Result<ASSFile, Error> {
    if files.is_empty() {
        return Err(Error::new(
            std::io::ErrorKind::InvalidInput,
            "No files provided for merging.",
        ));
    }
    if output.is_dir() {
        output.push("merged.ass");
    }

    let first_file = &files[0];
    let mut first_ass = ASSFile::parse_from_file(&PathBuf::from(&first_file.file_path))?;

    for event in &mut first_ass.events {
        event.adjust_time(
            first_file.offset_hours,
            first_file.offset_minutes,
            first_file.offset_seconds,
            first_file.offset_centiseconds,
            TimeAdjustTarget::Both,
        );
    }

    // TODO: 实现可以指定主要使用 ScriptInfo 的文件
    let mut merged_ass = ASSFile::from(&first_ass);

    for file in &files[1..] {
        let path = PathBuf::from(&file.file_path);
        let mut ass_file = ASSFile::parse_from_file(&path)?;

        merged_ass.merge_styles(&ass_file);

        for event in &mut ass_file.events {
            event.adjust_time(
                file.offset_hours,
                file.offset_minutes,
                file.offset_seconds,
                file.offset_centiseconds,
                TimeAdjustTarget::Both,
            );
        }
        merged_ass.events.extend(ass_file.events);
    }
    Ok(merged_ass)
}

#[cfg(test)]
mod tests {
    use std::{collections::HashMap, fs, str::FromStr};

    use super::*;
    use crate::utils::create_test_file;

    #[test]
    fn test_merge_ass_files_with_offsets() -> std::io::Result<()> {
        let test_content_1 = r#"[Script Info]
Title: Test Subtitle
ScriptType: v4.00+
PlayResX: 1920
PlayResY: 1080

[V4+ Styles]
Format: Name, Fontname, Fontsize, PrimaryColour, SecondaryColour, OutlineColour, BackColour, Bold, Italic, Underline, StrikeOut, ScaleX, ScaleY, Spacing, Angle, BorderStyle, Outline, Shadow, Alignment, MarginL, MarginR, MarginV, Encoding
Style: Default,Arial,20,&H00FFFFFF,&H000000FF,&H00000000,&H00000000,0,0,0,0,100,100,0,0,1,2,2,2,10,10,10,1
Style: Test1,Arial,20,&H00FFFFFF,&H000000FF,&H00000000,&H00000000,0,0,0,0,100,100,0,0,1,2,2,2,10,10,10,1

[Events]
Format: Layer, Start, End, Style, Name, MarginL, MarginR, MarginV, Effect, Text
Dialogue: 0,0:00:01.00,0:00:04.00,Default,,0,0,0,,这是一个测试字幕"#;
        let test_content_2 = r#"[Script Info]
Title: Test Subtitle
ScriptType: v4.00+
PlayResX: 1920
PlayResY: 1080

[V4+ Styles]
Format: Name, Fontname, Fontsize, PrimaryColour, SecondaryColour, OutlineColour, BackColour, Bold, Italic, Underline, StrikeOut, ScaleX, ScaleY, Spacing, Angle, BorderStyle, Outline, Shadow, Alignment, MarginL, MarginR, MarginV, Encoding
Style: Default,Arial,20,&H00FFFFFF,&H000000FF,&H00000000,&H00000000,0,0,0,0,100,100,0,0,1,2,2,2,10,10,10,1
Style: Test2,Arial,20,&H00FFFFFF,&H000000FF,&H00000000,&H00000000,0,0,0,0,100,100,0,0,1,2,2,2,10,10,10,1

[Events]
Format: Layer, Start, End, Style, Name, MarginL, MarginR, MarginV, Effect, Text
Dialogue: 0,0:00:01.00,0:00:04.00,Default,,0,0,0,,这是第二个测试字幕"#;
        let except_ass_file = ASSFile {
            script_info: ScriptInfo {
                script_type: ScriptType::ASS,
                scaled_border_and_shadow: true,
                ycbcr_matrix: YCbCrMatrix::Default,
                layout_res_x: 1920,
                layout_res_y: 1080,
                wrap_style: WrapStyle::Smart,
                title: Some("Test Subtitle".to_string()),
                original_script: None,
                original_translation: None,
                original_editing: None,
                original_timing: None,
                script_updated_by: None,
                update_details: None,
                other_fields: HashMap::new(),
            },
            styles: HashMap::from([
                (
                    "Default".to_string(),
                    StyleWithoutName {
                        fontname: "Arial".to_string(),
                        fontsize: 20.0,
                        primary_color: ASSColor::from_str("&H00FFFFFF").unwrap(),
                        secondary_color: ASSColor::from_str("&H000000FF").unwrap(),

                        outline_color: ASSColor::from_str("&H00000000").unwrap(),
                        back_color: ASSColor::from_str("&H00000000").unwrap(),
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
                        shadow: 2.0,
                        alignment: Alignment::BottomCenter,
                        margin_l: 10,
                        margin_r: 10,
                        margin_v: 10,
                        encoding: 1,
                    },
                ),
                (
                    "Test1".to_string(),
                    StyleWithoutName {
                        fontname: "Arial".to_string(),
                        fontsize: 20.0,
                        primary_color: ASSColor::from_str("&H00FFFFFF").unwrap(),
                        secondary_color: ASSColor::from_str("&H000000FF").unwrap(),

                        outline_color: ASSColor::from_str("&H00000000").unwrap(),
                        back_color: ASSColor::from_str("&H00000000").unwrap(),
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
                        shadow: 2.0,
                        alignment: Alignment::BottomCenter,
                        margin_l: 10,
                        margin_r: 10,
                        margin_v: 10,
                        encoding: 1,
                    },
                ),
                (
                    "Test2".to_string(),
                    StyleWithoutName {
                        fontname: "Arial".to_string(),
                        fontsize: 20.0,
                        primary_color: ASSColor::from_str("&H00FFFFFF").unwrap(),
                        secondary_color: ASSColor::from_str("&H000000FF").unwrap(),

                        outline_color: ASSColor::from_str("&H00000000").unwrap(),
                        back_color: ASSColor::from_str("&H00000000").unwrap(),
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
                        shadow: 2.0,
                        alignment: Alignment::BottomCenter,
                        margin_l: 10,
                        margin_r: 10,
                        margin_v: 10,
                        encoding: 1,
                    },
                ),
            ]),
            events: vec![
                Event {
                    event_type: EventType::Dialogue,
                    layer: 0,
                    start: ASSTime {
                        hours: 0,
                        minutes: 0,
                        seconds: 1,
                        centiseconds: 0,
                    },
                    end: ASSTime {
                        hours: 0,
                        minutes: 0,
                        seconds: 4,
                        centiseconds: 0,
                    },
                    style: "Default".to_string(),
                    name: None,
                    margin_l: EventMargin(0),
                    margin_r: EventMargin(0),
                    margin_v: EventMargin(0),
                    effect: None,
                    raw_text: "这是一个测试字幕".to_string(),
                    parsed_text: None,
                },
                Event {
                    event_type: EventType::Dialogue,
                    layer: 0,
                    start: ASSTime {
                        hours: 0,
                        minutes: 0,
                        seconds: 5,
                        centiseconds: 0,
                    },
                    end: ASSTime {
                        hours: 0,
                        minutes: 0,
                        seconds: 8,
                        centiseconds: 0,
                    },
                    style: "Default".to_string(),
                    name: None,
                    margin_l: EventMargin(0),
                    margin_r: EventMargin(0),
                    margin_v: EventMargin(0),
                    effect: None,
                    raw_text: "这是第二个测试字幕".to_string(),
                    parsed_text: None,
                },
            ],
        };

        let temp_path_1 = create_test_file(test_content_1)?;
        let temp_path_2 = create_test_file(test_content_2)?;
        let temp_output_path = create_test_file("")?;
        let file_with_offset_1 = FileWithOffset::new(temp_path_1.as_path(), 0, 0, 0, 0);
        let file_with_offset_2 = FileWithOffset::new(temp_path_2.as_path(), 0, 0, 4, 0);
        let output_ass_file = merge_ass_files_with_offsets(
            vec![file_with_offset_1, file_with_offset_2],
            temp_output_path,
        )?;

        assert_eq!(output_ass_file.script_info, except_ass_file.script_info);
        assert_eq!(output_ass_file.styles, except_ass_file.styles);
        assert_eq!(output_ass_file.events, except_ass_file.events);

        Ok(())
    }
}
