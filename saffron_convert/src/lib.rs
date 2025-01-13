use saffron_ass::types::Event;

#[derive(Debug, PartialEq)]
pub struct Subtitle {
    pub start: String,
    pub end: String,
    pub text: String,
}

// TODO: 使用 nom 重写, 并直接使用 ASS 的 Event
pub fn parse_srt(srt_content: &str) -> Vec<Subtitle> {
    let mut subtitles = Vec::new();
    let mut lines = srt_content.lines().peekable();

    while let Some(line) = lines.next() {
        if line.parse::<usize>().is_ok() {
            // Skip the index line
            if let Some(time_line) = lines.next() {
                if let Some(text_start) = lines.peek() {
                    if !time_line.is_empty() && !text_start.is_empty() {
                        let (start, end) = time_line.split_once(" --> ").unwrap_or(("", ""));
                        let mut text_lines = Vec::new();

                        // Collect all lines until the next empty line or number
                        for text_line in lines.by_ref() {
                            if text_line.is_empty() || text_line.parse::<usize>().is_ok() {
                                break;
                            }
                            text_lines.push(text_line.trim());
                        }

                        let text = text_lines.join(" ");
                        subtitles.push(Subtitle {
                            start: start.to_string(),
                            end: end.to_string(),
                            text,
                        });
                    }
                }
            }
        }
    }

    subtitles
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_parse_srt() {
        let srt_content = r#"
45
00:02:52,184 --> 00:02:53,617
慢慢来

46
00:02:55,000 --> 00:02:57,000
不要着急，一切都会好起来的。
"#;

        let expected_subtitles = vec![
            Subtitle {
                start: "00:02:52,184".to_string(),
                end: "00:02:53,617".to_string(),
                text: "慢慢来".to_string(),
            },
            Subtitle {
                start: "00:02:55,000".to_string(),
                end: "00:02:57,000".to_string(),
                text: "不要着急，一切都会好起来的。".to_string(),
            },
        ];

        let parsed_subtitles = parse_srt(srt_content);
        assert_eq!(parsed_subtitles, expected_subtitles);
    }
}
