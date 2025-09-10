use nom::{
    branch::alt,
    character::complete::{char, digit1, multispace0, multispace1},
    combinator::map_res,
    multi::{many0, many1},
    number::complete::double,
    sequence::{preceded, separated_pair},
    IResult, Parser,
};

use super::{drawing::DrawingCommand, PointF64};

fn parse_point_u32(input: &str) -> IResult<&str, (u32, u32)> {
    separated_pair(
        map_res(digit1, |s: &str| s.parse::<u32>()),
        multispace1,
        map_res(digit1, |s: &str| s.parse::<u32>()),
    )
    .parse(input)
}

fn parse_move_to(input: &str) -> IResult<&str, Vec<DrawingCommand>> {
    let (input, _) = preceded(multispace0, char('m')).parse(input)?;
    let (input, _) = multispace1(input)?;
    let (input, (x, y)) = separated_pair(double, multispace1, double).parse(input)?;
    let cmds = vec![DrawingCommand::MoveTo(PointF64 { x, y })];
    Ok((input, cmds))
}

fn parse_line_to_multiple(input: &str) -> IResult<&str, Vec<DrawingCommand>> {
    let (input, _) = preceded(multispace0, char('l')).parse(input)?;
    let (input, _) = multispace1(input)?;
    // many1(parse_point) 表示至少要有一对 (x, y)。
    // 每对之间可能存在空白，需要在每对前消费可选空白。
    let (input, points) =
        many1(preceded(multispace0, separated_pair(double, multispace1, double))).parse(input)?;

    // 依次将每对坐标转换成单独的 DrawingCommand::LineTo
    let cmds = points
        .into_iter()
        .map(|(x, y)| DrawingCommand::LineTo(PointF64 { x, y }))
        .collect::<Vec<DrawingCommand>>();

    Ok((input, cmds))
}

fn parse_move_to_no_close(input: &str) -> IResult<&str, Vec<DrawingCommand>> {
    let (input, _) = preceded(multispace0, char('n')).parse(input)?;
    let (input, _) = multispace1(input)?;
    let (input, (x, y)) = separated_pair(double, multispace1, double).parse(input)?;
    let cmds = vec![DrawingCommand::MoveToNoClose(PointF64 { x, y })];
    Ok((input, cmds))
}

fn parse_bezier_curve_to(input: &str) -> IResult<&str, Vec<DrawingCommand>> {
    let (input, _) = preceded(multispace0, char('b')).parse(input)?;
    let (input, _) = multispace1(input)?;
    let (input, (control_point1, control_point2, end_point)) = (
        preceded(multispace0, separated_pair(double, multispace1, double)),
        preceded(multispace0, separated_pair(double, multispace1, double)),
        preceded(multispace0, separated_pair(double, multispace1, double)),
    )
        .parse(input)?;
    let cmd = vec![DrawingCommand::BezierCurveTo {
        control_point1: control_point1.into(),
        control_point2: control_point2.into(),
        end_point: end_point.into(),
    }];
    Ok((input, cmd))
}

fn parse_bspline_to(input: &str) -> IResult<&str, Vec<DrawingCommand>> {
    let (input, _) = preceded(multispace0, char('s')).parse(input)?;
    let (input, _) = multispace1(input)?;
    let (input, points) =
        many1(preceded(multispace0, separated_pair(double, multispace1, double))).parse(input)?;
    if points.len() < 3 {
        return Err(nom::Err::Error(nom::error::Error::new(
            input,
            nom::error::ErrorKind::Count,
        )));
    }

    let cmd = vec![DrawingCommand::BSplineTo {
        points: points.into_iter().map(PointF64::from).collect(),
    }];
    Ok((input, cmd))
}

fn parse_extend_bspline_to(input: &str) -> IResult<&str, Vec<DrawingCommand>> {
    let (input, _) = preceded(multispace0, char('p')).parse(input)?;
    let (input, _) = multispace1(input)?;
    let (input, (x, y)) = separated_pair(double, multispace1, double).parse(input)?;
    let cmd = vec![DrawingCommand::ExtendBSplineTo(PointF64 { x, y })];
    Ok((input, cmd))
}

fn parse_close_bspline(input: &str) -> IResult<&str, Vec<DrawingCommand>> {
    let (input, _) = preceded(multispace0, char('c')).parse(input)?;
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
    ))
    .parse(input)
}

pub fn parse_drawing_commands(input: &str) -> IResult<&str, Vec<DrawingCommand>> {
    let (input, all_cmds) = many0(parse_single_command).parse(input)?;
    // Vec<Vec<DrawingCommand>> -> Vec<DrawingCommand>
    let mut result = Vec::new();
    let mut current_bspline: Option<Vec<PointF64>> = None;
    let mut spline_start: Option<Vec<PointF64>> = None;
    for cmd in all_cmds.into_iter().flatten() {
        match cmd {
            DrawingCommand::BSplineTo { points } => {
                // start new b-spline
                if let Some(points) = current_bspline.take() {
                    result.push(DrawingCommand::BSplineTo {
                        points: points.into_iter().map(PointF64::from).collect(),
                    });
                }
                // add 3 points
                // https://github.com/libass/libass/blob/4d6e9ef3077fb279cab051cc00b216c5f08efb34/libass/ass_drawing.c#L235-L242
                spline_start = Some(points[..3].to_vec());
                current_bspline = Some(points);
            }
            DrawingCommand::ExtendBSplineTo(p) => {
                if let Some(ref mut points) = current_bspline {
                    points.push(p);
                }
            }
            DrawingCommand::CloseBSpline => {
                if let Some(ref mut points) = current_bspline {
                    if let Some(start_points) = &spline_start {
                        points.extend_from_slice(start_points);
                    }
                }
            }
            other => {
                result.push(other);
            }
        }
    }

    // last b-spline
    if let Some(points) = current_bspline {
        result.push(DrawingCommand::BSplineTo { points });
    }

    Ok((input, result))
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
                DrawingCommand::MoveTo(PointF64 { x: 0.0, y: 0.0 }),
                DrawingCommand::LineTo(PointF64 { x: 100.0, y: 0.0 }),
                DrawingCommand::LineTo(PointF64 { x: 100.0, y: 100.0 }),
                DrawingCommand::LineTo(PointF64 { x: 0.0, y: 100.0 }),
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
                DrawingCommand::MoveToNoClose(PointF64 { x: 10.0, y: 20.0 }),
                DrawingCommand::BezierCurveTo {
                    control_point1: PointF64 { x: 30.0, y: 40.0 },
                    control_point2: PointF64 { x: 50.0, y: 60.0 },
                    end_point: PointF64 { x: 70.0, y: 80.0 },
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
                DrawingCommand::MoveTo(PointF64 { x: 0.0, y: 0.0 }),
                DrawingCommand::BSplineTo {
                    points: vec![
                        PointF64 { x: 100.0, y: 0.0 },
                        PointF64 { x: 100.0, y: 100.0 },
                        PointF64 { x: 0.0, y: 100.0 },
                        PointF64 { x: 50.0, y: 150.0 },
                        PointF64 { x: 100.0, y: 0.0 },
                        PointF64 { x: 100.0, y: 100.0 },
                        PointF64 { x: 0.0, y: 100.0 }
                    ],
                },
            ]
        );
    }
}
