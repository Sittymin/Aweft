use nom::{
    bytes::complete::tag,
    character::complete::{char, digit1, line_ending, not_line_ending},
    combinator::map_res,
    multi::many1,
    sequence::{separated_pair, terminated, tuple},
    IResult,
};
use saffron_ass::types::{ASSTime, Event, EventMargin, EventType};

fn parse_timestamp(input: &str) -> IResult<&str, (u16, u8, u8, u16)> {
    let (input, (mut hours, _, mut minutes, _, mut seconds, _, milliseconds)) = tuple((
        map_res(digit1, |s: &str| s.parse::<u32>()),
        char(':'),
        map_res(digit1, |s: &str| s.parse::<u32>()),
        char(':'),
        map_res(digit1, |s: &str| s.parse::<u32>()),
        char(','),
        map_res(digit1, |s: &str| s.parse::<u32>()),
    ))(input)?;

    seconds += milliseconds / 1000;
    let milliseconds = milliseconds % 1000;

    minutes += seconds / 60;
    let seconds = (seconds % 60) as u8;

    hours += minutes / 60;
    let minutes = (minutes % 60) as u8;

    Ok((input, (hours as u16, minutes, seconds, milliseconds as u16)))
}

pub fn parse_srt(input: &str) -> IResult<&str, Event> {
    let (input, _index) = terminated(digit1, line_ending)(input)?;
    let (
        input,
        (
            (start_hours, start_minutes, start_seconds, start_milliseconds),
            (end_hours, end_minutes, end_seconds, end_milliseconds),
        ),
    ) = separated_pair(parse_timestamp, tag(" --> "), parse_timestamp)(input)?;

    // remove \n after timestamp
    let (mut rest, _) = line_ending(input)?;
    let mut subtitle_lines = Vec::new();

    loop {
        if rest.is_empty() {
            break;
        }

        match terminated(not_line_ending, line_ending)(rest) {
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
            Event {
                event_type: EventType::Dialogue,
                layer: 0,
                start: ASSTime {
                    hours: start_hours,
                    minutes: start_minutes,
                    seconds: start_seconds,
                    centiseconds: (start_milliseconds / 10) as u8,
                },
                end: ASSTime {
                    hours: end_hours,
                    minutes: end_minutes,
                    seconds: end_seconds,
                    centiseconds: (end_milliseconds / 10) as u8,
                },
                style: "Default".to_string(),
                name: None,
                margin_l: EventMargin(20),
                margin_r: EventMargin(20),
                margin_v: EventMargin(20),
                effect: None,
                raw_text: subtitle_lines.join("\n"),
                parsed_text: None,
            },
        ));
    }

    Err(nom::Err::Error(nom::error::Error::new(
        rest,
        nom::error::ErrorKind::Fail,
    )))
}

pub fn parse_srt_file(content: &str) -> IResult<&str, Vec<Event>> {
    let (rest, events) = many1(parse_srt)(content)?;
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
        assert_eq!(first_event.start.hours, 0);
        assert_eq!(first_event.start.minutes, 0);
        assert_eq!(first_event.start.seconds, 1);
        assert_eq!(first_event.start.centiseconds, 60);
        assert_eq!(first_event.end.centiseconds, 20);
        assert_eq!(
            first_event.raw_text,
            "English subtitle line 1\n中文字幕行 2"
        );
        assert_eq!(second_event.end.centiseconds, 99);
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
        assert_eq!(event.raw_text, "Test subtitle");
        assert_eq!(event.start.hours, 0);
        assert_eq!(event.start.minutes, 0);
        assert_eq!(event.start.seconds, 1);
        assert_eq!(event.start.centiseconds, 60);
    }

    #[test]
    fn test_parse_multiple_lines() {
        let result = parse_srt("1\n00:00:01,600 --> 00:00:04,200\nLine 1\nLine 2\n\n");
        assert!(result.is_ok());
        let (rest, event) = result.unwrap();
        assert!(rest.is_empty());
        assert_eq!(event.raw_text, "Line 1\nLine 2");
    }
}
