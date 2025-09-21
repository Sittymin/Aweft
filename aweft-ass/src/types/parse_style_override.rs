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
