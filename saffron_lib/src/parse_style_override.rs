use std::{mem::discriminant, str::FromStr};

use nom::{
    branch::alt,
    bytes::complete::{tag, take_until, take_until1, take_while, take_while1, take_while_m_n},
    character::complete::{char, digit1, not_line_ending},
    combinator::{map, map_res, opt, verify},
    multi::separated_list1,
    sequence::{delimited, preceded, separated_pair, terminated, tuple},
    IResult,
};
use parse_drawing_command::parse_drawing_commands;

use super::types::*;
use crate::utils::*;

/* -----------------------------------------------------------------------
   下面简要说明了我们使用的 nom 函数：
   --------------------------------------
   1) tag("..."): 匹配精确字符串（如 "\\b"）。
   2) alt((...)): 尝试多个解析器，直到其中一个成功为止。
   3) map(parser, |val| ...): 将解析器结果映射到另一个值或类型。
   4) delimited(...): 解析器，用来匹配特定的“开头-内容-结尾”模式，例如括号 ( ... )。
   5) tuple(...)、separated_pair(...)：组合多个解析器，依次解析不同项目。
   6) opt(...)：将一个解析器变为可选解析（可匹配一次或不匹配）。
   7) separated_list0(...)：重复同一个解析器，并用指定分隔符（比如空白符）分隔。
   8) take_while1(...)：匹配满足特定条件的字符序列（至少一个字符）。
   9) map_res(...)：对解析结果执行转换，并可以捕捉错误（Result）。
   10) preceded(tag("\\c"), parse_color)：先匹配字符串“\\c”，然后解析颜色。
   11) char('(')、digit1 等是 nom 内置的对单个字符或数字字符串的解析器。
   IResult<...> 泛型第一个参数是剩余部分，第二个参数是匹配部分
------------------------------------------------------------------------- */

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
// fn parse_fsc(input: &str) -> IResult<&str, StyleOverride> {
//     let (input, _) = tag(r"\fsc")(input)?;
// }

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

fn parse_xbold(input: &str) -> IResult<&str, StyleOverride> {
    let (input, _) = tag(r"\xbord")(input)?;
    let (input, val) = parse_float(input)?;
    Ok((input, StyleOverride::XBold(val)))
}
fn parse_ybold(input: &str) -> IResult<&str, StyleOverride> {
    let (input, _) = tag(r"\ybold")(input)?;
    let (input, val) = parse_float(input)?;
    Ok((input, StyleOverride::YBold(val)))
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
fn parse_primary_alpha(input: &str) -> IResult<&str, StyleOverride> {
    let (input, _) = tag(r"\1a")(input)?;
    let (input, alpha) = parse_color(input)?;
    Ok((input, StyleOverride::PrimaryAlpha(alpha)))
}
fn parse_secondary_alpha(input: &str) -> IResult<&str, StyleOverride> {
    let (input, _) = tag(r"\2a")(input)?;
    let (input, alpha) = parse_color(input)?;
    Ok((input, StyleOverride::SecondaryAlpha(alpha)))
}
fn parse_outline_alpha(input: &str) -> IResult<&str, StyleOverride> {
    let (input, _) = tag(r"\3a")(input)?;
    let (input, alpha) = parse_color(input)?;
    Ok((input, StyleOverride::OutlineAlpha(alpha)))
}
fn parse_back_alpha(input: &str) -> IResult<&str, StyleOverride> {
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

// TODO: 需要另外再次实现类似解析覆盖标签的解析
// 另外需要查看libass看看哪些确实是应该在 StyleModifier 中
// https://github.com/libass/libass/blob/7d8e335b094f00c57fb557b01e93b60a17e63434/libass/ass_parse.c#L670-L728
// fn parse_t(input: &str) -> IResult<&str, StyleOverride> {
//     let (input, _) = tag(r"\t")(input)?;
//     let (input, val) = parse_style_modifier_tag(input)?;
//     Ok((input, StyleOverride::Transformation{
//         start_time: None,
//         end_time: None,
//         acceleration: None,
//         style_modifiers: StyleModifier::PrimaryColor(0)
//     }))
// }

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
// fn parse_(input: &str) -> IResult<&str, StyleOverride> {
//     let (input, _) = tag(r"\")(input)?;
//     let (input, val) = parse_float(input)?;
//     Ok((input, StyleOverride::))
// }

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

// v4++
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
    Ok((input, StyleOverride::BaselineOffset(val)))
}

// TODO: 应该在特效标签区外部，单独识别，移出覆盖标签区
// https://github.com/libass/libass/blob/7d8e335b094f00c57fb557b01e93b60a17e63434/libass/ass_parse.c#L901
// https://aegisub.org/zh-cn/docs/latest/ass_tags/#p01---toggle-drawing-mode
// fn parse_p(input: &str) -> IResult<&str, StyleOverride> {
//     let (input, _) = tag(r"\p")(input)?;
//     let (input, val) = parse_u32(input)?;
//     Ok((input, StyleOverride::Drawing { scale }))
// }

fn consume_tag(input: &str) -> IResult<&str, StyleOverride> {
    // 确保以 \ 开头
    let (input, _) = tag(r"\")(input)?;
    // 消耗无效标签
    let (remaining, consumed) = take_while(|c| c != '\\' && c != '}')(input)?;
    eprintln!("Unknown or invalid tag consumed: \\{}", consumed);
    Ok((remaining, StyleOverride::Unknown))
}

/// 演示解析各种覆盖标签（如 \b1、\move(...)、\c&HFF00FF& 等）。
fn parse_override(input: &str) -> IResult<&str, StyleOverride> {
    alt((parse_bold, parse_move, parse_c, consume_tag))(input)
}

// 解析完整的覆盖语句块（即花括号中的内容），如“{\b1\move(10,20,30,40)\c&HFF00FF&}”。
// 在这里假设我们只传入花括号内的字符串，不包含外侧 "{" 与 "}"。
// fn parse_override_block(input: &str) -> IResult<&str, Vec<StyleOverride>> {
//     // separated_list0(multispace0, parse_override) 的含义是：用空白分隔符分割 parse_override 多次
//     separated_list0(multispace0, parse_override)(input)
// }
