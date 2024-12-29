use nom::{
    bytes::complete::{tag, take_while1},
    character::complete::{char, digit1},
    combinator::map_res,
    IResult,
};

pub fn parse_i32(input: &str) -> IResult<&str, i32> {
    map_res(digit1, |s: &str| s.parse::<i32>())(input)
}

pub fn parse_u32(input: &str) -> IResult<&str, u32> {
    map_res(digit1, |s: &str| s.parse::<u32>())(input)
}

pub fn parse_float(input: &str) -> IResult<&str, f64> {
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

pub fn parse_color(input: &str) -> IResult<&str, u32> {
    // 简化的示例：期望字符串形如 &HFF00FF&
    // 例如原始 ASS 中颜色顺序可能与此不同，请根据实际需求调整
    let (input, _) = tag("&H")(input)?;
    let (input, hex_digits) = take_while1(|c: char| c.is_ascii_hexdigit())(input)?;
    let (input, _) = char('&')(input)?;
    // 将提取到的十六进制字符串转为 u32
    let value = u32::from_str_radix(hex_digits, 16).unwrap_or(0);
    Ok((input, value))
}
