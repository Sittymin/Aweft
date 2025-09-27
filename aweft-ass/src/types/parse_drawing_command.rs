use log::warn;
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
    let (input, _) = separated_pair(multispace0, char('m'), multispace1).parse(input)?;

    let (input, points) = many1(preceded(
        multispace0,
        separated_pair(double, multispace1, double),
    ))
    .parse(input)?;

    let cmds = points
        .into_iter()
        .map(|(x, y)| DrawingCommand::MoveTo(PointF64 { x, y }))
        .collect();
    Ok((input, cmds))
}

fn parse_line_to(input: &str) -> IResult<&str, Vec<DrawingCommand>> {
    let (input, _) = separated_pair(multispace0, char('l'), multispace1).parse(input)?;

    let (input, points) = many1(preceded(
        multispace0,
        separated_pair(double, multispace1, double),
    ))
    .parse(input)?;

    let cmds = points
        .into_iter()
        .map(|(x, y)| DrawingCommand::LineTo(PointF64 { x, y }))
        .collect::<Vec<DrawingCommand>>();
    Ok((input, cmds))
}

fn parse_move_to_no_close(input: &str) -> IResult<&str, Vec<DrawingCommand>> {
    let (input, _) = separated_pair(multispace0, char('n'), multispace1).parse(input)?;
    let (input, points) = many1(preceded(
        multispace0,
        separated_pair(double, multispace1, double),
    ))
    .parse(input)?;
    let cmds = points
        .into_iter()
        .map(|(x, y)| DrawingCommand::MoveToNoClose(PointF64 { x, y }))
        .collect::<Vec<DrawingCommand>>();
    Ok((input, cmds))
}

fn parse_bezier_curve_to(input: &str) -> IResult<&str, Vec<DrawingCommand>> {
    let (input, _) = separated_pair(multispace0, char('b'), multispace1).parse(input)?;

    let (input, bezier_curve_points) = many1((
        preceded(multispace0, separated_pair(double, multispace1, double)),
        preceded(multispace0, separated_pair(double, multispace1, double)),
        preceded(multispace0, separated_pair(double, multispace1, double)),
    ))
    .parse(input)?;

    let cmds = bezier_curve_points
        .into_iter()
        .map(
            |(control_point1, control_point2, end_point)| DrawingCommand::BezierCurveTo {
                control_point1: control_point1.into(),
                control_point2: control_point2.into(),
                end_point: end_point.into(),
            },
        )
        .collect();
    Ok((input, cmds))
}

fn parse_bspline_to(input: &str) -> IResult<&str, Vec<DrawingCommand>> {
    let (input, _) = separated_pair(multispace0, char('s'), multispace1).parse(input)?;

    // 解析3个点用于BSplineTo，然后解析可能的额外点作为ExtendBSplineTo
    let (input, point1) =
        preceded(multispace0, separated_pair(double, multispace1, double)).parse(input)?;
    let (input, point2) =
        preceded(multispace0, separated_pair(double, multispace1, double)).parse(input)?;
    let (input, point3) =
        preceded(multispace0, separated_pair(double, multispace1, double)).parse(input)?;

    let mut cmds = vec![DrawingCommand::BSplineTo {
        points: vec![point1.into(), point2.into(), point3.into()],
    }];

    // 解析可能的额外点作为ExtendBSplineTo
    let (input, extra_points) = many0(preceded(
        multispace0,
        separated_pair(double, multispace1, double),
    ))
    .parse(input)?;

    for point in extra_points {
        cmds.push(DrawingCommand::ExtendBSplineTo(point.into()));
    }

    Ok((input, cmds))
}

fn parse_extend_bspline_to(input: &str) -> IResult<&str, Vec<DrawingCommand>> {
    let (input, _) = separated_pair(multispace0, char('p'), multispace1).parse(input)?;

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
        parse_line_to,
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
                // 开始新的b-spline，严格只取前3个点
                // https://github.com/libass/libass/blob/4d6e9ef3077fb279cab051cc00b216c5f08efb34/libass/ass_drawing.c#L235-L242
                if points.len() >= 3 {
                    spline_start = Some(points[..3].to_vec());
                    current_bspline = Some(points);
                } else {
                    // 如果点数不足3个，直接推入结果（虽然这在理论上不应该发生）
                    unreachable!("b-spline 不足三个点")
                }
            }
            DrawingCommand::ExtendBSplineTo(point) => {
                if let Some(ref mut points) = current_bspline {
                    points.push(point);
                } else {
                    // 如果没有活动的b-spline，忽略此命令（符合libass行为）
                    warn!("扩展 b-spline 时没有活动的 b-spline")
                }
            }
            DrawingCommand::CloseBSpline => {
                if let Some(ref mut points) = current_bspline {
                    if let Some(ref start_points) = spline_start {
                        points.extend_from_slice(start_points);
                    }
                }
                // 注意：不在这里完成 b-spline，等到下一个命令或结束时再将完整的 b-spline 加到命令 Vec 当中
            }
            other => {
                // 遇到非b-spline命令时，完成当前的b-spline
                if let Some(bspline_points) = current_bspline.take() {
                    result.push(DrawingCommand::BSplineTo {
                        points: bspline_points,
                    });
                    spline_start = None;
                }
                result.push(other);
            }
        }
    }

    // 处理最后的b-spline
    if let Some(points) = current_bspline {
        result.push(DrawingCommand::BSplineTo { points });
    }

    Ok((input, result))
}

#[cfg(test)]
mod tests {
    use super::*;
    use proptest::prelude::*;

    proptest! {
        #[test]
        fn parse_move_to_and_line_to_and_move_to_no_close_correctly(
            values in (1usize..=100)
            .prop_map(|k| k * 2)
            .prop_flat_map(|len| {
                prop::collection::vec(any::<f64>(), len..=len)
            })
        ) {
            let formatted_string = values
                .iter()
                .map(|n| n.to_string())
                .collect::<Vec<String>>()
                .join(" ");
            let move_to_s = format!("m {formatted_string}");
            let line_to_s = format!("l {formatted_string}");
            let move_to_no_close_s = format!("n {formatted_string}");
            let (input_move_to, result_move_to) = parse_move_to(&move_to_s)?;
            let (input_line_to, result_line_to) = parse_line_to(&line_to_s)?;
            let (input_move_to_no_close, result_move_to_no_close) = parse_move_to_no_close(&move_to_no_close_s)?;

            prop_assert_eq!(input_move_to, "");
            prop_assert_eq!(input_line_to, "");
            prop_assert_eq!(input_move_to_no_close, "");

            let reconstructed_move_to: Vec<f64> = result_move_to
                .into_iter()
                .flat_map(|cmd| {
                    if let DrawingCommand::MoveTo(PointF64 { x, y }) = cmd {
                        vec![x, y]
                    } else {
                        vec![]
                    }
                })
                .collect();
            let reconstructed_line_to: Vec<f64> = result_line_to
                .into_iter()
                .flat_map(|cmd| {
                    if let DrawingCommand::LineTo(PointF64 { x, y }) = cmd {
                        vec![x, y]
                    } else {
                        vec![]
                    }
                })
                .collect();
            let reconstructed_move_to_no_close: Vec<f64> = result_move_to_no_close
                .into_iter()
                .flat_map(|cmd| {
                    if let DrawingCommand::MoveToNoClose(PointF64 { x, y }) = cmd {
                        vec![x, y]
                    } else {
                        vec![]
                    }
                })
                .collect();

            prop_assert_eq!(&reconstructed_move_to, &values);
            prop_assert_eq!(&reconstructed_line_to, &values);
            prop_assert_eq!(&reconstructed_move_to_no_close, &values);
        }
        #[test]
        fn parse_bezier_curve_to_correctly(
            values in (1usize..=100) // 生成一个乘数 `k`，例如 k = 4
            .prop_map(|k| k * 6) // 计算出最终的长度 `len`，例如 len = 4 * 3 = 12
            .prop_flat_map(|len| {
                // 这个策略会生成一个长度恰好为 `len` (12) 的 Vec
                prop::collection::vec(any::<f64>(), len..=len)
            })
        ) {
            let formatted_string = values
                .iter()
                .map(|n| n.to_string())
                .collect::<Vec<String>>()
                .join(" ");
            let s = format!("b {formatted_string}");
            let (input, result) = parse_bezier_curve_to(&s)?;
            prop_assert_eq!(input, "");

            let reconstructed: Vec<f64> = result
                .into_iter()
                .flat_map(|cmd| {
                    if let DrawingCommand::BezierCurveTo { control_point1, control_point2, end_point } = cmd {
                        vec![control_point1.x, control_point1.y,
                             control_point2.x, control_point2.y,
                             end_point.x, end_point.y]
                    } else {
                        vec![]
                    }
                })
                .collect();

            prop_assert_eq!(reconstructed, values);
        }

        #[test]
        fn parse_bspline_to_correctly(
            initial_points in prop::collection::vec(any::<f64>(), 6..=6), // 恰好3个点 = 6个数字
            extend_points in prop::collection::vec(any::<f64>(), 0..=20)  // 0-10个额外点
                .prop_filter("Must be even length", |v| v.len() % 2 == 0)
        ) {
            let mut all_values = initial_points.clone();
            all_values.extend(extend_points.clone());

            let formatted_string = all_values
                .iter()
                .map(|n| n.to_string())
                .collect::<Vec<String>>()
                .join(" ");
            let s = format!("s {formatted_string}");
            let (input, result) = parse_bspline_to(&s)?;

            prop_assert_eq!(input, "");

            // 应该生成1个BSplineTo + N个ExtendBSplineTo
            let expected_extend_count = extend_points.len() / 2;
            prop_assert_eq!(result.len(), 1 + expected_extend_count);

            // 第一个应该是BSplineTo，包含前3个点
            if let Some(DrawingCommand::BSplineTo { points }) = result.first() {
                prop_assert_eq!(points.len(), 3);
                // 验证前3个点的数据
                for (i, point) in points.iter().enumerate() {
                    prop_assert_eq!(point.x, initial_points[i * 2]);
                    prop_assert_eq!(point.y, initial_points[i * 2 + 1]);
                }
            } else {
                prop_assert!(false, "第一个命令应该是BSplineTo");
            }

            // 其余应该是ExtendBSplineTo
            for (i, cmd) in result.iter().skip(1).enumerate() {
                if let DrawingCommand::ExtendBSplineTo(point) = cmd {
                    prop_assert_eq!(point.x, extend_points[i * 2]);
                    prop_assert_eq!(point.y, extend_points[i * 2 + 1]);
                } else {
                    prop_assert!(false, "扩展命令应该是ExtendBSplineTo");
                }
            }
        }

        #[test]
        fn parse_extend_bspline_to_correctly(
            values in prop::collection::vec(any::<f64>(), 2..=2) // 恰好1个点 = 2个数字
        ) {
            let formatted_string = values
                .iter()
                .map(|n| n.to_string())
                .collect::<Vec<String>>()
                .join(" ");
            let s = format!("p {formatted_string}");
            let (input, result) = parse_extend_bspline_to(&s)?;

            prop_assert_eq!(input, "");
            prop_assert_eq!(result.len(), 1);

            if let Some(DrawingCommand::ExtendBSplineTo(point)) = result.first() {
                prop_assert_eq!(point.x, values[0]);
                prop_assert_eq!(point.y, values[1]);
            } else {
                prop_assert!(false, "应该是ExtendBSplineTo命令");
            }
        }

        #[test]
        fn parse_bspline_sequence_correctly(
            initial_points in prop::collection::vec(any::<f64>(), 6..=6), // 3个点
            extend_points in prop::collection::vec(
                prop::collection::vec(any::<f64>(), 2..=2), // 每个扩展点2个坐标
                0..=10 // 0-10个扩展点
            )
        ) {
            // 构建完整的B-Spline序列：s + 多个p
            let mut command_parts = vec![];

            // 添加初始s命令
            let initial_str = initial_points
                .iter()
                .map(|n| n.to_string())
                .collect::<Vec<String>>()
                .join(" ");
            command_parts.push(format!("s {}", initial_str));

            // 添加p命令
            for point_coords in &extend_points {
                let point_str = point_coords
                    .iter()
                    .map(|n| n.to_string())
                    .collect::<Vec<String>>()
                    .join(" ");
                command_parts.push(format!("p {}", point_str));
            }

            let full_command = command_parts.join(" ");
            let (input, result) = parse_drawing_commands(&full_command)?;

            prop_assert_eq!(input, "");
            prop_assert_eq!(result.len(), 1); // 应该合并成一个BSplineTo

            if let Some(DrawingCommand::BSplineTo { points }) = result.first() {
                let expected_len = 3 + extend_points.len(); // 3个初始点 + N个扩展点
                prop_assert_eq!(points.len(), expected_len);

                // 验证初始3个点
                for (i, point) in points.iter().take(3).enumerate() {
                    prop_assert_eq!(point.x, initial_points[i * 2]);
                    prop_assert_eq!(point.y, initial_points[i * 2 + 1]);
                }

                // 验证扩展点
                for (i, point) in points.iter().skip(3).enumerate() {
                    prop_assert_eq!(point.x, extend_points[i][0]);
                    prop_assert_eq!(point.y, extend_points[i][1]);
                }
            } else {
                prop_assert!(false, "应该是BSplineTo命令");
            }
        }

    }
}
