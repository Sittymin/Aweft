use nom::{
    branch::alt,
    bytes::complete::{tag, take_while1},
    character::complete::{char, digit1},
    combinator::{map, map_res, opt},
    sequence::{delimited, preceded, terminated, tuple},
    IResult,
};

use super::types::*;

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

fn parse_i32(input: &str) -> IResult<&str, i32> {
    map_res(digit1, |s: &str| s.parse::<i32>())(input)
}

fn parse_u32(input: &str) -> IResult<&str, u32> {
    map_res(digit1, |s: &str| s.parse::<u32>())(input)
}

fn parse_float(input: &str) -> IResult<&str, f64> {
    map_res(
        recognize_float, // 下方自定义的识别浮点数函数
        |num_str: &str| num_str.parse::<f64>(),
    )(input)
}
fn recognize_float(input: &str) -> IResult<&str, &str> {
    // libass 使用 C 标准库支持科学计数法
    let float_chars = "0123456789+-.eE";
    take_while1(move |c: char| float_chars.contains(c))(input)
}

fn parse_color(input: &str) -> IResult<&str, u32> {
    // 简化的示例：期望字符串形如 &HFF00FF&
    // 例如原始 ASS 中颜色顺序可能与此不同，请根据实际需求调整
    let (input, _) = tag("&H")(input)?;
    let (input, hex_digits) = take_while1(|c: char| c.is_ascii_hexdigit())(input)?;
    let (input, _) = char('&')(input)?;
    // 将提取到的十六进制字符串转为 u32
    let value = u32::from_str_radix(hex_digits, 16).unwrap_or(0);
    Ok((input, value))
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

fn parse_iclip_rectangle(input: &str) -> IResult<&str, StyleOverride> {
    let (input, _) = tag(r"\iclip")(input)?;
    let (input, (x1, y1, x2, y2)) = delimited(
        char('('),
        tuple((
            terminated(parse_u32, char(',')),
            terminated(parse_u32, char(',')),
            terminated(parse_u32, char(',')),
            terminated(parse_u32, char(',')),
        )),
        char(')'),
    )(input)?;
    Ok((
        input,
        StyleOverride::ClipRectangle {
            x1,
            y1,
            x2,
            y2,
            is_display: false,
        },
    ))
}

// TODO: start at https://github.com/libass/libass/blob/7d8e335b094f00c57fb557b01e93b60a17e63434/libass/ass_parse.c#L425
// fn parse_xshad(input: &str) -> IResult<&str, StyleOverride> {
//     let (input, _) = tag(r"\xshad")(input)?;
//     let (input, val) = parse_float(input)?;
//     Ok((input, StyleOverride::XShadow(val)))
// }

/// 演示解析“\move(...)”标签，如“\move(10,20,30,40,500,1000)”。
fn parse_move_tag(input: &str) -> IResult<&str, StyleOverride> {
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

/// 演示解析各种覆盖标签（如 \b1、\move(...)、\c&HFF00FF& 等）。
fn parse_override(input: &str) -> IResult<&str, StyleOverride> {
    alt((
        parse_bold,
        parse_move_tag,
        // 如需解析 \italic、\fade 等，可以在此处继续扩展
        map(
            preceded(tag("\\c"), parse_color),
            StyleOverride::PrimaryColor,
        ),
    ))(input)
}

// 解析完整的覆盖语句块（即花括号中的内容），如“{\b1\move(10,20,30,40)\c&HFF00FF&}”。
// 在这里假设我们只传入花括号内的字符串，不包含外侧 "{" 与 "}"。
// fn parse_override_block(input: &str) -> IResult<&str, Vec<StyleOverride>> {
//     // separated_list0(multispace0, parse_override) 的含义是：用空白分隔符分割 parse_override 多次
//     separated_list0(multispace0, parse_override)(input)
// }
