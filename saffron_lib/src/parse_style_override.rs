use std::str::FromStr;

use nom::{
    branch::alt,
    bytes::complete::{is_not, tag, take_until, take_until1, take_while, take_while1},
    character::complete::{char, digit1, not_line_ending},
    combinator::{map, map_res, opt, verify},
    multi::{many0, separated_list0, separated_list1},
    sequence::{delimited, preceded, separated_pair, terminated, tuple},
    IResult,
};
use parse_drawing_command::parse_drawing_commands;

use super::types::*;
use crate::utils::*;

fn parse_xshad(input: &str) -> IResult<&str, StyleOverride> {
    let (input, _) = tag(r"\xshad")(input)?;
    let (input, val) = parse_float(input)?;
    Ok((input, StyleOverride::XShadow(val)))
}
fn parse_yshad(input: &str) -> IResult<&str, StyleOverride> {
    let (input, _) = tag(r"\yshad")(input)?;
    let (input, val) = parse_float(input)?;
    Ok((input, StyleOverride::YShadow(val)))
}

fn parse_fax(input: &str) -> IResult<&str, StyleOverride> {
    let (input, _) = tag(r"\fax")(input)?;
    let (input, val) = parse_float(input)?;
    Ok((input, StyleOverride::FactorX(val)))
}
fn parse_fay(input: &str) -> IResult<&str, StyleOverride> {
    let (input, _) = tag(r"\fay")(input)?;
    let (input, val) = parse_float(input)?;
    Ok((input, StyleOverride::FactorY(val)))
}

fn parse_blur(input: &str) -> IResult<&str, StyleOverride> {
    let (input, _) = tag(r"\blur")(input)?;
    let (input, val) = parse_float(input)?;
    Ok((input, StyleOverride::Blur(val)))
}

fn parse_fscx<'a>(input: &'a str) -> IResult<&'a str, StyleOverride> {
    let (input, _) = tag(r"\fscx")(input)?;
    let (input, x) = parse_float(input)?;

    Ok((input, StyleOverride::FontScaleX(x)))
}
fn parse_fscy<'a>(input: &'a str) -> IResult<&'a str, StyleOverride> {
    let (input, _) = tag(r"\fscy")(input)?;
    let (input, y) = parse_float(input)?;

    Ok((input, StyleOverride::FontScaleY(y)))
}

// \fsc 应该将字体缩放重置为样式的缩放
// https://github.com/libass/libass/blob/7d8e335b094f00c57fb557b01e93b60a17e63434/libass/ass_parse.c#L455-L456
fn parse_fsc(input: &str) -> IResult<&str, StyleOverride> {
    let (input, _) = tag(r"\fsc")(input)?;
    Ok((input, StyleOverride::FontScaleReset))
}

fn parse_fsp(input: &str) -> IResult<&str, StyleOverride> {
    let (input, _) = tag(r"\fsp")(input)?;
    let (input, val) = parse_float(input)?;
    Ok((input, StyleOverride::FontSpacing(val)))
}
fn parse_fs(input: &str) -> IResult<&str, StyleOverride> {
    let (input, _) = tag(r"\fs")(input)?;
    let (input, val) = parse_float(input)?;
    // WARN: 如果为负数应该重置为样式的字体大小
    // https://github.com/libass/libass/blob/7d8e335b094f00c57fb557b01e93b60a17e63434/libass/ass_parse.c#L474-L475
    Ok((input, StyleOverride::FontSize(val)))
}

fn parse_bord(input: &str) -> IResult<&str, StyleOverride> {
    let (input, _) = tag(r"\bord")(input)?;
    let (input, val) = parse_float(input)?;
    Ok((input, StyleOverride::Bord(val)))
}
fn parse_xbord(input: &str) -> IResult<&str, StyleOverride> {
    let (input, _) = tag(r"\xbord")(input)?;
    let (input, val) = parse_float(input)?;
    Ok((input, StyleOverride::XBord(val)))
}
fn parse_ybord(input: &str) -> IResult<&str, StyleOverride> {
    let (input, _) = tag(r"\ybord")(input)?;
    let (input, val) = parse_float(input)?;
    Ok((input, StyleOverride::YBord(val)))
}
fn parse_bold(input: &str) -> IResult<&str, StyleOverride> {
    let (input, _) = tag(r"\b")(input)?;
    let (input, val) = alt((
        map(char('0'), |_| BoldType::NonBold),
        map(char('1'), |_| BoldType::Bold),
        map_res(digit1, |n: &str| {
            let weight = n.parse::<i32>()?;
            Ok::<_, std::num::ParseIntError>(BoldType::Weight(weight.try_into().unwrap_or(100)))
        }),
    ))(input)?;
    Ok((input, StyleOverride::Bold(val)))
}

/// 演示解析“\move(...)”标签，如“\move(10,20,30,40,500,1000)”。
fn parse_move(input: &str) -> IResult<&str, StyleOverride> {
    let (input, _) = tag(r"\move")(input)?;
    let (input, (x1, y1, x2, y2, t1, t2)) = delimited(
        char('('),
        tuple((
            terminated(parse_float, char(',')),
            terminated(parse_float, char(',')),
            terminated(parse_float, char(',')),
            terminated(parse_float, opt(char(','))),
            opt(parse_u32),                      // 可选开始时间
            opt(preceded(char(','), parse_u32)), // 可选结束时间
        )),
        char(')'),
    )(input)?;

    Ok((
        input,
        StyleOverride::Move {
            start_x: x1,
            start_y: y1,
            end_x: x2,
            end_y: y2,
            start_time: t1,
            end_time: t2,
        },
    ))
}

fn parse_frx(input: &str) -> IResult<&str, StyleOverride> {
    let (input, _) = tag(r"\frx")(input)?;
    let (input, val) = parse_float(input)?;
    Ok((input, StyleOverride::FontRotationX(val)))
}
fn parse_fry(input: &str) -> IResult<&str, StyleOverride> {
    let (input, _) = tag(r"\fry")(input)?;
    let (input, val) = parse_float(input)?;
    Ok((input, StyleOverride::FontRotationY(val)))
}
fn parse_frz(input: &str) -> IResult<&str, StyleOverride> {
    let (input, _) = alt((tag(r"\frz"), tag(r"\fr")))(input)?;
    let (input, val) = parse_float(input)?;
    Ok((input, StyleOverride::FontRotationZ(val)))
}

fn parse_fn(input: &str) -> IResult<&str, StyleOverride> {
    let (input, _) = tag(r"\fn")(input)?;
    // 识别除了其他标签的全部字符
    let (input, val) = opt(take_until1(r"\"))(input)?;
    let font_name = val.unwrap_or("").trim().to_string();
    Ok((input, StyleOverride::FontName(font_name)))
}

fn parse_all_alpha(input: &str) -> IResult<&str, StyleOverride> {
    let (input, _) = tag(r"\alpha")(input)?;
    let (input, alpha) = parse_color(input)?;
    Ok((input, StyleOverride::AllAlpha(alpha)))
}
fn parse_1a(input: &str) -> IResult<&str, StyleOverride> {
    let (input, _) = tag(r"\1a")(input)?;
    let (input, alpha) = parse_color(input)?;
    Ok((input, StyleOverride::PrimaryAlpha(alpha)))
}
fn parse_2a(input: &str) -> IResult<&str, StyleOverride> {
    let (input, _) = tag(r"\2a")(input)?;
    let (input, alpha) = parse_color(input)?;
    Ok((input, StyleOverride::SecondaryAlpha(alpha)))
}
fn parse_3a(input: &str) -> IResult<&str, StyleOverride> {
    let (input, _) = tag(r"\3a")(input)?;
    let (input, alpha) = parse_color(input)?;
    Ok((input, StyleOverride::OutlineAlpha(alpha)))
}
fn parse_4a(input: &str) -> IResult<&str, StyleOverride> {
    let (input, _) = tag(r"\4a")(input)?;
    let (input, alpha) = parse_color(input)?;
    Ok((input, StyleOverride::BackAlpha(alpha)))
}

fn parse_an(input: &str) -> IResult<&str, StyleOverride> {
    let (input, _) = tag(r"\an")(input)?;
    let (input, val) = verify(digit1, |v: &str| {
        v.parse::<u8>().map_or(false, |n| (1..=9).contains(&n))
    })(input)?;
    Ok((
        input,
        StyleOverride::Alignment(Alignment::from_str(val).unwrap_or(Alignment::BottomCenter)),
    ))
}
fn parse_a(input: &str) -> IResult<&str, StyleOverride> {
    let (input, _) = tag(r"\a")(input)?;
    let (input, _) = verify(digit1, |v: &str| {
        v.parse::<u8>().map_or(false, |n| (1..=11).contains(&n))
    })(input)?;
    Ok((
        input,
        // NOTE: if use \a with \an will override \an
        StyleOverride::Alignment(Alignment::BottomCenter),
    ))
}

fn parse_pos(input: &str) -> IResult<&str, StyleOverride> {
    let (input, _) = tag(r"\pos")(input)?;
    let (input, (x, y)) = delimited(
        char('('),
        separated_pair(parse_float, char(','), parse_float),
        char(')'),
    )(input)?;
    Ok((input, StyleOverride::Position { x, y }))
}

fn parse_fade(input: &str) -> IResult<&str, StyleOverride> {
    let (input, tag) = alt((tag(r"\fade"), tag(r"\fad")))(input)?;
    if tag == r"\fad" {
        let (input, (time2, time3)) = delimited(
            char('('),
            separated_pair(parse_u32, char(','), parse_u32),
            char(')'),
        )(input)?;
        Ok((
            input,
            StyleOverride::Fade {
                alpha1: 0xFF,
                alpha2: 0,
                alpha3: 0xFF,
                time1: 0,
                time2,
                // https://github.com/libass/libass/blob/7d8e335b094f00c57fb557b01e93b60a17e63434/libass/ass_parse.c#L647-L648
                // FIXME: will be time4 - time3
                time3,
                // FIXME: need from event duration
                time4: 0,
            },
        ))
    } else if tag == r"\fade" {
        let (input, params) =
            delimited(char('('), separated_list1(char(','), parse_u32), char(')'))(input)?;
        if params.len() == 7 {
            Ok((
                input,
                StyleOverride::Fade {
                    alpha1: params[0],
                    alpha2: params[1],
                    alpha3: params[2],
                    time1: params[3],
                    time2: params[4],
                    time3: params[5],
                    time4: params[6],
                },
            ))
        } else {
            Err(nom::Err::Error(nom::error::Error::new(
                input,
                nom::error::ErrorKind::Count,
            )))
        }
    } else {
        Err(nom::Err::Error(nom::error::Error::new(
            input,
            nom::error::ErrorKind::Tag,
        )))
    }
}

fn parse_org(input: &str) -> IResult<&str, StyleOverride> {
    let (input, _) = tag(r"\org")(input)?;
    let (input, (x, y)) = delimited(
        char('('),
        separated_pair(parse_float, char(','), parse_float),
        char(')'),
    )(input)?;
    Ok((input, StyleOverride::Origin { x, y }))
}

// https://github.com/libass/libass/blob/7d8e335b094f00c57fb557b01e93b60a17e63434/libass/ass_parse.c#L670-L728
fn parse_t_style_modifiers(input: &str) -> IResult<&str, StyleModifier> {
    let (output, style_override) = parse_override(input)?;
    // 以上会消耗标签

    match StyleModifier::try_from(style_override) {
        Ok(style_mod) => Ok((output, style_mod)),
        Err(_) => Err(nom::Err::Error(nom::error::Error::new(
            output,
            nom::error::ErrorKind::Fail,
        ))),
    }
}
fn parse_t(input: &str) -> IResult<&str, StyleOverride> {
    let (input, _) = tag(r"\t")(input)?;
    let (input, _) = delimited(char('('), is_not(")"), char(')'))(input)?;
    let (input, list) = separated_list0(char(','), is_not(","))(input)?;

    if list.len() == 1 {
        let (input, style_modifiers) = many0(parse_t_style_modifiers)(input)?;
        Ok((
            input,
            StyleOverride::Transformation {
                start_time: None,
                end_time: None,
                acceleration: None,
                style_modifiers,
            },
        ))
    } else if list.len() == 2 {
        let (input, acceleration) = parse_float(input)?;
        let (input, style_modifiers) = many0(parse_t_style_modifiers)(input)?;
        Ok((
            input,
            StyleOverride::Transformation {
                start_time: None,
                end_time: None,
                acceleration: Some(acceleration),
                style_modifiers,
            },
        ))
    } else if list.len() == 3 {
        let (input, start_time) = parse_u32(input)?;
        let (input, end_time) = parse_u32(input)?;
        let (input, style_modifiers) = many0(parse_t_style_modifiers)(input)?;
        Ok((
            input,
            StyleOverride::Transformation {
                start_time: Some(start_time),
                end_time: Some(end_time),
                acceleration: None,
                style_modifiers,
            },
        ))
    } else if list.len() == 4 {
        let (input, start_time) = parse_u32(input)?;
        let (input, end_time) = parse_u32(input)?;
        let (input, acceleration) = parse_float(input)?;
        let (input, style_modifiers) = many0(parse_t_style_modifiers)(input)?;
        Ok((
            input,
            StyleOverride::Transformation {
                start_time: Some(start_time),
                end_time: Some(end_time),
                acceleration: Some(acceleration),
                style_modifiers,
            },
        ))
    } else {
        Err(nom::Err::Error(nom::error::Error::new(
            input,
            nom::error::ErrorKind::Count,
        )))
    }
}

fn parse_clip(input: &str) -> IResult<&str, StyleOverride> {
    let (input, tag) = alt((tag(r"\clip"), tag(r"\iclip")))(input)?;
    let is_display = tag == r"\clip";
    let (input, params) = delimited(
        char('('),
        separated_list1(char(','), not_line_ending),
        char(')'),
    )(input)?;
    if params.len() == 4 {
        Ok((
            input,
            StyleOverride::ClipRectangle {
                x1: params[0].parse::<u32>().unwrap_or(0),
                y1: params[1].parse::<u32>().unwrap_or(0),
                x2: params[2].parse::<u32>().unwrap_or(0),
                y2: params[3].parse::<u32>().unwrap_or(0),
                is_display,
            },
        ))
    } else {
        let mut scale = 1;
        let mut rest;
        let mut drawing_commands = Vec::new();
        if params.len() == 2 {
            scale = params[0].parse::<u32>().unwrap_or(1);
            (rest, drawing_commands) = parse_drawing_commands(params[1])?;
            debug_assert!(
                rest.is_empty(),
                "Parsing error: Drawing commands weren't fully parsed. Remaining input: '{}'",
                rest
            );
            Ok((
                input,
                StyleOverride::ClipDrawing {
                    scale,
                    drawing_commands,
                    is_display,
                },
            ))
        } else {
            (rest, drawing_commands) = parse_drawing_commands(params[0])?;
            debug_assert!(
                rest.is_empty(),
                "Parsing error: Drawing commands weren't fully parsed. Remaining input: '{}'",
                rest
            );
            Ok((
                input,
                StyleOverride::ClipDrawing {
                    scale,
                    drawing_commands,
                    is_display,
                },
            ))
        }
    }
}

fn parse_c(input: &str) -> IResult<&str, StyleOverride> {
    let (input, _) = alt((tag(r"\c"), tag(r"\1c")))(input)?;
    let (input, color) = parse_color(input)?;
    Ok((input, StyleOverride::PrimaryColor(color)))
}
fn parse_2c(input: &str) -> IResult<&str, StyleOverride> {
    let (input, _) = tag(r"\2c")(input)?;
    let (input, color) = parse_color(input)?;
    Ok((input, StyleOverride::SecondaryColor(color)))
}

fn parse_3c(input: &str) -> IResult<&str, StyleOverride> {
    let (input, _) = tag(r"\3c")(input)?;
    let (input, color) = parse_color(input)?;
    Ok((input, StyleOverride::OutlineColor(color)))
}

fn parse_4c(input: &str) -> IResult<&str, StyleOverride> {
    let (input, _) = tag(r"\4c")(input)?;
    let (input, color) = parse_color(input)?;
    Ok((input, StyleOverride::BackColor(color)))
}

fn parse_r(input: &str) -> IResult<&str, StyleOverride> {
    let (input, _) = tag(r"\r")(input)?;
    if input.starts_with(r"\") {
        Ok((input, StyleOverride::Reset(None)))
    } else {
        let (input, style_name) = take_while1(|c: char| c != '\\' && c != '}')(input)?;
        Ok((input, StyleOverride::Reset(Some(style_name.to_string()))))
    }
}

fn parse_be(input: &str) -> IResult<&str, StyleOverride> {
    let (input, _) = tag(r"\be")(input)?;
    let (input, val) = parse_u32(input)?;
    match val {
        0 => Ok((
            input,
            StyleOverride::BlurEdges(BlurEdgesTypes::NonBlurEdges),
        )),
        1 => Ok((input, StyleOverride::BlurEdges(BlurEdgesTypes::BlurEdges))),
        _ => Ok((input, StyleOverride::BlurEdges(BlurEdgesTypes::Weight(val)))),
    }
}

fn parse_b(input: &str) -> IResult<&str, StyleOverride> {
    let (input, _) = tag(r"\b")(input)?;
    let (input, val) = parse_u32(input)?;
    match val {
        0 => Ok((input, StyleOverride::Bold(BoldType::NonBold))),
        1 => Ok((input, StyleOverride::Bold(BoldType::Bold))),
        _ => Ok((input, StyleOverride::Bold(BoldType::Weight(val)))),
    }
}

fn parse_i(input: &str) -> IResult<&str, StyleOverride> {
    let (input, _) = tag(r"\i")(input)?;
    let (input, val) = parse_u32(input)?;
    match val {
        0 => Ok((input, StyleOverride::Italic(false))),
        1 => Ok((input, StyleOverride::Italic(true))),
        _ => Err(nom::Err::Error(nom::error::Error::new(
            input,
            nom::error::ErrorKind::Verify,
        ))),
    }
}

// TODO: v4++
// fn parse_kt(input: &str) -> IResult<&str, StyleOverride>

fn parse_kf(input: &str) -> IResult<&str, StyleOverride> {
    let (input, _) = alt((tag(r"\kf"), tag(r"\K")))(input)?;
    let (input, duration) = parse_float(input)?;
    Ok((
        input,
        StyleOverride::Karaoke(KaraokeOverrideType::KaraokeKf { duration }),
    ))
}
fn parse_ko(input: &str) -> IResult<&str, StyleOverride> {
    let (input, _) = tag(r"\ko")(input)?;
    let (input, duration) = parse_float(input)?;
    Ok((
        input,
        StyleOverride::Karaoke(KaraokeOverrideType::KaraokeKo { duration }),
    ))
}
fn parse_k(input: &str) -> IResult<&str, StyleOverride> {
    let (input, _) = tag(r"\k")(input)?;
    let (input, duration) = parse_float(input)?;
    Ok((
        input,
        StyleOverride::Karaoke(KaraokeOverrideType::Karaoke { duration }),
    ))
}

fn parse_shad(input: &str) -> IResult<&str, StyleOverride> {
    let (input, _) = tag(r"\shad")(input)?;
    let (input, depth) = parse_float(input)?;
    Ok((input, StyleOverride::Shadow(depth)))
}

fn parse_s(input: &str) -> IResult<&str, StyleOverride> {
    let (input, _) = tag(r"\s")(input)?;
    let (input, is_enable) = parse_u32(input)?;
    match is_enable {
        0 => Ok((input, StyleOverride::Strikeout(false))),
        1 => Ok((input, StyleOverride::Strikeout(true))),
        _ => Err(nom::Err::Error(nom::error::Error::new(
            input,
            nom::error::ErrorKind::Verify,
        ))),
    }
}

fn parse_u(input: &str) -> IResult<&str, StyleOverride> {
    let (input, _) = tag(r"\u")(input)?;
    let (input, is_enable) = parse_u32(input)?;
    match is_enable {
        0 => Ok((input, StyleOverride::Underline(false))),
        1 => Ok((input, StyleOverride::Underline(true))),
        _ => Err(nom::Err::Error(nom::error::Error::new(
            input,
            nom::error::ErrorKind::Verify,
        ))),
    }
}

fn parse_pbo(input: &str) -> IResult<&str, StyleOverride> {
    let (input, _) = tag(r"\pbo")(input)?;
    let (input, val) = parse_float(input)?;
    Ok((input, StyleOverride::DrawingBaselineOffset(val)))
}

// https://github.com/libass/libass/blob/7d8e335b094f00c57fb557b01e93b60a17e63434/libass/ass_parse.c#L901
// https://aegisub.org/zh-cn/docs/latest/ass_tags/#p01---toggle-drawing-mode
pub fn parse_p(input: &str) -> IResult<&str, StyleOverride> {
    let (input, _) = tag(r"\p")(input)?;
    let (input, scale) = parse_u32(input)?;
    Ok((input, StyleOverride::DrawingScale(scale)))
}

fn parse_q(input: &str) -> IResult<&str, StyleOverride> {
    let (input, _) = tag(r"\q")(input)?;
    let (input, val) = parse_u32(input)?;
    Ok((
        input,
        StyleOverride::WrapStyle(WrapStyle::try_from(val).unwrap_or(WrapStyle::Smart)),
    ))
}

// 无视文件内的设置
fn parse_fe(input: &str) -> IResult<&str, StyleOverride> {
    let (input, _) = tag(r"\fe")(input)?;
    let (input, _) = parse_u32(input)?;
    Ok((input, StyleOverride::Unknown))
}

fn consume_tag(input: &str) -> IResult<&str, StyleOverride> {
    // 确保以 \ 开头
    let (input, _) = tag(r"\")(input)?;
    // 消耗无效标签
    let (remaining, consumed) = take_while(|c| c != '\\' && c != '}')(input)?;
    eprintln!("Unknown or invalid tag consumed: \\{}", consumed);
    Ok((remaining, StyleOverride::Unknown))
}

fn parse_override_group1(input: &str) -> IResult<&str, StyleOverride> {
    alt((
        parse_xshad,
        parse_yshad,
        parse_fax,
        parse_fay,
        parse_blur,
        parse_fscx,
        parse_fscy,
        parse_fsc,
        parse_fsp,
        parse_fs,
        parse_bord,
        parse_xbord,
        parse_ybord,
        parse_bold,
    ))(input)
}
fn parse_override_group2(input: &str) -> IResult<&str, StyleOverride> {
    alt((
        parse_move,
        parse_frx,
        parse_fry,
        parse_frz,
        parse_fn,
        parse_all_alpha,
        parse_1a,
        parse_2a,
        parse_3a,
        parse_4a,
        parse_an,
        parse_a,
    ))(input)
}
fn parse_override_group3(input: &str) -> IResult<&str, StyleOverride> {
    alt((
        parse_pos, parse_fade, parse_org, parse_t, parse_clip, parse_c, parse_2c, parse_3c,
        parse_4c, parse_r, parse_be, parse_b, parse_i,
    ))(input)
}
fn parse_override_group4(input: &str) -> IResult<&str, StyleOverride> {
    alt((
        // parse_kt,
        parse_kf,
        parse_ko,
        parse_k,
        parse_shad,
        parse_s,
        parse_u,
        parse_pbo,
        parse_p,
        parse_q,
        parse_fe,
        consume_tag,
    ))(input)
}
/// 演示解析各种覆盖标签（如 \b1、\move(...)、\c&HFF00FF& 等）。
fn parse_override(input: &str) -> IResult<&str, StyleOverride> {
    alt((
        parse_override_group1,
        parse_override_group2,
        parse_override_group3,
        parse_override_group4,
    ))(input)
}

// 解析完整的覆盖语句块（即花括号中的内容），如“{\b1\move(10,20,30,40)\c&HFF00FF&}”。
pub fn parse_multiple_overrides(input: &str) -> IResult<&str, Vec<StyleOverride>> {
    let (output, style_overrides) = delimited(tag(r"{"), take_until("}"), tag(r"}"))(input)?;
    let (_, overrides) = many0(parse_override)(style_overrides)?;

    let mut unique_overrides: Vec<StyleOverride> = Vec::new();
    for override_item in overrides {
        if !matches!(override_item, StyleOverride::Unknown) {
            // 检查是否已存在相同类型的变体
            if !unique_overrides.iter().any(|x| x == &override_item) {
                unique_overrides.push(override_item);
            }
        }
    }

    Ok((output, unique_overrides))
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn test_parse_override() {
        let input = r"\fad(300,100)\fn方正卡通_GBK";
        let except = (
            r"\fn方正卡通_GBK",
            StyleOverride::Fade {
                alpha1: 0xFF,
                alpha2: 0,
                alpha3: 0xFF,
                time1: 0,
                time2: 300,
                time3: 100,
                time4: 0,
            },
        );
        let (rest, style_override) = parse_override(input).unwrap();
        assert_eq!(except, (rest, style_override));
    }
    #[test]
    fn test_parse_multiple_overrides() {
        let input = r"{\fad(300,100)\fn方正卡通_GBK\fs75\frz18.757\c&H2CA600&\pos(500,97)}";
        let except = (
            "",
            vec![
                StyleOverride::Fade {
                    alpha1: 0xFF,
                    alpha2: 0,
                    alpha3: 0xFF,
                    time1: 0,
                    time2: 300,
                    time3: 100,
                    time4: 0,
                },
                StyleOverride::FontName("方正卡通_GBK".to_string()),
                StyleOverride::FontSize(75.0),
                StyleOverride::FontRotationZ(18.757),
                StyleOverride::PrimaryColor(0x2CA600),
                StyleOverride::Position { x: 500.0, y: 97.0 },
            ],
        );
        let output = parse_multiple_overrides(input).unwrap();
        assert_eq!(except, output);
    }
}
