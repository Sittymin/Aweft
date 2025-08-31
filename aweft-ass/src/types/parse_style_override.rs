use super::{
    tags::{AnimatedStyleTag, BlurEdgesTypes, KaraokeKind, LibassEncoding, StyleTag},
    *,
};
use aweft_libass::types::{Alignment, AssColorRgb};
use chrono::TimeDelta;
use nom::{
    branch::alt,
    bytes::complete::{is_not, tag, take_until, take_while, take_while1},
    character::complete::{self, char, hex_digit1, not_line_ending, one_of},
    combinator::{map, map_res, opt, recognize, verify},
    error::context,
    multi::{many0, many1, separated_list1},
    number::complete::double,
    sequence::{delimited, preceded, separated_pair, terminated},
    IResult, Parser,
};
use parse_drawing_command::parse_drawing_commands;
use std::str::FromStr;

// TODO: REFECTOR: 小于中等复杂度的解析使用函数式组合子(即逻辑上的一行)，否则使用 let 分段处理
fn parse_xshad(input: &str) -> IResult<&str, StyleTag> {
    let (input, _) = tag(r"\xshad")(input)?;
    let (input, val) = double(input)?;
    Ok((input, StyleTag::XShadow(val)))
}
fn parse_yshad(input: &str) -> IResult<&str, StyleTag> {
    let (input, _) = tag(r"\yshad")(input)?;
    let (input, val) = double(input)?;
    Ok((input, StyleTag::YShadow(val)))
}

fn parse_fax(input: &str) -> IResult<&str, StyleTag> {
    let (input, _) = tag(r"\fax")(input)?;
    let (input, val) = double(input)?;
    Ok((input, StyleTag::FactorX(val)))
}
fn parse_fay(input: &str) -> IResult<&str, StyleTag> {
    let (input, _) = tag(r"\fay")(input)?;
    let (input, val) = double(input)?;
    Ok((input, StyleTag::FactorY(val)))
}

fn parse_blur(input: &str) -> IResult<&str, StyleTag> {
    let (input, _) = tag(r"\blur")(input)?;
    let (input, val) = double(input)?;
    Ok((input, StyleTag::Blur(val)))
}

fn parse_fscx<'a>(input: &'a str) -> IResult<&'a str, StyleTag> {
    let (input, _) = tag(r"\fscx")(input)?;
    let (input, x) = double(input)?;

    Ok((input, StyleTag::FontScaleX(x)))
}
fn parse_fscy<'a>(input: &'a str) -> IResult<&'a str, StyleTag> {
    let (input, _) = tag(r"\fscy")(input)?;
    let (input, y) = double(input)?;

    Ok((input, StyleTag::FontScaleY(y)))
}

// \fsc 应该将字体缩放重置为样式的缩放
// https://github.com/libass/libass/blob/7d8e335b094f00c57fb557b01e93b60a17e63434/libass/ass_parse.c#L455-L456
fn parse_fsc(input: &str) -> IResult<&str, StyleTag> {
    let (input, _) = tag(r"\fsc")(input)?;
    Ok((input, StyleTag::FontScaleReset))
}

fn parse_fsp(input: &str) -> IResult<&str, StyleTag> {
    let (input, _) = tag(r"\fsp")(input)?;
    let (input, val) = double(input)?;
    Ok((input, StyleTag::FontSpacing(val)))
}
fn parse_fs(input: &str) -> IResult<&str, StyleTag> {
    let (input, _) = tag(r"\fs")(input)?;
    let (input, val) = double(input)?;
    // WARN: 如果为负数应该重置为样式的字体大小
    // https://github.com/libass/libass/blob/7d8e335b094f00c57fb557b01e93b60a17e63434/libass/ass_parse.c#L474-L475
    Ok((input, StyleTag::FontSize(val)))
}

fn parse_bord(input: &str) -> IResult<&str, StyleTag> {
    let (input, _) = tag(r"\bord")(input)?;
    let (input, val) = double(input)?;
    Ok((input, StyleTag::Bord(val)))
}
fn parse_xbord(input: &str) -> IResult<&str, StyleTag> {
    let (input, _) = tag(r"\xbord")(input)?;
    let (input, val) = double(input)?;
    Ok((input, StyleTag::XBord(val)))
}
fn parse_ybord(input: &str) -> IResult<&str, StyleTag> {
    let (input, _) = tag(r"\ybord")(input)?;
    let (input, val) = double(input)?;
    Ok((input, StyleTag::YBord(val)))
}

/// 演示解析“\move(...)”标签，如“\move(10,20,30,40,500,1000)”。
fn parse_move(input: &str) -> IResult<&str, StyleTag> {
    let (input, _) = tag(r"\move")(input)?;
    let (input, (x1, y1, x2, y2, t1, t2)) = delimited(
        char('('),
        (
            terminated(double, char(',')),
            terminated(double, char(',')),
            terminated(double, char(',')),
            terminated(double, opt(char(','))),
            opt(complete::i64),
            opt(preceded(char(','), complete::i64)),
        ),
        char(')'),
    )
    .parse(input)?;

    Ok((
        input,
        StyleTag::Move {
            start: PointF64 { x: x1, y: y1 },
            end: PointF64 { x: x2, y: y2 },
            start_time: t1.map(TimeDelta::milliseconds),
            end_time: t2.map(TimeDelta::milliseconds),
        },
    ))
}

fn parse_frx(input: &str) -> IResult<&str, StyleTag> {
    preceded(tag(r"\frx"), map(double, StyleTag::FontRotationX)).parse(input)
}
fn parse_fry(input: &str) -> IResult<&str, StyleTag> {
    preceded(tag(r"\fry"), map(double, StyleTag::FontRotationY)).parse(input)
}
fn parse_frz(input: &str) -> IResult<&str, StyleTag> {
    preceded(
        alt((tag(r"\frz"), tag(r"\fr"))),
        map(double, StyleTag::FontRotationZ),
    )
    .parse(input)
}

fn parse_fn(input: &str) -> IResult<&str, StyleTag> {
    preceded(
        tag(r"\fn"),
        map(is_not(r"\}"), |font_name: &str| {
            StyleTag::FontName(font_name.to_string())
        }),
    )
    .parse(input)
}

fn parse_all_alpha(input: &str) -> IResult<&str, StyleTag> {
    preceded(
        tag(r"\alpha"),
        map(
            map_res(
                context(
                    "alpha hex code must be 2 digits",
                    verify(hex_digit1, |s: &str| s.len() == 2),
                ),
                |s: &str| u8::from_str_radix(s, 16).map(|val| 255u8.saturating_sub(val)),
            ),
            // 如果一切顺利，将 u8 包装成 StyleTag
            StyleTag::AllAlpha,
        ),
    )
    .parse(input)
}
fn parse_1a(input: &str) -> IResult<&str, StyleTag> {
    preceded(
        tag(r"\1a"),
        map(
            map_res(
                context(
                    "1a hex code must be 2 digits",
                    verify(hex_digit1, |s: &str| s.len() == 2),
                ),
                |s: &str| u8::from_str_radix(s, 16).map(|val| 255u8.saturating_sub(val)),
            ),
            StyleTag::PrimaryAlpha,
        ),
    )
    .parse(input)
}
fn parse_2a(input: &str) -> IResult<&str, StyleTag> {
    preceded(
        tag(r"\2a"),
        map(
            map_res(
                context(
                    "2a hex code must be 2 digits",
                    verify(hex_digit1, |s: &str| s.len() == 2),
                ),
                |s: &str| u8::from_str_radix(s, 16).map(|val| 255u8.saturating_sub(val)),
            ),
            StyleTag::SecondaryAlpha,
        ),
    )
    .parse(input)
}
fn parse_3a(input: &str) -> IResult<&str, StyleTag> {
    preceded(
        tag(r"\3a"),
        map(
            map_res(
                context(
                    "3a hex code must be 2 digits",
                    verify(hex_digit1, |s: &str| s.len() == 2),
                ),
                |s: &str| u8::from_str_radix(s, 16).map(|val| 255u8.saturating_sub(val)),
            ),
            StyleTag::OutlineAlpha,
        ),
    )
    .parse(input)
}
fn parse_4a(input: &str) -> IResult<&str, StyleTag> {
    preceded(
        tag(r"\4a"),
        map(
            map_res(
                context(
                    "4a hex code must be 2 digits",
                    verify(hex_digit1, |s: &str| s.len() == 2),
                ),
                |s: &str| u8::from_str_radix(s, 16).map(|val| 255u8.saturating_sub(val)),
            ),
            StyleTag::BackAlpha,
        ),
    )
    .parse(input)
}

fn parse_an(input: &str) -> IResult<&str, StyleTag> {
    let (input, _) = tag(r"\an")(input)?;
    // NOTE: recognize 是为了将 char 转换为 &str
    let (input, val) = recognize(one_of("123456789")).parse(input)?;
    Ok((
        input,
        StyleTag::Alignment(Alignment::from_str(val).unwrap_or(Alignment::BottomCenter)),
    ))
}
fn parse_a(input: &str) -> IResult<&str, StyleTag> {
    let (input, _) = tag(r"\a")(input)?;
    let (input, _) = alt((
        // WARN: 必须先尝试匹配更长的字符串！
        tag("11"),
        tag("10"),
        recognize(one_of("123456789")),
    ))
    .parse(input)?;
    Ok((
        input,
        // NOTE: if use \a with \an will override \an
        StyleTag::Alignment(Alignment::BottomCenter),
    ))
}

fn parse_pos(input: &str) -> IResult<&str, StyleTag> {
    let (input, _) = tag(r"\pos")(input)?;
    let (input, (x, y)) = delimited(
        char('('),
        separated_pair(double, char(','), double),
        char(')'),
    )
    .parse(input)?;
    Ok((input, StyleTag::Position(PointF64 { x, y })))
}

fn parse_fade(input: &str) -> IResult<&str, StyleTag> {
    let (input, tag) = alt((tag(r"\fade"), tag(r"\fad"))).parse(input)?;
    if tag == r"\fad" {
        let (input, (time1, time2)) = delimited(
            char('('),
            separated_pair(complete::i64, char(','), complete::i64),
            char(')'),
        )
        .parse(input)?;
        Ok((
            input,
            StyleTag::SimpleFade {
                in_time: TimeDelta::milliseconds(time1),
                out_time: TimeDelta::milliseconds(time2),
            },
        ))
    } else if tag == r"\fade" {
        let (input, (a1, a2, a3, t1, t2, t3, t4)) = delimited(
            char('('),
            (
                complete::u8,
                preceded(char(','), complete::u8),
                preceded(char(','), complete::u8),
                preceded(char(','), complete::i64),
                preceded(char(','), complete::i64),
                preceded(char(','), complete::i64),
                preceded(char(','), complete::i64),
            ),
            char(')'),
        )
        .parse(input)?;
        Ok((
            input,
            StyleTag::Fade {
                start_opacity: 255u8.saturating_sub(a1),
                mid_opacity: 255u8.saturating_sub(a2),
                end_opacity: 255u8.saturating_sub(a3),
                in_start_time: TimeDelta::milliseconds(t1),
                in_end_time: TimeDelta::milliseconds(t2),
                out_start_time: TimeDelta::milliseconds(t3),
                out_end_time: TimeDelta::milliseconds(t4),
            },
        ))
    } else {
        Err(nom::Err::Error(nom::error::Error::new(
            input,
            nom::error::ErrorKind::Tag,
        )))
    }
}

fn parse_org(input: &str) -> IResult<&str, StyleTag> {
    let (input, _) = tag(r"\org")(input)?;
    let (input, (x, y)) = delimited(
        char('('),
        separated_pair(double, char(','), double),
        char(')'),
    )
    .parse(input)?;
    Ok((input, StyleTag::Origin(PointF64 { x, y })))
}

fn parse_t(input: &str) -> IResult<&str, StyleTag> {
    let (input, _) = tag(r"\t")(input)?;
    let (_, input) = delimited(char('('), is_not(")"), char(')')).parse(input)?;
    alt((
        // 格式4: (t1, t2, accel, tags)
        map(
            (
                double,
                preceded(char(','), double),
                preceded(char(','), double),
                preceded(char(','), many1(parse_animated_style_tag)),
            ),
            |(t1, t2, acc, tags)| StyleTag::Transform {
                start_time: Some(TimeDelta::microseconds((t1 * 1_000_000.0) as i64)),
                end_time: Some(TimeDelta::microseconds((t2 * 1_000_000.0) as i64)),
                acceleration: Some(acc),
                tags,
            },
        ),
        // 格式3: (t1, t2, tags)
        map(
            (
                double,
                preceded(char(','), double),
                preceded(char(','), many1(parse_animated_style_tag)),
            ),
            |(t1, t2, tags)| StyleTag::Transform {
                start_time: Some(TimeDelta::microseconds((t1 * 1_000_000.0) as i64)),
                end_time: Some(TimeDelta::microseconds((t2 * 1_000_000.0) as i64)),
                acceleration: None,
                tags,
            },
        ),
        // 格式2: (accel, tags)
        map(
            (double, preceded(char(','), many1(parse_animated_style_tag))),
            |(acc, tags)| StyleTag::Transform {
                start_time: None,
                end_time: None,
                acceleration: Some(acc),
                tags,
            },
        ),
        // 格式1: (tags) - 最後匹配
        map(many1(parse_animated_style_tag), |tags| {
            StyleTag::Transform {
                start_time: None,
                end_time: None,
                acceleration: None,
                tags,
            }
        }),
    ))
    .parse(input)
}

fn parse_clip(input: &str) -> IResult<&str, StyleTag> {
    let (input, tag) = alt((tag(r"\clip"), tag(r"\iclip"))).parse(input)?;
    let is_display = tag == r"\clip";
    let (input, params) = delimited(
        char('('),
        separated_list1(char(','), not_line_ending),
        char(')'),
    )
    .parse(input)?;
    if params.len() == 4 {
        Ok((
            input,
            StyleTag::ClipRectangle {
                p1: PointI32 {
                    x: params[0].parse::<i32>().unwrap_or(0),
                    y: params[1].parse::<i32>().unwrap_or(0),
                },
                p2: PointI32 {
                    x: params[2].parse::<i32>().unwrap_or(0),
                    y: params[3].parse::<i32>().unwrap_or(0),
                },
                is_display,
            },
        ))
    } else {
        let mut scale = 1;
        let rest;
        let drawing;
        if params.len() == 2 {
            scale = params[0].parse::<u32>().unwrap_or(1);
            (rest, drawing) = parse_drawing_commands(params[1])?;
            debug_assert!(
                rest.is_empty(),
                "Parsing error: Drawing commands weren't fully parsed. Remaining input: '{}'",
                rest
            );
            Ok((
                input,
                StyleTag::ClipDrawing {
                    scale,
                    drawing,
                    is_display,
                },
            ))
        } else {
            (rest, drawing) = parse_drawing_commands(params[0])?;
            debug_assert!(
                rest.is_empty(),
                "Parsing error: Drawing commands weren't fully parsed. Remaining input: '{}'",
                rest
            );
            Ok((
                input,
                StyleTag::ClipDrawing {
                    scale,
                    drawing,
                    is_display,
                },
            ))
        }
    }
}

fn parse_c(input: &str) -> IResult<&str, StyleTag> {
    preceded(
        alt((tag(r"\c"), tag(r"\1c"))),
        map(
            map_res(
                context(
                    "c hex code must be 6 digits",
                    verify(hex_digit1, |s: &str| s.len() == 6),
                ),
                |s: &str| u32::from_str_radix(s, 16).map(AssColorRgb::from_ass_override_u32),
            ),
            StyleTag::PrimaryColor,
        ),
    )
    .parse(input)
}
fn parse_2c(input: &str) -> IResult<&str, StyleTag> {
    preceded(
        tag(r"\2c"),
        map(
            map_res(
                context(
                    "2c hex code must be 6 digits",
                    verify(hex_digit1, |s: &str| s.len() == 6),
                ),
                |s: &str| u32::from_str_radix(s, 16).map(AssColorRgb::from_ass_override_u32),
            ),
            StyleTag::SecondaryColor,
        ),
    )
    .parse(input)
}

fn parse_3c(input: &str) -> IResult<&str, StyleTag> {
    preceded(
        tag(r"\3c"),
        map(
            map_res(
                context(
                    "3c hex code must be 6 digits",
                    verify(hex_digit1, |s: &str| s.len() == 6),
                ),
                |s: &str| u32::from_str_radix(s, 16).map(AssColorRgb::from_ass_override_u32),
            ),
            StyleTag::OutlineColor,
        ),
    )
    .parse(input)
}

fn parse_4c(input: &str) -> IResult<&str, StyleTag> {
    preceded(
        tag(r"\4c"),
        map(
            map_res(
                context(
                    "4c hex code must be 6 digits",
                    verify(hex_digit1, |s: &str| s.len() == 6),
                ),
                |s: &str| u32::from_str_radix(s, 16).map(AssColorRgb::from_ass_override_u32),
            ),
            StyleTag::BackColor,
        ),
    )
    .parse(input)
}

fn parse_r(input: &str) -> IResult<&str, StyleTag> {
    let (input, _) = tag(r"\r")(input)?;
    if input.starts_with(r"\") {
        Ok((input, StyleTag::Reset(None)))
    } else {
        let (input, style_name) = take_while1(|c: char| c != '\\' && c != '}')(input)?;
        Ok((input, StyleTag::Reset(Some(style_name.to_string()))))
    }
}

fn parse_be(input: &str) -> IResult<&str, StyleTag> {
    let (input, _) = tag(r"\be")(input)?;
    let (input, val) = complete::u32(input)?;
    match val {
        0 => Ok((input, StyleTag::BlurEdges(BlurEdgesTypes::NonBlurEdges))),
        1 => Ok((input, StyleTag::BlurEdges(BlurEdgesTypes::BlurEdges))),
        _ => Ok((input, StyleTag::BlurEdges(BlurEdgesTypes::Weight(val)))),
    }
}

fn parse_b(input: &str) -> IResult<&str, StyleTag> {
    let (input, _) = tag(r"\b")(input)?;
    let (input, val) = complete::u32(input)?;
    match val {
        0 => Ok((input, StyleTag::Bold(BoldType::NonBold))),
        1 => Ok((input, StyleTag::Bold(BoldType::Bold))),
        _ => Ok((input, StyleTag::Bold(BoldType::Weight(val)))),
    }
}

fn parse_i(input: &str) -> IResult<&str, StyleTag> {
    let (input, _) = tag(r"\i")(input)?;
    let (input, val) = complete::u32(input)?;
    match val {
        0 => Ok((input, StyleTag::Italic(false))),
        1 => Ok((input, StyleTag::Italic(true))),
        _ => Err(nom::Err::Error(nom::error::Error::new(
            input,
            nom::error::ErrorKind::Verify,
        ))),
    }
}

// TODO: v4++
// fn parse_kt(input: &str) -> IResult<&str, StyleTag>

fn parse_kf(input: &str) -> IResult<&str, StyleTag> {
    let (input, _) = alt((tag(r"\kf"), tag(r"\K"))).parse(input)?;
    let (input, centiseconds) = double(input)?;
    let milliseconds = (centiseconds * 10.0) as i64;
    Ok((
        input,
        StyleTag::Karaoke {
            duration: TimeDelta::milliseconds(milliseconds),
            kind: KaraokeKind::KaraokeKf,
        },
    ))
}
fn parse_ko(input: &str) -> IResult<&str, StyleTag> {
    let (input, _) = tag(r"\ko")(input)?;
    let (input, centiseconds) = double(input)?;
    let milliseconds = (centiseconds * 10.0) as i64;
    Ok((
        input,
        StyleTag::Karaoke {
            duration: TimeDelta::milliseconds(milliseconds),
            kind: KaraokeKind::KaraokeKo,
        },
    ))
}
fn parse_k(input: &str) -> IResult<&str, StyleTag> {
    let (input, _) = tag(r"\k")(input)?;
    let (input, centiseconds) = double(input)?;
    let milliseconds = (centiseconds * 10.0) as i64;
    Ok((
        input,
        StyleTag::Karaoke {
            duration: TimeDelta::milliseconds(milliseconds),
            kind: KaraokeKind::Karaoke,
        },
    ))
}

fn parse_shad(input: &str) -> IResult<&str, StyleTag> {
    let (input, _) = tag(r"\shad")(input)?;
    let (input, depth) = double(input)?;
    Ok((input, StyleTag::Shadow(depth)))
}

fn parse_s(input: &str) -> IResult<&str, StyleTag> {
    let (input, _) = tag(r"\s")(input)?;
    let (input, is_enable) = complete::u32(input)?;
    match is_enable {
        0 => Ok((input, StyleTag::Strikeout(false))),
        1 => Ok((input, StyleTag::Strikeout(true))),
        _ => Err(nom::Err::Error(nom::error::Error::new(
            input,
            nom::error::ErrorKind::Verify,
        ))),
    }
}

fn parse_u(input: &str) -> IResult<&str, StyleTag> {
    let (input, _) = tag(r"\u")(input)?;
    let (input, is_enable) = complete::u32(input)?;
    match is_enable {
        0 => Ok((input, StyleTag::Underline(false))),
        1 => Ok((input, StyleTag::Underline(true))),
        _ => Err(nom::Err::Error(nom::error::Error::new(
            input,
            nom::error::ErrorKind::Verify,
        ))),
    }
}

fn parse_pbo(input: &str) -> IResult<&str, StyleTag> {
    let (input, _) = tag(r"\pbo")(input)?;
    let (input, val) = double(input)?;
    Ok((input, StyleTag::DrawingBaselineOffset(val)))
}

// https://github.com/libass/libass/blob/7d8e335b094f00c57fb557b01e93b60a17e63434/libass/ass_parse.c#L901
// https://aegisub.org/zh-cn/docs/latest/ass_tags/#p01---toggle-drawing-mode
pub fn parse_p(input: &str) -> IResult<&str, StyleTag> {
    let (input, _) = tag(r"\p")(input)?;
    let (input, scale) = complete::u32(input)?;
    Ok((input, StyleTag::DrawingScale(scale)))
}

fn parse_q(input: &str) -> IResult<&str, StyleTag> {
    let (input, _) = tag(r"\q")(input)?;
    let (input, val) = complete::u32(input)?;
    Ok((
        input,
        StyleTag::WrapStyle(WrapStyle::try_from(val).unwrap_or(WrapStyle::Smart)),
    ))
}

// 在 libass 当中支持的编码有限
fn parse_fe(input: &str) -> IResult<&str, StyleTag> {
    let (input, _) = tag(r"\fe")(input)?;
    let (input, encoding) = map_res(
        complete::i32,
        |id: i32| LibassEncoding::from_codepage(id).ok_or(()), // 2. 对结果进行转换
    )
    .parse(input)?;
    Ok((input, StyleTag::Encoding(encoding)))
}

fn consume_tag(input: &str) -> IResult<&str, StyleTag> {
    // 确保以 \ 开头
    let (input, _) = tag(r"\")(input)?;
    // 消耗无效标签
    let (remaining, consumed) = take_while(|c| c != '\\' && c != '}')(input)?;
    eprintln!("Unknown or invalid tag consumed: \\{}", consumed);
    Ok((remaining, StyleTag::Unknown))
}

fn parse_override_group(input: &str) -> IResult<&str, StyleTag> {
    alt((
        // 可以 animate 的标签属于 override 子集
        parse_animated_style_tag_font_group,
        parse_animated_style_tag_geometry_group,
        parse_animated_style_tag_other_effects_group,
        parse_fsc,
        parse_move,
        parse_fn,
        parse_an,
        parse_a,
        parse_pos,
        parse_fade,
        parse_org,
        parse_t,
        parse_r,
        parse_b,
        parse_i,
    ))
    .parse(input)
}
fn parse_style_tag(input: &str) -> IResult<&str, StyleTag> {
    alt((
        parse_override_group,
        // parse_kt,
        parse_kf,
        parse_ko,
        parse_k,
        parse_s,
        parse_u,
        parse_pbo,
        parse_p,
        parse_q,
        parse_fe,
        consume_tag,
    ))
    .parse(input)
}
fn parse_animated_style_tag_font_group(input: &str) -> IResult<&str, StyleTag> {
    alt((
        parse_fs,
        parse_fsp,
        parse_c,
        parse_2c,
        parse_3c,
        parse_4c,
        parse_all_alpha,
        parse_1a,
        parse_2a,
        parse_3a,
        parse_4a,
    ))
    .parse(input)
}

fn parse_animated_style_tag_geometry_group(input: &str) -> IResult<&str, StyleTag> {
    alt((
        parse_fscx, parse_fscy, parse_frx, parse_fry, parse_frz, parse_fax, parse_fay,
    ))
    .parse(input)
}

fn parse_animated_style_tag_other_effects_group(input: &str) -> IResult<&str, StyleTag> {
    alt((
        parse_bord,
        parse_xbord,
        parse_ybord,
        parse_shad,
        parse_xshad,
        parse_yshad,
        parse_clip,
        parse_be,
        parse_blur,
    ))
    .parse(input)
}

fn parse_animated_style_tag(input: &str) -> IResult<&str, AnimatedStyleTag> {
    // WARN: 这里只放入那些 `TryFrom<StyleTag> for AnimatedStyleTag` 会成功的解析器
    let (input, style_tag) = alt((
        parse_animated_style_tag_font_group,
        parse_animated_style_tag_geometry_group,
        parse_animated_style_tag_other_effects_group,
    ))
    .parse(input)?;

    AnimatedStyleTag::try_from(style_tag)
        .map(|animated_tag| (input, animated_tag))
        .map_err(|_| nom::Err::Error(nom::error::Error::new(input, nom::error::ErrorKind::Verify)))
}

// 解析完整的覆盖语句块（即花括号中的内容），如“{\b1\move(10,20,30,40)\c&HFF00FF&}”。
pub fn parse_multiple_overrides(input: &str) -> IResult<&str, Vec<StyleTag>> {
    let (output, style_overrides) =
        delimited(tag(r"{"), take_until("}"), tag(r"}")).parse(input)?;
    let (_, overrides) = many0(parse_style_tag).parse(style_overrides)?;

    let mut unique_overrides: Vec<StyleTag> = Vec::new();
    for override_item in overrides {
        if !matches!(override_item, StyleTag::Unknown) {
            unique_overrides.push(override_item);
        }
    }

    Ok((output, unique_overrides))
}
