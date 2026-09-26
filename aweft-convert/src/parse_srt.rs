use aweft_ass::types::{
    event::{Dialogue, Event, TextSegment},
    time::AssTime,
};
use nom::{
    bytes::complete::tag,
    character::complete::{char, digit1, line_ending, not_line_ending},
    combinator::map_res,
    multi::many1,
    sequence::{separated_pair, terminated},
    IResult, Parser,
};

fn parse_timestamp(input: &str) -> IResult<&str, (u16, u8, u8, u16)> {
    let (input, (mut hours, _, mut minutes, _, mut seconds, _, milliseconds)) = (
        map_res(digit1, |s: &str| s.parse::<u32>()),
        char(':'),
        map_res(digit1, |s: &str| s.parse::<u32>()),
        char(':'),
        map_res(digit1, |s: &str| s.parse::<u32>()),
        char(','),
        map_res(digit1, |s: &str| s.parse::<u32>()),
    )
        .parse(input)?;

    seconds += milliseconds / 1000;
    let milliseconds = milliseconds % 1000;

    minutes += seconds / 60;
    let seconds = (seconds % 60) as u8;

    hours += minutes / 60;
    let minutes = (minutes % 60) as u8;

    Ok((input, (hours as u16, minutes, seconds, milliseconds as u16)))
}

fn parse_srt(input: &str) -> IResult<&str, Event> {
    let (input, _index) = terminated(digit1, line_ending).parse(input)?;
    let (
        input,
        (
            (start_hours, start_minutes, start_seconds, start_milliseconds),
            (end_hours, end_minutes, end_seconds, end_milliseconds),
        ),
    ) = separated_pair(parse_timestamp, tag(" --> "), parse_timestamp).parse(input)?;

    // remove \n after timestamp
    let (mut rest, _) = line_ending.parse(input)?;
    let mut subtitle_lines = Vec::new();

    loop {
        if rest.is_empty() {
            break;
        }

        match terminated(not_line_ending, line_ending).parse(rest) {
            Ok((new_rest, line)) => {
                if line.trim().is_empty() {
                    rest = new_rest;
                    break;
                }
                subtitle_lines.push(line);
                rest = new_rest;
            }
            Err(nom::Err::Error(_)) if !rest.is_empty() => {
                let line = rest.trim();
                if !line.is_empty() {
                    subtitle_lines.push(line);
                }
                rest = "";
                break;
            }
            Err(e) => return Err(e),
        }
    }

    if !subtitle_lines.is_empty() {
        return Ok((
            rest,
            Event::Dialogue(Dialogue {
                layer: 0,
                start: AssTime::from_hms_cs(
                    start_hours as i64,
                    start_minutes as i64,
                    start_seconds as i64,
                    (start_milliseconds / 10) as i64,
                ),
                end: AssTime::from_hms_cs(
                    end_hours as i64,
                    end_minutes as i64,
                    end_seconds as i64,
                    (end_milliseconds / 10) as i64,
                ),
                style: "Default".to_string(),
                name: None,
                margin_l: 20,
                margin_r: 20,
                margin_v: 20,
                effect: None,
                content: vec![TextSegment::Text(subtitle_lines.join("\n"))],
            }),
        ));
    }

    Err(nom::Err::Error(nom::error::Error::new(
        rest,
        nom::error::ErrorKind::Fail,
    )))
}

pub fn parse_srt_file(content: &str) -> IResult<&str, Vec<Event>> {
    let (rest, events) = many1(parse_srt).parse(content)?;
    Ok((rest, events))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_parse_str_file() {
        let test_srt_content: &str = r#"1
00:00:01,600 --> 00:00:04,200
English subtitle line 1
中文字幕行 2

2
00:00:05,900 --> 00:00:07,999
This is a test subtitle

"#;
        let (_, events) = parse_srt_file(test_srt_content).unwrap();
        let first_event = &events[0];
        let second_event = &events[1];

        if let Event::Dialogue(dialogue) = first_event {
            assert_eq!(dialogue.start.as_centiseconds(), 160); // 1.6 seconds = 160 centiseconds
            assert_eq!(dialogue.end.as_centiseconds(), 420); // 4.2 seconds = 420 centiseconds
            if let TextSegment::Text(text) = &dialogue.content[0] {
                assert_eq!(text, "English subtitle line 1\n中文字幕行 2");
            } else {
                panic!("Expected TextSegment::Text");
            }
        } else {
            panic!("Expected Event::Dialogue");
        }

        if let Event::Dialogue(dialogue) = second_event {
            assert_eq!(dialogue.end.as_centiseconds(), 799); // 7.999 seconds = 799.9 centiseconds
        } else {
            panic!("Expected Event::Dialogue");
        }
    }

    #[test]
    fn test_parse_timestamp() {
        let input = "00:00:01,600";
        let result = parse_timestamp(input);
        assert!(result.is_ok());
        let (_, (hours, minutes, seconds, milliseconds)) = result.unwrap();
        assert_eq!(hours, 0);
        assert_eq!(minutes, 0);
        assert_eq!(seconds, 1);
        assert_eq!(milliseconds, 600);
    }

    #[test]
    fn test_parse_single_subtitle() {
        let result = parse_srt("1\n00:00:01,600 --> 00:00:04,200\nTest subtitle\n\n");
        assert!(result.is_ok());
        let (rest, event) = result.unwrap();
        assert!(rest.is_empty());

        if let Event::Dialogue(dialogue) = event {
            if let TextSegment::Text(text) = &dialogue.content[0] {
                assert_eq!(text, "Test subtitle");
            } else {
                panic!("Expected TextSegment::Text");
            }
            assert_eq!(dialogue.start.as_centiseconds(), 160); // 1.6 seconds = 160 centiseconds
        } else {
            panic!("Expected Event::Dialogue");
        }
    }

    #[test]
    fn test_parse_multiple_lines() {
        let result = parse_srt("1\n00:00:01,600 --> 00:00:04,200\nLine 1\nLine 2\n\n");
        assert!(result.is_ok());
        let (rest, event) = result.unwrap();
        assert!(rest.is_empty());

        if let Event::Dialogue(dialogue) = event {
            if let TextSegment::Text(text) = &dialogue.content[0] {
                assert_eq!(text, "Line 1\nLine 2");
            } else {
                panic!("Expected TextSegment::Text");
            }
        } else {
            panic!("Expected Event::Dialogue");
        }
    }
}
