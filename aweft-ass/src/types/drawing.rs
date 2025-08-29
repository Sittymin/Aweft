// WARN: 渲染时注意
// 在ASS文件当中使用的是浮点数，但是在 libass 当中它会被转换为 26.6 定点(用int来表示小数)
use super::PointF64;

#[derive(Debug, Clone, PartialEq)]
pub enum DrawingCommand {
    // l 100 0 100 100 0 100 is same with l 100 0 l 100 100 l 0 100

    // https://github.com/libass/libass/blob/7d8e335b094f00c57fb557b01e93b60a17e63434/libass/ass_outline.h#L30-L32
    MoveTo(PointF64), // 'm'

    MoveToNoClose(PointF64), // 'n'

    LineTo(PointF64), // 'l'

    BezierCurveTo {
        control_point1: PointF64,
        control_point2: PointF64,
        end_point: PointF64,
    }, // 'b'

    BSplineTo {
        points: Vec<PointF64>, // At least 3 points are included
    }, // 's'

    // WARN: Only used in parse
    ExtendBSplineTo(PointF64), // 'p'

    // WARN: Only used in parse
    CloseBSpline, // 'c'
}
