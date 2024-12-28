use nom::{
    bytes::complete::take_while1, character::complete::digit1, combinator::map_res, IResult,
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
