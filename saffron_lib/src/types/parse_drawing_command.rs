use nom::{
    branch::alt,
    character::complete::{char, multispace0, multispace1},
    multi::{many0, many1},
    sequence::{preceded, tuple},
    IResult,
};

use crate::utils::*;

use super::{Alignment, DrawingCommand};

fn parse_point(input: &str) -> IResult<&str, (u32, u32)> {
    // 先解析一个整数，然后至少一个空格，再解析另一个整数
    let (input, x) = preceded(multispace0, parse_u32)(input)?;
    let (input, _) = multispace1(input)?;
    let (input, y) = preceded(multispace0, parse_u32)(input)?;
    Ok((input, (x, y)))
}

fn parse_move_to(input: &str) -> IResult<&str, Vec<DrawingCommand>> {
    let (input, _) = preceded(multispace0, char('m'))(input)?;
    let (input, _) = multispace1(input)?;
    let (input, (x, y)) = parse_point(input)?;
    let cmds = vec![DrawingCommand::MoveTo { x, y }];
    Ok((input, cmds))
}

fn parse_line_to_multiple(input: &str) -> IResult<&str, Vec<DrawingCommand>> {
    let (input, _) = preceded(multispace0, char('l'))(input)?;
    let (input, _) = multispace1(input)?;
    // many1(parse_point) 表示至少要有一对 (x, y)
    let (input, points) = many1(parse_point)(input)?;

    // 依次将每对坐标转换成单独的 DrawingCommand::LineTo
    let cmds = points
        .into_iter()
        .map(|(x, y)| DrawingCommand::LineTo { x, y })
        .collect::<Vec<DrawingCommand>>();

    Ok((input, cmds))
}

fn parse_move_to_no_close(input: &str) -> IResult<&str, Vec<DrawingCommand>> {
    let (input, _) = preceded(multispace0, char('n'))(input)?;
    let (input, _) = multispace1(input)?;
    let (input, (x, y)) = parse_point(input)?;
    let cmds = vec![DrawingCommand::MoveToNoClose { x, y }];
    Ok((input, cmds))
}

fn parse_bezier_curve_to(input: &str) -> IResult<&str, Vec<DrawingCommand>> {
    let (input, _) = preceded(multispace0, char('b'))(input)?;
    let (input, (control_point1, control_point2, end_point)) =
        tuple((parse_point, parse_point, parse_point))(input)?;
    let cmd = vec![DrawingCommand::BezierCurveTo {
        control_point1,
        control_point2,
        end_point,
    }];
    Ok((input, cmd))
}

fn parse_bspline_to(input: &str) -> IResult<&str, Vec<DrawingCommand>> {
    let (input, _) = preceded(multispace0, char('s'))(input)?;
    let (input, points) = many1(parse_point)(input)?;
    if points.len() < 3 {
        return Err(nom::Err::Error(nom::error::Error::new(
            input,
            nom::error::ErrorKind::Count,
        )));
    }

    let cmd = vec![DrawingCommand::BSplineTo { points }];
    Ok((input, cmd))
}

fn parse_extend_bspline_to(input: &str) -> IResult<&str, Vec<DrawingCommand>> {
    let (input, _) = preceded(multispace0, char('p'))(input)?;
    let (input, _) = multispace1(input)?;
    let (input, (x, y)) = parse_point(input)?;
    let cmd = vec![DrawingCommand::ExtendBSplineTo { x, y }];
    Ok((input, cmd))
}

fn parse_close_bspline(input: &str) -> IResult<&str, Vec<DrawingCommand>> {
    let (input, _) = preceded(multispace0, char('c'))(input)?;
    let cmd = vec![DrawingCommand::CloseBSpline];
    Ok((input, cmd))
}

fn parse_single_command(input: &str) -> IResult<&str, Vec<DrawingCommand>> {
    alt((
        parse_move_to,
        parse_move_to_no_close,
        parse_line_to_multiple,
        parse_bezier_curve_to,
        parse_bspline_to,
        parse_extend_bspline_to,
        parse_close_bspline,
    ))(input)
}

pub fn parse_drawing_commands(input: &str) -> IResult<&str, Vec<DrawingCommand>> {
    // 先用 many0(parse_single_command) 得到 Vec<Vec<DrawingCommand>>
    let (input, all_cmds) = many0(parse_single_command)(input)?;
    // 将 Vec<Vec<DrawingCommand>> 扁平化为 Vec<DrawingCommand>
    let flattened = all_cmds.into_iter().flatten().collect();
    Ok((input, flattened))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_parse_drawing_commands() {
        // Test a simple move and line sequence
        let input = "m 0 0 l 100 0 100 100 0 100";
        let result = parse_drawing_commands(input);
        assert!(result.is_ok());
        let (_, commands) = result.unwrap();
        assert_eq!(
            commands,
            vec![
                DrawingCommand::MoveTo { x: 0, y: 0 },
                DrawingCommand::LineTo { x: 100, y: 0 },
                DrawingCommand::LineTo { x: 100, y: 100 },
                DrawingCommand::LineTo { x: 0, y: 100 },
            ]
        );

        // Test a combination of move without close, then a cubic Bézier
        let input2 = "n 10 20 b 30 40 50 60 70 80";
        let result2 = parse_drawing_commands(input2);
        assert!(result2.is_ok());
        let (_, commands2) = result2.unwrap();
        assert_eq!(
            commands2,
            vec![
                DrawingCommand::MoveToNoClose { x: 10, y: 20 },
                DrawingCommand::BezierCurveTo {
                    control_point1: (30, 40),
                    control_point2: (50, 60),
                    end_point: (70, 80),
                },
            ]
        );

        // Test b-spline and its extension
        let input3 = "m 0 0 s 100 0 100 100 0 100 p 50 150 c";
        let result3 = parse_drawing_commands(input3);
        assert!(result3.is_ok());
        let (_, commands3) = result3.unwrap();
        assert_eq!(
            commands3,
            vec![
                DrawingCommand::MoveTo { x: 0, y: 0 },
                DrawingCommand::BSplineTo {
                    points: vec![(100, 0), (100, 100), (0, 100)],
                },
                DrawingCommand::ExtendBSplineTo { x: 50, y: 150 },
                DrawingCommand::CloseBSpline,
            ]
        );
    }
}
