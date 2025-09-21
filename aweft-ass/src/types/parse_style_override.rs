use super::{
    tags::{AnimatedStyleTag, BlurEdgesTypes, KaraokeKind, LibassEncoding, StyleTag},
    *,
};
use aweft_libass::types::{Alignment, AssColorRgb};
use chrono::TimeDelta;
use log::warn;
use nom::{
    branch::alt,
    bytes::complete::{is_not, tag, take_until, take_while, take_while1},
    character::complete::{self, char, hex_digit1, multispace0, one_of},
    combinator::{map, map_res, opt, recognize, verify},
    error::{context, ParseError},
    multi::{many0, many1},
    number::complete::double,
    sequence::{delimited, preceded, separated_pair, terminated},
    IResult, Parser,
};
use parse_drawing_command::parse_drawing_commands;
use std::str::FromStr;

// TODO: REFECTOR: 小于中等复杂度的解析使用函数式组合子(即逻辑上的一行)，否则使用 let 分段处理
fn parse_xshad(input: &str) -> IResult<&str, StyleTag> {
    preceded(tag(r"\xshad"), map(double, StyleTag::XShadow)).parse(input)
}
fn parse_yshad(input: &str) -> IResult<&str, StyleTag> {
    preceded(tag(r"\yshad"), map(double, StyleTag::YShadow)).parse(input)
}

fn parse_shad(input: &str) -> IResult<&str, StyleTag> {
    let (input, depth) = preceded(tag(r"\shad"), double).parse(input)?;
    // https://github.com/libass/libass/blob/534a5f8299c5ab3c2782856fcb843bfea47b7afc/libass/ass_parse.c#L875-L877
    Ok((input, StyleTag::Shadow(depth.max(0.0))))
}

fn parse_fax(input: &str) -> IResult<&str, StyleTag> {
    preceded(tag(r"\fax"), map(double, StyleTag::FactorX)).parse(input)
}
fn parse_fay(input: &str) -> IResult<&str, StyleTag> {
    preceded(tag(r"\fay"), map(double, StyleTag::FactorY)).parse(input)
}

fn parse_blur(input: &str) -> IResult<&str, StyleTag> {
    let (input, val) = preceded(tag(r"\blur"), double).parse(input)?;
    Ok((input, StyleTag::Blur(val.max(0.0).min(100.0))))
}

fn parse_fscx<'a>(input: &'a str) -> IResult<&'a str, StyleTag> {
    let (input, x) = preceded(tag(r"\fscx"), double).parse(input)?;
    Ok((input, StyleTag::FontScaleX(x.max(0.0))))
}
fn parse_fscy<'a>(input: &'a str) -> IResult<&'a str, StyleTag> {
    let (input, y) = preceded(tag(r"\fscy"), double).parse(input)?;
    Ok((input, StyleTag::FontScaleY(y.max(0.0))))
}

// \fsc 应该将字体缩放重置为样式的缩放
// https://github.com/libass/libass/blob/7d8e335b094f00c57fb557b01e93b60a17e63434/libass/ass_parse.c#L455-L456
fn parse_fsc(input: &str) -> IResult<&str, StyleTag> {
    let (input, _) = tag(r"\fsc")(input)?;
    Ok((input, StyleTag::FontScaleReset))
}

fn parse_fsp(input: &str) -> IResult<&str, StyleTag> {
    preceded(tag(r"\fsp"), map(double, StyleTag::FontSpacing)).parse(input)
}

fn parse_fs(input: &str) -> IResult<&str, StyleTag> {
    let (input, val) = preceded(tag(r"\fs"), double).parse(input)?;
    // https://github.com/libass/libass/blob/7d8e335b094f00c57fb557b01e93b60a17e63434/libass/ass_parse.c#L474-L475
    if val <= 0.0 {
        return Ok((input, StyleTag::FontSizeReset));
    }
    Ok((input, StyleTag::FontSize(val)))
}

fn parse_bord(input: &str) -> IResult<&str, StyleTag> {
    let (input, val) = preceded(tag(r"\bord"), double).parse(input)?;
    Ok((input, StyleTag::Bord(val.max(0.0))))
}
fn parse_xbord(input: &str) -> IResult<&str, StyleTag> {
    let (input, val) = preceded(tag(r"\xbord"), double).parse(input)?;
    Ok((input, StyleTag::XBord(val.max(0.0))))
}
fn parse_ybord(input: &str) -> IResult<&str, StyleTag> {
    let (input, val) = preceded(tag(r"\ybord"), double).parse(input)?;
    Ok((input, StyleTag::YBord(val.max(0.0))))
}

// 忽略前后空格的逗号解析器
fn comma<'a, E: ParseError<&'a str>>(input: &'a str) -> IResult<&'a str, char, E> {
    delimited(multispace0, char(','), multispace0).parse(input)
}

fn parse_move(input: &str) -> IResult<&str, StyleTag> {
    let (input, _) = tag(r"\move")(input)?;

    // 坐标部分：严格为 4 个 double，中间以逗号分隔
    let coords = (
        terminated(double, comma),
        terminated(double, comma),
        terminated(double, comma),
        double,
    );

    // 时间部分：严格为“,+i64,+i64”，要么两个都给，要么都不给
    let times = (
        preceded(comma, complete::i64),
        preceded(comma, complete::i64),
    );

    let (input, ((x1, y1, x2, y2), opt_times)) =
        delimited(char('('), (coords, opt(times)), char(')')).parse(input)?;

    let (t1, t2) = match opt_times {
        Some((t1, t2)) => (Some(t1), Some(t2)),
        None => (None, None),
    };

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
        map(take_while(|s| s != '\\'), |font_name: &str| {
            StyleTag::FontName(font_name.to_string())
        }),
    )
    .parse(input)
}

fn parse_all_alpha(input: &str) -> IResult<&str, StyleTag> {
    let (input, _) = tag(r"\alpha").parse(input)?;
    // Skip optional &H or & prefix
    let (input, _) = opt(alt((tag("&H"), tag("&")))).parse(input)?;
    let (input, hex_str) = context(
        "alpha hex code must be 2 digits",
        verify(hex_digit1, |s: &str| s.len() == 2),
    )
    .parse(input)?;

    let alpha_val = u8::from_str_radix(hex_str, 16)
        .map(|val| 255u8.saturating_sub(val))
        .map_err(|_| {
            nom::Err::Error(nom::error::Error::new(input, nom::error::ErrorKind::MapRes))
        })?;

    Ok((input, StyleTag::AllAlpha(alpha_val)))
}
fn parse_1a(input: &str) -> IResult<&str, StyleTag> {
    let (input, _) = tag(r"\1a").parse(input)?;
    // Skip optional &H or & prefix
    let (input, _) = opt(alt((tag("&H"), tag("&")))).parse(input)?;
    let (input, hex_str) = context(
        "1a hex code must be 2 digits",
        verify(hex_digit1, |s: &str| s.len() == 2),
    )
    .parse(input)?;

    let alpha_val = u8::from_str_radix(hex_str, 16)
        .map(|val| 255u8.saturating_sub(val))
        .map_err(|_| {
            nom::Err::Error(nom::error::Error::new(input, nom::error::ErrorKind::MapRes))
        })?;

    Ok((input, StyleTag::PrimaryAlpha(alpha_val)))
}
fn parse_2a(input: &str) -> IResult<&str, StyleTag> {
    let (input, _) = tag(r"\2a").parse(input)?;
    // Skip optional &H or & prefix
    let (input, _) = opt(alt((tag("&H"), tag("&")))).parse(input)?;
    let (input, hex_str) = context(
        "2a hex code must be 2 digits",
        verify(hex_digit1, |s: &str| s.len() == 2),
    )
    .parse(input)?;

    let alpha_val = u8::from_str_radix(hex_str, 16)
        .map(|val| 255u8.saturating_sub(val))
        .map_err(|_| {
            nom::Err::Error(nom::error::Error::new(input, nom::error::ErrorKind::MapRes))
        })?;

    Ok((input, StyleTag::SecondaryAlpha(alpha_val)))
}
fn parse_3a(input: &str) -> IResult<&str, StyleTag> {
    let (input, _) = tag(r"\3a").parse(input)?;
    // Skip optional &H or & prefix
    let (input, _) = opt(alt((tag("&H"), tag("&")))).parse(input)?;
    let (input, hex_str) = context(
        "3a hex code must be 2 digits",
        verify(hex_digit1, |s: &str| s.len() == 2),
    )
    .parse(input)?;

    let alpha_val = u8::from_str_radix(hex_str, 16)
        .map(|val| 255u8.saturating_sub(val))
        .map_err(|_| {
            nom::Err::Error(nom::error::Error::new(input, nom::error::ErrorKind::MapRes))
        })?;

    Ok((input, StyleTag::OutlineAlpha(alpha_val)))
}
fn parse_4a(input: &str) -> IResult<&str, StyleTag> {
    let (input, _) = tag(r"\4a").parse(input)?;
    // Skip optional &H or & prefix
    let (input, _) = opt(alt((tag("&H"), tag("&")))).parse(input)?;
    let (input, hex_str) = context(
        "4a hex code must be 2 digits",
        verify(hex_digit1, |s: &str| s.len() == 2),
    )
    .parse(input)?;

    let alpha_val = u8::from_str_radix(hex_str, 16)
        .map(|val| 255u8.saturating_sub(val))
        .map_err(|_| {
            nom::Err::Error(nom::error::Error::new(input, nom::error::ErrorKind::MapRes))
        })?;

    Ok((input, StyleTag::BackAlpha(alpha_val)))
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
    warn!("忽略 a 覆盖标签");
    Ok((
        input,
        // NOTE: if use \a with \an will override \an
        StyleTag::Alignment(Alignment::BottomCenter),
    ))
}

fn parse_pos(input: &str) -> IResult<&str, StyleTag> {
    let (input, _) = tag(r"\pos")(input)?;
    let (input, (x, y)) =
        delimited(char('('), separated_pair(double, comma, double), char(')')).parse(input)?;
    Ok((input, StyleTag::Position(PointF64 { x, y })))
}

fn parse_fade(input: &str) -> IResult<&str, StyleTag> {
    let (input, tag) = alt((tag(r"\fade"), tag(r"\fad"))).parse(input)?;
    if tag == r"\fad" {
        let (input, (time1, time2)) = delimited(
            char('('),
            separated_pair(complete::i64, comma, complete::i64),
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
                preceded(comma, complete::u8),
                preceded(comma, complete::u8),
                preceded(comma, complete::i64),
                preceded(comma, complete::i64),
                preceded(comma, complete::i64),
                preceded(comma, complete::i64),
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
    let (input, (x, y)) =
        delimited(char('('), separated_pair(double, comma, double), char(')')).parse(input)?;
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
                preceded(comma, double),
                preceded(comma, double),
                preceded(comma, many1(parse_animated_style_tag)),
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
                preceded(comma, double),
                preceded(comma, many1(parse_animated_style_tag)),
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
            (double, preceded(comma, many1(parse_animated_style_tag))),
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

    let (input, style_tag) = delimited(
        char('('),
        alt((
            // --- 分支 1: 解析矩形格式 "x1,y1,x2,y2" ---
            map(
                (
                    complete::i32,
                    preceded(comma, complete::i32),
                    preceded(comma, complete::i32),
                    preceded(comma, complete::i32),
                ),
                |(x1, y1, x2, y2)| StyleTag::ClipRectangle {
                    p1: PointI32 { x: x1, y: y1 },
                    p2: PointI32 { x: x2, y: y2 },
                    is_display,
                },
            ),
            // --- 分支 2: 解析绘图格式 "scale,drawing" 或 "drawing" ---
            map_res(
                alt((
                    separated_pair(complete::i32, &comma, is_not(")")),
                    map(is_not(")"), |s| (1, s)), // 如果只有一个参数，scale默认为1
                )),
                |(scale, drawing_str)| {
                    match parse_drawing_commands(drawing_str) {
                        // 必须完全消耗 drawing_str 才算成功
                        Ok(("", drawing)) => Ok(StyleTag::ClipDrawing {
                            scale,
                            drawing,
                            is_display,
                        }),
                        // 如果解析失败或有剩余，则整个分支匹配失败
                        _ => Err(nom::error::Error::new(
                            drawing_str,
                            nom::error::ErrorKind::Verify,
                        )),
                    }
                },
            ),
        )),
        char(')'),
    )
    .parse(input)?;

    Ok((input, style_tag))
}

fn parse_c(input: &str) -> IResult<&str, StyleTag> {
    let (input, _) = alt((tag(r"\c"), tag(r"\1c"))).parse(input)?;
    // Skip optional &H or & prefix
    let (input, _) = opt(alt((tag("&H"), tag("&")))).parse(input)?;
    let (input, hex_str) = context(
        "c hex code must be 6 digits",
        verify(hex_digit1, |s: &str| s.len() == 6),
    )
    .parse(input)?;

    let color = u32::from_str_radix(hex_str, 16)
        .map(AssColorRgb::from_ass_override_u32)
        .map_err(|_| {
            nom::Err::Error(nom::error::Error::new(input, nom::error::ErrorKind::MapRes))
        })?;

    Ok((input, StyleTag::PrimaryColor(color)))
}
fn parse_2c(input: &str) -> IResult<&str, StyleTag> {
    let (input, _) = tag(r"\2c").parse(input)?;
    // Skip optional &H or & prefix
    let (input, _) = opt(alt((tag("&H"), tag("&")))).parse(input)?;
    let (input, hex_str) = context(
        "2c hex code must be 6 digits",
        verify(hex_digit1, |s: &str| s.len() == 6),
    )
    .parse(input)?;

    let color = u32::from_str_radix(hex_str, 16)
        .map(AssColorRgb::from_ass_override_u32)
        .map_err(|_| {
            nom::Err::Error(nom::error::Error::new(input, nom::error::ErrorKind::MapRes))
        })?;

    Ok((input, StyleTag::SecondaryColor(color)))
}

fn parse_3c(input: &str) -> IResult<&str, StyleTag> {
    let (input, _) = tag(r"\3c").parse(input)?;
    // Skip optional &H or & prefix
    let (input, _) = opt(alt((tag("&H"), tag("&")))).parse(input)?;
    let (input, hex_str) = context(
        "3c hex code must be 6 digits",
        verify(hex_digit1, |s: &str| s.len() == 6),
    )
    .parse(input)?;

    let color = u32::from_str_radix(hex_str, 16)
        .map(AssColorRgb::from_ass_override_u32)
        .map_err(|_| {
            nom::Err::Error(nom::error::Error::new(input, nom::error::ErrorKind::MapRes))
        })?;

    Ok((input, StyleTag::OutlineColor(color)))
}

fn parse_4c(input: &str) -> IResult<&str, StyleTag> {
    let (input, _) = tag(r"\4c").parse(input)?;
    // Skip optional &H or & prefix
    let (input, _) = opt(alt((tag("&H"), tag("&")))).parse(input)?;
    let (input, hex_str) = context(
        "4c hex code must be 6 digits",
        verify(hex_digit1, |s: &str| s.len() == 6),
    )
    .parse(input)?;

    let color = u32::from_str_radix(hex_str, 16)
        .map(AssColorRgb::from_ass_override_u32)
        .map_err(|_| {
            nom::Err::Error(nom::error::Error::new(input, nom::error::ErrorKind::MapRes))
        })?;

    Ok((input, StyleTag::BackColor(color)))
}

fn parse_r(input: &str) -> IResult<&str, StyleTag> {
    let (input, _) = tag(r"\r").parse(input)?;
    if input.is_empty() || input.starts_with(r"\") {
        return Ok((input, StyleTag::Reset(None)));
    }
    let (input, style_name) = take_while1(|c: char| c != '\\').parse(input)?;
    let trimmed_name = style_name.trim();

    if trimmed_name.is_empty() {
        Ok((input, StyleTag::Reset(None)))
    } else {
        Ok((input, StyleTag::Reset(Some(trimmed_name.to_string()))))
    }
}

fn parse_be(input: &str) -> IResult<&str, StyleTag> {
    let (input, _) = tag(r"\be")(input)?;
    let (input, val) = double(input)?;
    // https://github.com/libass/libass/blob/534a5f8299c5ab3c2782856fcb843bfea47b7afc/libass/ass_parse.c#L817-L821
    let adjusted_val = val + 0.5;
    let clamped_val = adjusted_val.max(0.0).min(127.0) as u32;
    match clamped_val {
        0 => Ok((input, StyleTag::BlurEdges(BlurEdgesTypes::NonBlurEdges))),
        _ => Ok((
            input,
            StyleTag::BlurEdges(BlurEdgesTypes::Weight(clamped_val)),
        )),
    }
}

fn parse_b(input: &str) -> IResult<&str, StyleTag> {
    let (input, _) = tag(r"\b")(input)?;
    let (input, val) = complete::u32(input)?;
    match val {
        0 => Ok((input, StyleTag::Bold(BoldType::NonBold))),
        1 => Ok((input, StyleTag::Bold(BoldType::Bold))),
        // https://github.com/libass/libass/blob/534a5f8299c5ab3c2782856fcb843bfea47b7afc/libass/ass_parse.c#L827
        v if v >= 100 => Ok((input, StyleTag::Bold(BoldType::Weight(v)))),
        _ => Err(nom::Err::Error(nom::error::Error::new(
            input,
            nom::error::ErrorKind::Verify,
        ))),
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
    let (input, centiseconds) = double.parse(input)?;
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
    match val {
        0 | 1 | 2 | 3 => {
            return Ok((
                input,
                StyleTag::WrapStyle(WrapStyle::try_from(val).unwrap()),
            ))
        }
        _ => Err(nom::Err::Error(nom::error::Error::new(
            input,
            nom::error::ErrorKind::Verify,
        ))),
    }
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

#[cfg(test)]
mod tests {
    use proptest::prelude::*;

    use crate::types::{drawing::DrawingCommand, parse_style_override::*};

    #[test]
    fn parses_fsc_reset_correctly() {
        let s = "\\fsc";
        let (input, result) = parse_fsc(s).unwrap();
        assert_eq!(input, "");
        assert_eq!(result, StyleTag::FontScaleReset);
    }
    #[test]
    fn parses_t_format1_correctly() {
        // 格式1: (tags) - 只有动画标签
        let s = r"\t(\fs20)";
        let (input, result) = parse_t(&s).unwrap();
        assert_eq!(input, "");
        match result {
            StyleTag::Transform {
                start_time,
                end_time,
                acceleration,
                tags,
            } => {
                assert!(start_time.is_none());
                assert!(end_time.is_none());
                assert!(acceleration.is_none());
                assert_eq!(tags.len(), 1);
                assert!(matches!(tags[0], AnimatedStyleTag::FontSize(_)));
            }
            _ => panic!("应返回 Transform，实际返回 {:?}", result),
        }
    }
    #[test]
    fn parses_t_multiple_tags_correctly() {
        // 测试多个嵌套的动画标签
        let s = r"\t(\fs20\c&HFF0000\fscx150)";
        let (input, result) = parse_t(&s).unwrap();
        assert_eq!(input, "");
        match result {
            StyleTag::Transform {
                start_time,
                end_time,
                acceleration,
                tags,
            } => {
                assert!(start_time.is_none());
                assert!(end_time.is_none());
                assert!(acceleration.is_none());
                assert_eq!(tags.len(), 3);
                // 验证标签类型
                assert!(matches!(tags[0], AnimatedStyleTag::FontSize(_)));
                assert!(matches!(tags[1], AnimatedStyleTag::PrimaryColor(_)));
                assert!(matches!(tags[2], AnimatedStyleTag::FontScaleX(_)));
            }
            _ => panic!("应返回 Transform，实际返回 {:?}", result),
        }
    }
    #[test]
    fn parses_r_none_correctly() {
        let s1 = r"\r";
        let s2 = r"\r\b1";

        let (input1, result1) = parse_r(s1).unwrap();
        assert_eq!(input1, "");
        assert_eq!(result1, StyleTag::Reset(None));

        let (input2, result2) = parse_r(s2).unwrap();
        assert_eq!(input2, r"\b1");
        assert_eq!(result2, StyleTag::Reset(None));
    }
    proptest! {
        #[test]
        fn parses_xshad_yshad_correctly(
            v in any::<f64>()
        ) {
            let xshad_s = format!("\\xshad{}", v);
            let yshad_s = format!("\\yshad{}", v);
            let (input_x, result_x) = parse_xshad(&xshad_s).unwrap();
            let (input_y, result_y) = parse_yshad(&yshad_s).unwrap();
            prop_assert_eq!(input_x, "");
            prop_assert_eq!(result_x, StyleTag::XShadow(v));
            prop_assert_eq!(input_y, "");
            prop_assert_eq!(result_y, StyleTag::YShadow(v));
        }
        #[test]
        fn parses_shad_correctly(
            v in any::<f64>()
        ) {
            let shad_s = format!("\\shad{}", v);
            let (input, result) = parse_shad(&shad_s).unwrap();
            prop_assert_eq!(input, "");

            match result {
                StyleTag::Shadow(actual) => {
                    if v < 0.0 && !v.is_nan() {
                        prop_assert_eq!(actual, 0.0, "负数应被强制设为 0.0");
                    } else {
                        prop_assert_eq!(actual, v, "非负数应保持原值");
                    }
                }
                _ => panic!("应返回 Shadow，实际返回 {:?}", result),
            }
        }
        #[test]
        fn parses_fax_fay_correctly(
            v in any::<f64>()
        ) {
            let fax_s = format!("\\fax{}", v);
            let fay_s = format!("\\fay{}", v);

            let (input_x, result_x) = parse_fax(&fax_s).unwrap();
            let (input_y, result_y) = parse_fay(&fay_s).unwrap();

            prop_assert_eq!(input_x, "");
            prop_assert_eq!(result_x, StyleTag::FactorX(v));
            prop_assert_eq!(input_y, "");
            prop_assert_eq!(result_y, StyleTag::FactorY(v));
        }
        #[test]
        fn parses_blur_correctly(
            v in any::<f64>()
        ) {
            let blur_s = format!("\\blur{}", v);
            let (input, result) = parse_blur(&blur_s).unwrap();
            prop_assert_eq!(input, "");

            match result {
                StyleTag::Blur(actual) => {
                    if v.is_nan() {
                        prop_assert!(actual.is_nan(), "NaN 应保持为 NaN");
                    } else if v < 0.0 {
                        prop_assert_eq!(actual, 0.0, "负数应被夹取为 0.0");
                    } else if v > 100.0 {
                        prop_assert_eq!(actual, 100.0, ">100 应被夹取为 100.0");
                    } else {
                        prop_assert_eq!(actual, v, "范围内数值应保持原值");
                    }
                }
                _ => panic!("应返回 Blur，实际返回 {:?}", result),
            }
        }
        #[test]
        fn parses_fscx_fscy_correctly(
            v in any::<f64>()
        ) {
            let fscx_s = format!("\\fscx{}", v);
            let fscy_s = format!("\\fscy{}", v);

            let (input_x, result_x) = parse_fscx(&fscx_s).unwrap();
            let (input_y, result_y) = parse_fscy(&fscy_s).unwrap();

            prop_assert_eq!(input_x, "");
            prop_assert_eq!(input_y, "");

            match result_x { StyleTag::FontScaleX(actual) => {
                if v.is_nan() { prop_assert!(actual.is_nan(), "NaN 应保持为 NaN"); }
                else if v < 0.0 { prop_assert_eq!(actual, 0.0, "负数应被夹取为 0.0"); }
                else { prop_assert_eq!(actual, v, "非负应保持原值"); }
            }, _ => panic!("应返回 FontScaleX，实际返回 {:?}", result_x) }

            match result_y { StyleTag::FontScaleY(actual) => {
                if v.is_nan() { prop_assert!(actual.is_nan(), "NaN 应保持为 NaN"); }
                else if v < 0.0 { prop_assert_eq!(actual, 0.0, "负数应被夹取为 0.0"); }
                else { prop_assert_eq!(actual, v, "非负应保持原值"); }
            }, _ => panic!("应返回 FontScaleY，实际返回 {:?}", result_y) }
        }
        #[test]
        fn parses_fsp_correctly(
            v in any::<f64>()
        ) {
            let s = format!("\\fsp{}", v);
            let (input, result) = parse_fsp(&s).unwrap();
            prop_assert_eq!(input, "");
            match result {
                StyleTag::FontSpacing(actual) => {
                    if v.is_nan() { prop_assert!(actual.is_nan(), "NaN 应保持为 NaN"); }
                    else { prop_assert_eq!(actual, v, "应保持原值"); }
                }
                _ => panic!("应返回 FontSpacing，实际返回 {:?}", result),
            }
        }
        #[test]
        fn parses_fs_correctly(
            v in any::<f64>()
        ) {
            let s = format!("\\fs{}", v);
            let (input, result) = parse_fs(&s).unwrap();
            prop_assert_eq!(input, "");
            if v <= 0.0 {
                prop_assert!(matches!(result, StyleTag::FontSizeReset), "v <= 0.0 应重置为样式字体大小");
            } else {
                match result {
                    StyleTag::FontSize(actual) => {
                        if v.is_nan() { prop_assert!(actual.is_nan(), "NaN 应保持为 NaN"); }
                        else { prop_assert_eq!(actual, v, "应保持原值"); }
                    }
                    _ => panic!("应返回 FontSize，实际返回 {:?}", result),
                }
            }
        }
        #[test]
        fn parses_bord_xbord_ybord_correctly(
            v in any::<f64>()
        ) {
            let bord_s = format!("\\bord{}", v);
            let xbord_s = format!("\\xbord{}", v);
            let ybord_s = format!("\\ybord{}", v);

            let (input_b, result_b) = parse_bord(&bord_s).unwrap();
            let (input_x, result_x) = parse_xbord(&xbord_s).unwrap();
            let (input_y, result_y) = parse_ybord(&ybord_s).unwrap();

            prop_assert_eq!(input_b, "");
            prop_assert_eq!(input_x, "");
            prop_assert_eq!(input_y, "");

            match result_b { StyleTag::Bord(actual) => {
                if v.is_nan() { prop_assert!(actual.is_nan(), "NaN 应保持为 NaN"); }
                else if v < 0.0 { prop_assert_eq!(actual, 0.0, "负数应被夹取为 0.0"); }
                else { prop_assert_eq!(actual, v, "非负应保持原值"); }
            }, _ => panic!("应返回 Bord，实际返回 {:?}", result_b) }

            match result_x { StyleTag::XBord(actual) => {
                if v.is_nan() { prop_assert!(actual.is_nan(), "NaN 应保持为 NaN"); }
                else if v < 0.0 { prop_assert_eq!(actual, 0.0, "负数应被夹取为 0.0"); }
                else { prop_assert_eq!(actual, v, "非负应保持原值"); }
            }, _ => panic!("应返回 XBord，实际返回 {:?}", result_x) }

            match result_y { StyleTag::YBord(actual) => {
                if v.is_nan() { prop_assert!(actual.is_nan(), "NaN 应保持为 NaN"); }
                else if v < 0.0 { prop_assert_eq!(actual, 0.0, "负数应被夹取为 0.0"); }
                else { prop_assert_eq!(actual, v, "非负应保持原值"); }
            }, _ => panic!("应返回 YBord，实际返回 {:?}", result_y) }
        }
        #[test]
        fn parses_move_no_time_correctly(
            x1 in any::<f64>(),
            y1 in any::<f64>(),
            x2 in any::<f64>(),
            y2 in any::<f64>()
        ) {
            let s = format!("\\move({},{},{},{})", x1, y1, x2, y2);
            let (input, result) = parse_move(&s).unwrap();
            prop_assert_eq!(input, "");
            match result {
                StyleTag::Move { start, end, start_time, end_time } => {
                    prop_assert_eq!(start.x, x1);
                    prop_assert_eq!(start.y, y1);
                    prop_assert_eq!(end.x, x2);
                    prop_assert_eq!(end.y, y2);
                    prop_assert!(start_time.is_none());
                    prop_assert!(end_time.is_none());
                }
                _ => panic!("应返回 Move，实际返回 {:?}", result),
            }
        }
        #[test]
        fn parses_move_with_t1_t2_correctly(
            x1 in any::<f64>(),
            y1 in any::<f64>(),
            x2 in any::<f64>(),
            y2 in any::<f64>(),
            t1 in any::<i64>(),
            t2 in any::<i64>()
        ) {
            let s = format!("\\move({},{},{},{},{},{})", x1, y1, x2, y2, t1, t2);
            let (input, result) = parse_move(&s).unwrap();
            prop_assert_eq!(input, "");
            match result {
                StyleTag::Move { start, end, start_time, end_time } => {
                    prop_assert_eq!(start.x, x1);
                    prop_assert_eq!(start.y, y1);
                    prop_assert_eq!(end.x, x2);
                    prop_assert_eq!(end.y, y2);
                    prop_assert_eq!(start_time, Some(TimeDelta::milliseconds(t1)));
                    prop_assert_eq!(end_time, Some(TimeDelta::milliseconds(t2)));
                }
                _ => panic!("应返回 Move，实际返回 {:?}", result),
            }
        }
        #[test]
        fn parses_frx_fry_correctly(
            v in any::<f64>()
        ) {
            let frx_s = format!("\\frx{}", v);
            let fry_s = format!("\\fry{}", v);

            let (input_x, result_x) = parse_frx(&frx_s).unwrap();
            let (input_y, result_y) = parse_fry(&fry_s).unwrap();

            prop_assert_eq!(input_x, "");
            prop_assert_eq!(input_y, "");

            match result_x { StyleTag::FontRotationX(actual) => {
                if v.is_nan() { prop_assert!(actual.is_nan(), "NaN 应保持为 NaN"); }
                else { prop_assert_eq!(actual, v, "应保持原值"); }
            }, _ => panic!("应返回 FontRotationX，实际返回 {:?}", result_x) }

            match result_y { StyleTag::FontRotationY(actual) => {
                if v.is_nan() { prop_assert!(actual.is_nan(), "NaN 应保持为 NaN"); }
                else { prop_assert_eq!(actual, v, "应保持原值"); }
            }, _ => panic!("应返回 FontRotationY，实际返回 {:?}", result_y) }
        }
        #[test]
        fn parses_frz_and_fr_alias_correctly(
            v in any::<f64>()
        ) {
            let frz_s = format!("\\frz{}", v);
            let fr_s = format!("\\fr{}", v);

            let (input_z1, result_z1) = parse_frz(&frz_s).unwrap();
            let (input_z2, result_z2) = parse_frz(&fr_s).unwrap();

            prop_assert_eq!(input_z1, "");
            prop_assert_eq!(input_z2, "");

            match result_z1 { StyleTag::FontRotationZ(actual) => {
                if v.is_nan() { prop_assert!(actual.is_nan(), "NaN 应保持为 NaN"); }
                else { prop_assert_eq!(actual, v, "应保持原值"); }
            }, _ => panic!("应返回 FontRotationZ，实际返回 {:?}", result_z1) }

            match result_z2 { StyleTag::FontRotationZ(actual) => {
                if v.is_nan() { prop_assert!(actual.is_nan(), "NaN 应保持为 NaN"); }
                else { prop_assert_eq!(actual, v, "应保持原值"); }
            }, _ => panic!("应返回 FontRotationZ，实际返回 {:?}", result_z2) }
        }
        #[test]
        fn parses_fn_simple_correctly(
            name in r"[^\\{}\s]*".prop_filter("name must not be empty after trimming", |s| !s.trim().is_empty()),
        ) {
            let s = format!("\\fn{}", name);
            let (input, result) = parse_fn(&s).unwrap();
            prop_assert_eq!(input, "");
            match result {
                StyleTag::FontName(actual) => prop_assert_eq!(actual, name),
                _ => panic!("应返回 FontName，实际返回 {:?}", result),
            }
        }
        #[test]
        fn parses_fn_stops_before_next_tag(
            name in r"[^\\{}\s]*".prop_filter("name must not be empty after trimming", |s| !s.trim().is_empty()),
        ) {
            let s = format!("\\fn{}\\b1", name);
            let (input, result) = parse_fn(&s).unwrap();
            prop_assert_eq!(input, "\\b1");
            match result {
                StyleTag::FontName(actual) => prop_assert_eq!(actual, name),
                _ => panic!("应返回 FontName，实际返回 {:?}", result),
            }
        }
        #[test]
        fn parses_fn_alpha_correctly(
            alpha in 0u8..=255
        ) {
            let all_s = format!("\\alpha&H{:02X}", alpha);
            let a1_s = format!("\\1a&H{:02X}", alpha);
            let a2_s = format!("\\2a&H{:02X}", alpha);
            let a3_s = format!("\\3a&H{:02X}", alpha);
            let a4_s = format!("\\4a&H{:02X}", alpha);

            let (input_all, result_all) = parse_all_alpha(&all_s).unwrap();
            let (input_a1, result_a1) = parse_1a(&a1_s).unwrap();
            let (input_a2, result_a2) = parse_2a(&a2_s).unwrap();
            let (input_a3, result_a3) = parse_3a(&a3_s).unwrap();
            let (input_a4, result_a4) = parse_4a(&a4_s).unwrap();

            prop_assert_eq!(input_all, "");
            prop_assert_eq!(input_a1, "");
            prop_assert_eq!(input_a2, "");
            prop_assert_eq!(input_a3, "");
            prop_assert_eq!(input_a4, "");

            match result_all {
                StyleTag::AllAlpha(actual) => prop_assert_eq!(actual, 255u8.saturating_sub(alpha)),
                _ => panic!("应返回 AllAlpha，实际返回 {:?}", result_all),
            }
            match result_a1 {
                StyleTag::PrimaryAlpha(actual) => prop_assert_eq!(actual, 255u8.saturating_sub(alpha)),
                _ => panic!("应返回 PrimaryAlpha，实际返回 {:?}", result_a1),
            }
            match result_a2 {
                StyleTag::SecondaryAlpha(actual) => prop_assert_eq!(actual, 255u8.saturating_sub(alpha)),
                _ => panic!("应返回 SecondaryAlpha，实际返回 {:?}", result_a2),
            }
            match result_a3 {
                StyleTag::OutlineAlpha(actual) => prop_assert_eq!(actual, 255u8.saturating_sub(alpha)),
                _ => panic!("应返回 OutlineAlpha，实际返回 {:?}", result_a3),
            }
            match result_a4 {
                StyleTag::BackAlpha(actual) => prop_assert_eq!(actual, 255u8.saturating_sub(alpha)),
                _ => panic!("应返回 BackAlpha，实际返回 {:?}", result_a4),
            }
        }
        #[test]
        fn parses_fn_an_correctly(
            an in 1u8..=9
        ) {
            let s = format!("\\an{}", an);
            let (input, result) = parse_an(&s).unwrap();
            prop_assert_eq!(input, "");
            let expected = match an {
                1 => aweft_libass::types::Alignment::BottomLeft,
                2 => aweft_libass::types::Alignment::BottomCenter,
                3 => aweft_libass::types::Alignment::BottomRight,
                4 => aweft_libass::types::Alignment::MiddleLeft,
                5 => aweft_libass::types::Alignment::MiddleCenter,
                6 => aweft_libass::types::Alignment::MiddleRight,
                7 => aweft_libass::types::Alignment::TopLeft,
                8 => aweft_libass::types::Alignment::TopCenter,
                9 => aweft_libass::types::Alignment::TopRight,
                _ => unreachable!(),
            };
            match result {
                StyleTag::Alignment(actual) => prop_assert_eq!(actual, expected),
                _ => panic!("应返回 Alignment，实际返回 {:?}", result),
            }
        }
        #[test]
        fn parses_pos_correctly(
            v1 in any::<f64>(), v2 in any::<f64>()
        ) {
            let s = format!("\\pos({},{})", v1, v2);

            let (input, result) = parse_pos(&s).unwrap();

            prop_assert_eq!(input, "");

            match result {
                StyleTag::Position(PointF64{x: actual_x, y: actual_y}) => {
                    prop_assert_eq!(actual_x, v1);
                    prop_assert_eq!(actual_y, v2);
                },
                _ => panic!("应返回 Position，实际返回 {:?}", result)
            }
        }
        #[test]
        fn parses_fad_fade_correctly(
            fad_in_time in any::<i64>(),
            fad_out_time in any::<i64>(),
            fade_alpha1 in any::<u8>(),
            fade_alpha2 in any::<u8>(),
            fade_alpha3 in any::<u8>(),
            fade_time1 in any::<i64>(),
            fade_time2 in any::<i64>(),
            fade_time3 in any::<i64>(),
            fade_time4 in any::<i64>(),
        ) {
            let fad_s = format!("\\fad({},{})", fad_in_time, fad_out_time);
            let fade_s = format!("\\fade({},{},{},{},{},{},{})", fade_alpha1, fade_alpha2, fade_alpha3, fade_time1, fade_time2, fade_time3, fade_time4);

            let (input_fad, result_fad) = parse_fade(&fad_s).unwrap();
            let (input_fade, result_fade) = parse_fade(&fade_s).unwrap();

            prop_assert_eq!(input_fad, "");
            prop_assert_eq!(input_fade, "");

            match result_fad {
                StyleTag::SimpleFade{in_time: actual_in, out_time: actual_out} => {
                    prop_assert_eq!(actual_in.num_milliseconds(), fad_in_time);
                    prop_assert_eq!(actual_out.num_milliseconds(), fad_out_time);
                },
                _ => panic!("应返回 SimpleFade，实际返回 {:?}", result_fad)
            }
            match result_fade {
                StyleTag::Fade{
                    start_opacity: actual_alpha1,
                    mid_opacity: actual_alpha2,
                    end_opacity: actual_alpha3,
                    in_start_time: actual_time1,
                    in_end_time: actual_time2,
                    out_start_time: actual_time3,
                    out_end_time: actual_time4
                } => {
                    prop_assert_eq!(255u8.saturating_sub(actual_alpha1), fade_alpha1);
                    prop_assert_eq!(255u8.saturating_sub(actual_alpha2), fade_alpha2);
                    prop_assert_eq!(255u8.saturating_sub(actual_alpha3), fade_alpha3);
                    prop_assert_eq!(actual_time1.num_milliseconds(), fade_time1);
                    prop_assert_eq!(actual_time2.num_milliseconds(), fade_time2);
                    prop_assert_eq!(actual_time3.num_milliseconds(), fade_time3);
                    prop_assert_eq!(actual_time4.num_milliseconds(), fade_time4);
                },
                _ => panic!("应返回 Fade，实际返回 {:?}", result_fad)
            }
        }
         #[test]
         fn parses_org_correctly(
             x in any::<f64>(),
             y in any::<f64>()
         ) {
             let s = format!("\\org({},{})", x, y);
             let (input, result) = parse_org(&s).unwrap();
             prop_assert_eq!(input, "");
             match result {
                 StyleTag::Origin (PointF64{ x: actual_x, y: actual_y }) => {
                     prop_assert_eq!(actual_x, x);
                     prop_assert_eq!(actual_y, y);
                 }
                 _ => panic!("应返回 Origin，实际返回 {:?}", result),
             }
         }
         #[test]
         fn parses_t_format2_correctly(
             accel in any::<f64>()
         ) {
             // 格式2: (accel, tags) - 加速度和动画标签
             let s = format!("\\t({},\\fs20)", accel);
             let (input, result) = parse_t(&s).unwrap();
             prop_assert_eq!(input, "");
             match result {
                 StyleTag::Transform { start_time, end_time, acceleration, tags } => {
                     prop_assert!(start_time.is_none());
                     prop_assert!(end_time.is_none());
                     prop_assert_eq!(acceleration, Some(accel));
                     prop_assert_eq!(tags.len(), 1);
                     prop_assert!(matches!(tags[0], AnimatedStyleTag::FontSize(_)));
                 }
                 _ => panic!("应返回 Transform，实际返回 {:?}", result),
             }
         }
         #[test]
         fn parses_t_format3_correctly(
             t1 in any::<f64>(),
             t2 in any::<f64>()
         ) {
             // 格式3: (t1, t2, tags) - 开始时间、结束时间和动画标签
             let s = format!("\\t({},{},\\fs20)", t1, t2);
             let (input, result) = parse_t(&s).unwrap();
             prop_assert_eq!(input, "");
             match result {
                 StyleTag::Transform { start_time, end_time, acceleration, tags } => {
                     prop_assert_eq!(start_time, Some(TimeDelta::microseconds((t1 * 1_000_000.0) as i64)));
                     prop_assert_eq!(end_time, Some(TimeDelta::microseconds((t2 * 1_000_000.0) as i64)));
                     prop_assert!(acceleration.is_none());
                     prop_assert_eq!(tags.len(), 1);
                     prop_assert!(matches!(tags[0], AnimatedStyleTag::FontSize(_)));
                 }
                 _ => panic!("应返回 Transform，实际返回 {:?}", result),
             }
         }
         #[test]
         fn parses_t_format4_correctly(
             t1 in any::<f64>(),
             t2 in any::<f64>(),
             accel in any::<f64>()
         ) {
             // 格式4: (t1, t2, accel, tags) - 开始时间、结束时间、加速度和动画标签
             let s = format!("\\t({t1},{t2},{accel},\\fs20)");
             let (input, result) = parse_t(&s).unwrap();
             prop_assert_eq!(input, "");
             match result {
                 StyleTag::Transform { start_time, end_time, acceleration, tags } => {
                     prop_assert_eq!(start_time, Some(TimeDelta::microseconds((t1 * 1_000_000.0) as i64)));
                     prop_assert_eq!(end_time, Some(TimeDelta::microseconds((t2 * 1_000_000.0) as i64)));
                     prop_assert_eq!(acceleration, Some(accel));
                     prop_assert_eq!(tags.len(), 1);
                     prop_assert!(matches!(tags[0], AnimatedStyleTag::FontSize(_)));
                 }
                 _ => panic!("应返回 Transform，实际返回 {:?}", result),
             }
         }
         #[test]
         fn parse_clip_rectangle_correctly(
             x1 in any::<i32>(),
             y1 in any::<i32>(),
             x2 in any::<i32>(),
             y2 in any::<i32>()
         ) {
             let display_s = format!("\\clip({x1},{y1},{x2},{y2})");
             let undisplay_s = format!("\\iclip({x1},{y1},{x2},{y2})");
             let (input_display, result_display) = parse_clip(&display_s).unwrap();
             let (input_undisplay, result_undisplay) = parse_clip(&undisplay_s).unwrap();
             prop_assert_eq!(input_display, "");
             prop_assert_eq!(input_undisplay, "");
             match result_display {
                 StyleTag::ClipRectangle { p1, p2, is_display } => {
                     prop_assert_eq!(p1, PointI32 { x: x1, y: y1 });
                     prop_assert_eq!(p2, PointI32 { x: x2, y: y2 });
                     prop_assert_eq!(is_display, true);
                 }
                 _ => panic!("应返回 ClipRectangle，实际返回 {:?}", result_display),
             }
             match result_undisplay {
                 StyleTag::ClipRectangle { p1, p2, is_display } => {
                     prop_assert_eq!(p1, PointI32 { x: x1, y: y1 });
                     prop_assert_eq!(p2, PointI32 { x: x2, y: y2 });
                     prop_assert_eq!(is_display, false);
                 }
                 _ => panic!("应返回 ClipRectangle，实际返回 {:?}", result_undisplay),
             }
         }
         #[test]
         fn parse_clip_drawing_correctly(
             scale in any::<i32>(),
         ) {
             let display_s = format!("\\clip({scale},m 50 50)");
             let undisplay_s = format!("\\iclip({scale},m 50 50)");
             let (input_display, result_display) = parse_clip(&display_s).unwrap();
             let (input_undisplay, result_undisplay) = parse_clip(&undisplay_s).unwrap();
             prop_assert_eq!(input_display, "");
             prop_assert_eq!(input_undisplay, "");
             match result_display {
                 StyleTag::ClipDrawing { scale: actual_scale, drawing, is_display } => {
                     prop_assert_eq!(actual_scale, scale);
                    prop_assert_eq!(drawing, vec![DrawingCommand::MoveTo(PointF64 { x: 50.0, y: 50.0 })]);
                    prop_assert_eq!(is_display, true);
                }
                _ => panic!("应返回 ClipDrawing，实际返回 {:?}", result_display),
            }
            match result_undisplay {
                StyleTag::ClipDrawing { scale: actual_scale, drawing, is_display } => {
                    prop_assert_eq!(actual_scale, scale);
                    prop_assert_eq!(drawing, vec![DrawingCommand::MoveTo(PointF64 { x: 50.0, y: 50.0 })]);
                    prop_assert_eq!(is_display, false);
                 }
                 _ => panic!("应返回 ClipDrawing，实际返回 {:?}", result_undisplay),
             }
         }
        #[test]
        fn parses_fn_color_correctly(
            color in 0u32..=0xFFFFFF
        ) {
            let c_s = format!("\\c&H{:06X}", color);
            let c1_s = format!("\\1c&H{:06X}", color);
            let c2_s = format!("\\2c&H{:06X}", color);
            let c3_s = format!("\\3c&H{:06X}", color);
            let c4_s = format!("\\4c&H{:06X}", color);

            let (input_c, result_c) = parse_c(&c_s).unwrap();
            let (input_c1, result_c1) = parse_c(&c1_s).unwrap();
            let (input_c2, result_c2) = parse_2c(&c2_s).unwrap();
            let (input_c3, result_c3) = parse_3c(&c3_s).unwrap();
            let (input_c4, result_c4) = parse_4c(&c4_s).unwrap();

            prop_assert_eq!(input_c, "");
            prop_assert_eq!(input_c1, "");
            prop_assert_eq!(input_c2, "");
            prop_assert_eq!(input_c3, "");
            prop_assert_eq!(input_c4, "");

            match result_c {
                StyleTag::PrimaryColor(actual) => prop_assert_eq!(actual, AssColorRgb::from_ass_override_u32(color)),
                _ => panic!("应返回 PrimaryColor，实际返回 {:?}", result_c),
            }
            match result_c1 {
                StyleTag::PrimaryColor(actual) => prop_assert_eq!(actual, AssColorRgb::from_ass_override_u32(color)),
                _ => panic!("应返回 PrimaryColor，实际返回 {:?}", result_c),
            }
            match result_c2 {
                StyleTag::SecondaryColor(actual) => prop_assert_eq!(actual, AssColorRgb::from_ass_override_u32(color)),
                _ => panic!("应返回 SecondaryColor，实际返回 {:?}", result_c2),
            }
            match result_c3 {
                StyleTag::OutlineColor(actual) => prop_assert_eq!(actual, AssColorRgb::from_ass_override_u32(color)),
                _ => panic!("应返回 OutlineColor，实际返回 {:?}", result_c3),
            }
            match result_c4 {
                StyleTag::BackColor(actual) => prop_assert_eq!(actual, AssColorRgb::from_ass_override_u32(color)),
                _ => panic!("应返回 BackColor，实际返回 {:?}", result_c4),
            }
        }
        #[test]
        fn parses_r_some_correctly(
            // 此测试当中不应当为空
            name in r"[^\\{}\s]+".prop_filter("name must not be empty after trimming", |s| !s.trim().is_empty()),
        ) {
            let s = format!("\\r{}", name);

            let (input, result) = parse_r(&s).unwrap();

            prop_assert_eq!(input, "");

            match result {
                StyleTag::Reset(actual) => {
                    let expected = Some(name.trim().to_string());
                    prop_assert_eq!(actual, expected);
                }
                _ => panic!("应返回 Reset，实际返回 {:?}", result),
            }
        }
        #[test]
        fn parses_r_stops_before_next_tag(
            name in r"[^\\{}\s]+".prop_filter("name must not be empty after trimming", |s| !s.trim().is_empty()),
        ) {
            let s = format!("\\r{}\\b1", name);
            let (input, result) = parse_r(&s).unwrap();
            prop_assert_eq!(input, "\\b1");
            match result {
                StyleTag::Reset(actual) => {
                    let expected = Some(name.trim().to_string());
                    prop_assert_eq!(actual, expected);
                },
                _ => panic!("应返回 FontName，实际返回 {:?}", result),
            }
        }
        #[test]
        fn parses_be_correctly(
            val in any::<f64>()
        ) {
            let s = format!("\\be{}", val);
            let (input, result) = parse_be(&s).unwrap();
            prop_assert_eq!(input, "");

            // https://github.com/libass/libass/blob/534a5f8299c5ab3c2782856fcb843bfea47b7afc/libass/ass_parse.c#L817-L821
            let adjusted_val = val + 0.5;
            let clamped_val = adjusted_val.max(0.0).min(127.0) as u32;

            let expected = if clamped_val == 0 {
                StyleTag::BlurEdges(BlurEdgesTypes::NonBlurEdges)
            } else {
                StyleTag::BlurEdges(BlurEdgesTypes::Weight(clamped_val))
            };

            prop_assert_eq!(result, expected);
        }
        #[test]
        fn parses_b_correctly(
            val in any::<u32>()
        ) {
            let s = format!("\\b{}", val);

            // https://github.com/libass/libass/blob/534a5f8299c5ab3c2782856fcb843bfea47b7afc/libass/ass_parse.c#L827
            match val {
                0 => {
                    let (input, result) = parse_b(&s).unwrap();
                    prop_assert_eq!(input, "");
                    prop_assert_eq!(result, StyleTag::Bold(BoldType::NonBold));
                },
                1 => {
                    let (input, result) = parse_b(&s).unwrap();
                    prop_assert_eq!(input, "");
                    prop_assert_eq!(result, StyleTag::Bold(BoldType::Bold));
                },
                v if v >= 100 => {
                    let (input, result) = parse_b(&s).unwrap();
                    prop_assert_eq!(input, "");
                    prop_assert_eq!(result, StyleTag::Bold(BoldType::Weight(v)));
                },
                _ => {
                    prop_assert!(parse_b(&s).is_err());
                }
            }
        }
        #[test]
        fn parses_i_correctly(
          val in prop::sample::select(vec![0, 1])
        ) {
            let s = format!("\\i{}", val);
            let (input, result) = parse_i(&s).unwrap();
            prop_assert_eq!(input, "");

            prop_assert_eq!(result, StyleTag::Italic(val == 1));
        }
        #[test]
        fn parses_kf_correctly(
          val in any::<i32>()
        ) {
            // NOTE: libass 当中如果后面没有数字就使用 100 默认值，不过我们就必须要提供值好了
            let s1 = format!("\\kf{}", val);
            let s2 = format!("\\K{}", val);
            let (input1, result1) = parse_kf(&s1).unwrap();
            let (input2, result2) = parse_kf(&s2).unwrap();
            prop_assert_eq!(input1, "");
            prop_assert_eq!(input2, "");

            prop_assert_eq!(
                result1,
                StyleTag::Karaoke {
                    kind: KaraokeKind::KaraokeKf,
                    duration: TimeDelta::milliseconds(val as i64 * 10)
                }
            );
            prop_assert_eq!(
                result2,
                StyleTag::Karaoke {
                    kind: KaraokeKind::KaraokeKf,
                    duration: TimeDelta::milliseconds(val as i64 * 10)
                }
            );
        }
        #[test]
        fn parses_ko_correctly(
          val in any::<i32>()
        ) {
            // NOTE: libass 当中如果后面没有数字就使用 100 默认值，不过我们就必须要提供值好了
            let s = format!("\\ko{}", val);
            let (input, result) = parse_ko(&s).unwrap();
            prop_assert_eq!(input, "");

            prop_assert_eq!(
                result,
                StyleTag::Karaoke {
                    kind: KaraokeKind::KaraokeKo,
                    duration: TimeDelta::milliseconds(val as i64 * 10)
                }
            );
        }
        #[test]
        fn parses_k_correctly(
          val in any::<i32>()
        ) {
            // NOTE: libass 当中如果后面没有数字就使用 100 默认值，不过我们就必须要提供值好了
            let s = format!("\\k{}", val);
            let (input, result) = parse_k(&s).unwrap();
            prop_assert_eq!(input, "");

            prop_assert_eq!(
                result,
                StyleTag::Karaoke {
                    kind: KaraokeKind::Karaoke,
                    duration: TimeDelta::milliseconds(val as i64 * 10)
                }
            );
        }
        #[test]
        fn parses_s_correctly(
          val in prop::sample::select(vec![0, 1])
        ) {
            let s = format!("\\s{}", val);
            let (input, result) = parse_s(&s).unwrap();
            prop_assert_eq!(input, "");

            prop_assert_eq!(result, StyleTag::Strikeout(val == 1));
        }
        #[test]
        fn parses_u_correctly(
          val in prop::sample::select(vec![0, 1])
        ) {
            let s = format!("\\u{}", val);
            let (input, result) = parse_u(&s).unwrap();
            prop_assert_eq!(input, "");

            prop_assert_eq!(result, StyleTag::Underline(val == 1));
        }
        #[test]
        fn parses_pbo_correctly(
            val in any::<f64>()
        ) {
            let s = format!("\\pbo{}", val);
            let (input, result) = parse_pbo(&s).unwrap();
            prop_assert_eq!(input, "");

            prop_assert_eq!(result, StyleTag::DrawingBaselineOffset(val));
        }
        #[test]
        fn parses_p_correctly(
            val in any::<u32>()
        ) {
            let s = format!("\\p{}", val);
            let (input, result) = parse_p(&s).unwrap();
            prop_assert_eq!(input, "");

            prop_assert_eq!(result, StyleTag::DrawingScale(val));
        }
        #[test]
        fn parses_q_correctly(
          val in prop::sample::select(vec![0, 1, 2, 3])
        ) {
            let s = format!("\\q{}", val);
            let (input, result) = parse_q(&s).unwrap();
            prop_assert_eq!(input, "");

            match val {
                0 => {
                    prop_assert_eq!(result, StyleTag::WrapStyle(WrapStyle::Smart));
                },
                1 => {
                    prop_assert_eq!(result, StyleTag::WrapStyle(WrapStyle::EndOfWord));
                },
                2 => {
                    prop_assert_eq!(result, StyleTag::WrapStyle(WrapStyle::NoWrap));
                }
                3 => {
                    prop_assert_eq!(result, StyleTag::WrapStyle(WrapStyle::Smart));
                }
                _ => unreachable!()
            }
        }
        #[test]
        fn parses_fe_correctly(
          val in prop::sample::select(vec![
              932, // Shift_JIS (日语)
              936, // GBK (简体中文)
              949, // EUC-KR/Wansung (韩语)
              950, // Big5 (繁体中文)
              1361 // Johab (韩语)
          ])
        ) {
            let s = format!("\\fe{}", val);
            let (input, _result) = parse_fe(&s).unwrap();
            prop_assert_eq!(input, "");
        }
    }
}
