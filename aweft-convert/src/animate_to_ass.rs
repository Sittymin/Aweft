use chrono::TimeDelta;
use palette::Srgba;

use aweft_ass::types::tags::{AnimatedStyleTag, BlurEdgesTypes, StyleTag};
use aweft_ass::types::{BoldType, PointF64, PointI32};
use aweft_ass::AssColorRgb;
use aweft_model::animation::action::ActionCurve;
use aweft_model::animation::anim_data::PropertyPath;
use aweft_model::animation::fcurve::{evaluate_fcurve, FCurve};
use aweft_model::animation::AnimationLibrary;
use aweft_model::node_graph::style::StyleResult;

mod convert;
pub use convert::convert_anim_data;
mod fit;

use fit::{chord_error, fit_color_segment, fit_scalar_segment, Acceleration};

// ── 拟合 / 分段精度旋钮 ───────────────────────────────────────────────────
// 关键帧区间（Hold/Linear/Bezier）是曲线内容的真实单位：区间内部的极值来自
// 单个区间内的插值，因此范围扫描给每个区间独立的采样预算，而不是把固定的
// 总量摊到原始时间跨度上。总工作量仍有限制。
const RANGE_SAMPLES_PER_INTERVAL: usize = 256;
const RANGE_SAMPLES_MAX: usize = 100_000;
/// 用于评估拟合段相对源曲线误差的采样数。
// 时间拆分数比较误差值。DP 的每个候选段都会用这个预算把段内每个动画属性求一遍，
// 候选总数是格点的平方量级，因此这里是主要的乘数。64 个点在「每个关键帧区间
// 至少 16 个采样」的形状下仍能抓住极值，峰值低估约 0.2%，远小于 tolerance。
const FIT_SAMPLES: usize = 64;
/// minimax 优化器的粗指数网格。
// 幂函数区间分割数（尝试的幂函数）。网格在 log(0.001)..log(1000) 上均分，只负责
// 挑对盆地，随后的黄金分割精化决定精度，所以每倍量级十个点足够。
const MINIMAX_GRID: usize = 32;

// ── 属性 → ASS 标签映射表（固定表） ───────────────────────────────────────
type ColorGroupFn = fn(AssColorRgb) -> AnimatedStyleTag;
type ScalarGroupFn = fn(f64) -> AnimatedStyleTag;
type AlphaGroupFn = fn(u8) -> AnimatedStyleTag;

static COLOR_GROUPS: &[(&[PropertyPath], ColorGroupFn)] = &[
    (
        &[
            PropertyPath::ColorPrimaryR,
            PropertyPath::ColorPrimaryG,
            PropertyPath::ColorPrimaryB,
        ],
        AnimatedStyleTag::PrimaryColor,
    ),
    (
        &[
            PropertyPath::ColorSecondaryR,
            PropertyPath::ColorSecondaryG,
            PropertyPath::ColorSecondaryB,
        ],
        AnimatedStyleTag::SecondaryColor,
    ),
    (
        &[
            PropertyPath::ColorOutlineR,
            PropertyPath::ColorOutlineG,
            PropertyPath::ColorOutlineB,
        ],
        AnimatedStyleTag::OutlineColor,
    ),
    (
        &[
            PropertyPath::ColorBackR,
            PropertyPath::ColorBackG,
            PropertyPath::ColorBackB,
        ],
        AnimatedStyleTag::BackColor,
    ),
];

static SCALAR_MAP: &[(PropertyPath, ScalarGroupFn)] = &[
    (PropertyPath::FontSize, AnimatedStyleTag::FontSize),
    (PropertyPath::FontRotationX, AnimatedStyleTag::FontRotationX),
    (PropertyPath::FontRotationY, AnimatedStyleTag::FontRotationY),
    (PropertyPath::FontRotationZ, AnimatedStyleTag::FontRotationZ),
    (PropertyPath::FactorX, AnimatedStyleTag::FactorX),
    (PropertyPath::FactorY, AnimatedStyleTag::FactorY),
    (PropertyPath::Blur, AnimatedStyleTag::Blur),
    (PropertyPath::BorderX, AnimatedStyleTag::XBord),
    (PropertyPath::BorderY, AnimatedStyleTag::YBord),
    (PropertyPath::ShadowX, AnimatedStyleTag::XShadow),
    (PropertyPath::ShadowY, AnimatedStyleTag::YShadow),
    (PropertyPath::FontScaleX, AnimatedStyleTag::FontScaleX),
    (PropertyPath::FontScaleY, AnimatedStyleTag::FontScaleY),
];

static ALPHA_PAIRS: &[(PropertyPath, AlphaGroupFn)] = &[
    (PropertyPath::ColorPrimaryA, AnimatedStyleTag::PrimaryAlpha),
    (
        PropertyPath::ColorSecondaryA,
        AnimatedStyleTag::SecondaryAlpha,
    ),
    (PropertyPath::ColorOutlineA, AnimatedStyleTag::OutlineAlpha),
    (PropertyPath::ColorBackA, AnimatedStyleTag::BackAlpha),
];

/// DP 进度事件：由分段算法在运行时通过 channel 发送，供实时可视化消费。
#[derive(Debug, Clone)]
pub enum ProgressEvent {
    /// DP 开始处理一条新 action
    ActionStarted {
        action_id: aweft_model::ids::ActionId,
        /// 动作名，界面上「正在拟合哪条动画」就读它。
        name: String,
        /// 格点数（0..=n）
        n: usize,
        /// 格点对应的时间值（长度 n+1）
        grid_times: Vec<f64>,
        /// 真实曲线采样（供渲染用）：取全局摆幅最大的那条通道，画一根不动的
        /// 通道没有意义。
        curve_samples: Vec<(f64, f64)>,
    },
    /// DP 走完一个格点 `end`：进度、本格选出的起点，以及（诊断详略下）本格逐起点的
    /// 有效性判据。
    DpProgress {
        /// 当前格点，取值 `1..=n`。
        end: usize,
        /// 区间总数，与 `ActionStarted.n` 相同。
        n: usize,
        /// DP 为该 `end` 选出的起点，即最终 `parent[end]`；`None` 表示本格还不可达。
        /// `end` 的起点一旦定案就不再改写，所以按 `end` 顺序记下一份 `chosen` 就能
        /// 在消费端重建出已锁定的前缀路径，不必每格传整条链。
        chosen: Option<usize>,
        /// 该 `end` 下各起点的有效性：`valid[start]` 为真表示该段误差达标，
        /// 长度 = `end`。只有 `ProgressDetail::Diagnostic` 才填，Light 下为空。
        valid: Vec<bool>,
    },
    /// DP 完成，`path` 是最终分段路径
    SegmentsFound { path: Vec<(usize, usize)> },
}

/// 进度事件的详细程度。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum ProgressDetail {
    /// 界面够用的那一组：动作名、DP 推进（含本格起点）、最终路径。DP 走的是
    /// 与不发进度时同一条实现，只是每格多一次 channel 发送。
    #[default]
    Light,
    /// `DpProgress.valid` 额外带上该格全部起点的有效性判据，供画有效性色带
    /// （`live_dp` 与字幕拟合浮层用它）。每格多分配一张布尔表，`is_valid` 次数不变。
    Diagnostic,
}

pub struct ConvertOptions {
    pub error_tolerance: f64,
    /// 若设置，DP 分段过程中会通过此通道发送实时进度事件
    pub progress_sender: Option<std::sync::mpsc::Sender<ProgressEvent>>,
    /// 发了 `progress_sender` 时要不要附带调试用的全量判据。
    pub progress_detail: ProgressDetail,
}

impl Default for ConvertOptions {
    fn default() -> Self {
        Self {
            error_tolerance: 0.01,
            progress_sender: None,
            progress_detail: ProgressDetail::Light,
        }
    }
}

// 如果单行可以连续声明就不需要在Cursor当中
// 如果必须分行，则需要Cursor
#[derive(Debug, Clone)]
pub struct Cursor {
    /// 排版尺寸（写进脚本的 PlayResX/Y）：分数位置通道乘的就是它。
    layout: [f64; 2],
    /// 两轴的锚点基值，排版像素。绝对位置曲线缺省时取它，`\move` 走过一段后推进到
    /// 解析出的锚点。
    offset: [f64; 2],
    /// 两轴的锚点基值，排版尺寸的分数。锚点一旦解析成像素就清零，避免下一段重复计入。
    ratio: [f64; 2],
    /// 每个 COLOR_GROUPS 颜色组的当前颜色（按索引对齐）。颜色动画的缺省通道
    /// 从这里读取基础值，因此上一条 strip 动画过的通道会被继承，而不是跳回
    /// 静态样式。与 `offset` 携带缺失轴基础值的方式相同。
    colors: [Option<AssColorRgb>; 4],
}

/// 一轴的位置通道：两条单位不同的曲线，任一条都可能没有。
#[derive(Debug, Clone, Copy, Default)]
struct AxisCurves<'a> {
    offset: Option<&'a FCurve>,
    ratio: Option<&'a FCurve>,
}

impl Cursor {
    pub fn from_style(style: &StyleResult, layout: (u32, u32)) -> Self {
        Self {
            layout: [f64::from(layout.0), f64::from(layout.1)],
            offset: style.position.map(|v| [v.x, v.y]).unwrap_or([0.0; 2]),
            ratio: style.position_ratio.map(|v| [v.x, v.y]).unwrap_or([0.0; 2]),
            colors: [
                style.color_primary.as_ref().map(srgba_to_ass_rgb),
                style.color_secondary.as_ref().map(srgba_to_ass_rgb),
                style.color_outline.as_ref().map(srgba_to_ass_rgb),
                style.color_back.as_ref().map(srgba_to_ass_rgb),
            ],
        }
    }

    /// 从 action 的曲线表里按 `[x, y]` 取好两轴的位置通道。
    fn position_curves<'a>(
        curve_map: &std::collections::HashMap<PropertyPath, &'a ActionCurve>,
    ) -> [AxisCurves<'a>; 2] {
        let axis = |offset_path, ratio_path| AxisCurves {
            offset: curve_map.get(&offset_path).map(|curve| &curve.curve),
            ratio: curve_map.get(&ratio_path).map(|curve| &curve.curve),
        };
        [
            axis(PropertyPath::PositionX, PropertyPath::PositionRatioX),
            axis(PropertyPath::PositionY, PropertyPath::PositionRatioY),
        ]
    }

    /// `t` 时刻的锚点，排版像素：分数通道按比例随排版尺寸缩放，绝对通道作偏移。
    fn anchor_at(&self, axes: [AxisCurves<'_>; 2], t: f64) -> [f64; 2] {
        std::array::from_fn(|axis| {
            let curves = axes[axis];
            let offset = curves
                .offset
                .map_or(self.offset[axis], |curve| evaluate_fcurve(curve, t));
            let ratio = curves
                .ratio
                .map_or(self.ratio[axis], |curve| evaluate_fcurve(curve, t));
            offset + self.layout[axis] * ratio
        })
    }

    /// 一条 strip 走完：锚点已解析成像素，所以分数基值清零。
    /// 只在 strip 末尾调用一次——strip 内的每段都从同一个基值解析，折回来会重复计分。
    fn advance_to(&mut self, anchor: [f64; 2]) {
        self.offset = anchor;
        self.ratio = [0.0; 2];
    }

    /// 组 `idx` 的通道（0=R, 1=G, 2=B）值，范围 0..1；该组没有记录颜色时
    /// 默认为白色。
    fn color_or_default(&self, idx: usize, channel: usize) -> f64 {
        match self.colors[idx] {
            Some(c) => match channel {
                0 => c.red as f64 / 255.0,
                1 => c.green as f64 / 255.0,
                _ => c.blue as f64 / 255.0,
            },
            None => 1.0,
        }
    }
}

#[derive(Debug, Clone)]
pub struct AnimAssResult {
    pub segments: Vec<TimedTags>,
    pub fallbacks: Vec<(f64, Vec<StyleTag>)>,
}

#[derive(Debug, Clone)]
pub struct TimedTags {
    pub start_ms: f64,
    pub end_ms: f64,
    pub tags: Vec<StyleTag>,
}

// ── StyleResult → 静态 StyleTag ──────────────────────────────────────────

pub fn style_to_static_tags(style: &StyleResult) -> Vec<StyleTag> {
    let mut tags = Vec::new();

    if let Some(ref name) = style.font_name {
        tags.push(StyleTag::FontName(name.clone()));
    }
    if let Some(v) = style.font_size {
        tags.push(StyleTag::FontSize(v));
    }
    if let Some(v) = style.bold {
        tags.push(StyleTag::Bold(if v {
            BoldType::Bold
        } else {
            BoldType::NonBold
        }));
    }
    if let Some(v) = style.italic {
        tags.push(StyleTag::Italic(v));
    }
    if let Some(v) = style.underline {
        tags.push(StyleTag::Underline(v));
    }
    if let Some(v) = style.strikeout {
        tags.push(StyleTag::Strikeout(v));
    }

    if let Some(ref c) = style.color_primary {
        tags.push(StyleTag::PrimaryColor(srgba_to_ass_rgb(c)));
        tags.push(StyleTag::PrimaryAlpha(srgba_to_ass_alpha(c)));
    }
    if let Some(ref c) = style.color_secondary {
        tags.push(StyleTag::SecondaryColor(srgba_to_ass_rgb(c)));
        tags.push(StyleTag::SecondaryAlpha(srgba_to_ass_alpha(c)));
    }
    if let Some(ref c) = style.color_outline {
        tags.push(StyleTag::OutlineColor(srgba_to_ass_rgb(c)));
        tags.push(StyleTag::OutlineAlpha(srgba_to_ass_alpha(c)));
    }
    if let Some(ref c) = style.color_back {
        tags.push(StyleTag::BackColor(srgba_to_ass_rgb(c)));
        tags.push(StyleTag::BackAlpha(srgba_to_ass_alpha(c)));
    }

    if let Some(ref v) = style.border {
        tags.push(StyleTag::XBord(v.x));
        tags.push(StyleTag::YBord(v.y));
    }
    if let Some(ref v) = style.shadow {
        tags.push(StyleTag::XShadow(v.x));
        tags.push(StyleTag::YShadow(v.y));
    }
    if let Some(ref v) = style.font_scale {
        tags.push(StyleTag::FontScaleX(v.x));
        tags.push(StyleTag::FontScaleY(v.y));
    }
    if let Some(ref v) = style.font_rotation {
        tags.push(StyleTag::FontRotationX(v.x));
        tags.push(StyleTag::FontRotationY(v.y));
        tags.push(StyleTag::FontRotationZ(v.z));
    }
    if let Some(ref v) = style.factor {
        tags.push(StyleTag::FactorX(v.x));
        tags.push(StyleTag::FactorY(v.y));
    }

    if let Some(v) = style.blur {
        tags.push(StyleTag::Blur(v));
    }
    if let Some(v) = style.blur_edges {
        tags.push(StyleTag::BlurEdges(BlurEdgesTypes::Weight(v)));
    }
    if let Some(ref v) = style.origin {
        tags.push(StyleTag::Origin(PointF64 { x: v.x, y: v.y }));
    }
    if let Some(ref r) = style.clip_rect {
        tags.push(StyleTag::ClipRectangle {
            p1: PointI32 {
                x: r.x1.round() as i32,
                y: r.y1.round() as i32,
            },
            p2: PointI32 {
                x: r.x2.round() as i32,
                y: r.y2.round() as i32,
            },
            is_display: true,
        });
    }

    tags
}

/// `layout` 是脚本的排版尺寸（PlayResX/Y）：位置通道的分数单位按它换算成像素。
pub fn match_to_ass_tags(
    matched: &aweft_model::application_graph::ApplicationGraphMatch,
    library: &AnimationLibrary,
    layout: (u32, u32),
    duration: f64,
    opts: &ConvertOptions,
) -> AnimAssResult {
    let mut cursor = Cursor::from_style(&matched.style, layout);
    let style_baseline = style_to_static_tags(&matched.style);

    if let Some(ref anim_id) = matched.animation {
        if let Some(anim_data) = library.anim_datas.get(anim_id) {
            let mut result = convert::convert_anim_data(
                &matched.style,
                &mut cursor,
                anim_data,
                library,
                duration,
                opts,
            );

            // 在每个段前面预置静态样式基线。
            // 第 2 段之后会从事件行的 Style 字段继承默认值，因此严格来说只有
            // 第 1 段需要这个，但处处都加也无害。
            for seg in &mut result.segments {
                let mut full = style_baseline.clone();
                full.append(&mut seg.tags);
                seg.tags = full;
            }

            return result;
        }
    }

    // 无动画：输出单个承载静态样式基线的段。
    if !style_baseline.is_empty() {
        return AnimAssResult {
            segments: vec![TimedTags {
                start_ms: 0.0,
                end_ms: duration * 1000.0,
                tags: style_baseline,
            }],
            fallbacks: Vec::new(),
        };
    }

    AnimAssResult {
        segments: Vec::new(),
        fallbacks: Vec::new(),
    }
}

/// 段构建的误差诊断：`max_err` 是所有动画通道中最大的偏差，每个通道都按自身
/// 全局（整条 strip）值域归一化，因此「段有效」当且仅当 `max_err <= tolerance`。
struct FitDiagnostics {
    max_err: f64,
}

/// 把 action 局部时间映射到 strip 局部时间，每个 action strip 构建一次。
struct TimeMapper {
    actstart: f64,
    action_dur: f64,
    strip_start: f64,
    strip_dur: f64,
}

impl TimeMapper {
    fn to_strip(&self, action_time: f64) -> f64 {
        self.strip_start + ((action_time - self.actstart) / self.action_dur) * self.strip_dur
    }
}

/// 位置：单条 `\move` 弦。位置曲线是「该时刻的绝对锚点」，所以这一段只读基值、
/// 不推进 cursor——段与段的端点都由同一个基值解析出来，首尾自然相接；把上一段终点
/// 折回基值会让下一段在已解析的像素上再加一次分数。
///
/// 两轴各支持两种单位的曲线：排版像素与排版尺寸的分数。端点一律先解析成排版像素
/// 再写进 `\move`（ASS 的坐标就是 PlayRes 像素），所以分数曲线随排版尺寸等比缩放，
/// 绝对曲线一个像素都不动；同一轴上两者并存时相加。
fn build_position(
    curve_map: &std::collections::HashMap<PropertyPath, &ActionCurve>,
    t0: f64,
    t1: f64,
    time: &TimeMapper,
    cursor: &Cursor,
) -> (Vec<StyleTag>, FitDiagnostics) {
    let axes = Cursor::position_curves(curve_map);
    if axes
        .iter()
        .all(|axis| axis.offset.is_none() && axis.ratio.is_none())
    {
        return (Vec::new(), FitDiagnostics { max_err: 0.0 });
    }
    let [x0, y0] = cursor.anchor_at(axes, t0);
    let [x1, y1] = cursor.anchor_at(axes, t1);

    let chord_deviation = chord_error(
        |t| cursor.anchor_at(axes, t),
        t0,
        t1,
        FIT_SAMPLES,
        // 位置误差按排版尺寸归一，tolerance 于是读作「偏画面尺寸的百分之几」。
        cursor.layout[0].max(cursor.layout[1]),
    );

    let start_s = time.to_strip(t0);
    let end_s = time.to_strip(t1);
    (
        vec![StyleTag::Move {
            start: PointF64 { x: x0, y: y0 },
            end: PointF64 { x: x1, y: y1 },
            start_time: Some(TimeDelta::microseconds((start_s * 1_000_000.0) as i64)),
            end_time: Some(TimeDelta::microseconds((end_s * 1_000_000.0) as i64)),
        }],
        FitDiagnostics {
            max_err: chord_deviation,
        },
    )
}

/// 颜色：对一个颜色组的动画通道做联合拟合。
///
/// R/G/B 可独立动画且任一通道都可能缺省。ASS 的 `\c` / `\t(...,\c...)` 必须
/// 一次写全三通道，因此缺省通道从 cursor 的当前颜色补齐（样式基线，跨 strip
/// 继承，默认白色）。单一加速指数只对真正在移动的通道拟合；近平坦通道仍报告
/// 拟合误差供验收判断，但不约束共享指数。
fn build_colors(
    curve_map: &std::collections::HashMap<PropertyPath, &ActionCurve>,
    granges: &std::collections::HashMap<PropertyPath, f64>,
    cursor: &mut Cursor,
    t0: f64,
    t1: f64,
    time: &TimeMapper,
) -> (Vec<StyleTag>, FitDiagnostics) {
    let mut tags = Vec::new();
    let mut max_err = 0.0;
    for (color_group_index, (props, make_anim)) in COLOR_GROUPS.iter().enumerate() {
        let animated: Vec<Option<&ActionCurve>> =
            props.iter().map(|p| curve_map.get(p).copied()).collect();
        if animated.iter().all(Option::is_none) {
            continue;
        }
        let start_color = group_color(&animated, cursor, color_group_index, t0);
        let end_color = group_color(&animated, cursor, color_group_index, t1);
        tags.push(to_static_tag(make_anim(start_color)));
        cursor.colors[color_group_index] = Some(end_color);

        // 与标量同理：颜色组起止相同不等于组内不动。拟合误差必须无条件测量，
        // 否则一个完整往复的颜色会被静态颜色吃掉而 DP 照样判它合格。
        let fit_curves: Vec<&FCurve> = animated.iter().flatten().map(|ac| &ac.curve).collect();
        let fit = fit_color_segment(&fit_curves, t0, t1, FIT_SAMPLES);
        let mut present = 0;
        for (channel_idx, channel) in animated.iter().enumerate() {
            if channel.is_none() {
                continue;
            }
            let scale = fit.channel_errors[present] / granges[&props[channel_idx]];
            present += 1;
            if scale > max_err {
                max_err = scale;
            }
        }
        if start_color != end_color {
            tags.push(StyleTag::Transform {
                start_time: Some(TimeDelta::microseconds(
                    (time.to_strip(t0) * 1_000_000.0) as i64,
                )),
                end_time: Some(TimeDelta::microseconds(
                    (time.to_strip(t1) * 1_000_000.0) as i64,
                )),
                acceleration: fit.accel.map(|a| a.value()),
                tags: vec![make_anim(end_color)],
            });
        }
    }
    (tags, FitDiagnostics { max_err })
}

/// 在时刻 `t` 组装完整的 3 通道颜色：动画通道从曲线求值，缺省通道取 cursor
/// 当前的组颜色。
fn group_color(
    animated: &[Option<&ActionCurve>],
    cursor: &Cursor,
    color_group_index: usize,
    t: f64,
) -> AssColorRgb {
    let channel = |channel_idx: usize| -> f64 {
        match animated[channel_idx] {
            Some(ac) => evaluate_fcurve(&ac.curve, t).clamp(0.0, 1.0),
            None => cursor.color_or_default(color_group_index, channel_idx),
        }
    };
    AssColorRgb::new(
        (channel(0) * 255.0).round() as u8,
        (channel(1) * 255.0).round() as u8,
        (channel(2) * 255.0).round() as u8,
    )
}

/// 标量属性（字号、旋转、模糊、边框等）。
fn build_scalars(
    curve_map: &std::collections::HashMap<PropertyPath, &ActionCurve>,
    granges: &std::collections::HashMap<PropertyPath, f64>,
    t0: f64,
    t1: f64,
    time: &TimeMapper,
) -> (Vec<StyleTag>, FitDiagnostics) {
    let mut tags = Vec::new();
    let mut max_err = 0.0;
    for (prop, make_anim) in SCALAR_MAP {
        let action_curve = match curve_map.get(prop) {
            Some(ac) => ac,
            None => continue,
        };
        if has_modifiers(&action_curve.curve) {
            continue;
        }
        let start_value = evaluate_fcurve(&action_curve.curve, t0);
        let end_value = evaluate_fcurve(&action_curve.curve, t1);
        tags.push(to_static_tag(make_anim(start_value)));
        // 误差无条件测量：段端点相同不等于段内不动（一个完整往复的起止同值）。这种段
        // 只能输出一个静态值，把往复整个吃掉；判它不合格，DP 才会在转折点切开，两半
        // 各自起止不同、可用 `\t` 表达。拟合在 progress=1 处必然偏差一个整幅，所以真
        // 往复必定超限；幅度远小于全局值域的摆动仍然合格。
        let fit = fit_scalar_segment(&action_curve.curve, t0, t1, FIT_SAMPLES);
        let scale = fit.max_err / granges[prop];
        if scale > max_err {
            max_err = scale;
        }
        if (start_value - end_value).abs() > 1e-9 {
            tags.push(StyleTag::Transform {
                start_time: Some(TimeDelta::microseconds(
                    (time.to_strip(t0) * 1_000_000.0) as i64,
                )),
                end_time: Some(TimeDelta::microseconds(
                    (time.to_strip(t1) * 1_000_000.0) as i64,
                )),
                acceleration: fit.accel.map(Acceleration::abs).map(|a| a.value()),
                tags: vec![make_anim(end_value)],
            });
        }
    }
    (tags, FitDiagnostics { max_err })
}

/// Alpha 通道（0..1 反色标度，量化到 u8）。
fn build_alphas(
    curve_map: &std::collections::HashMap<PropertyPath, &ActionCurve>,
    granges: &std::collections::HashMap<PropertyPath, f64>,
    t0: f64,
    t1: f64,
    time: &TimeMapper,
) -> (Vec<StyleTag>, FitDiagnostics) {
    let mut tags = Vec::new();
    let mut max_err = 0.0;
    for (prop, make_anim) in ALPHA_PAIRS {
        let action_curve = match curve_map.get(prop) {
            Some(ac) => ac,
            None => continue,
        };
        if has_modifiers(&action_curve.curve) {
            continue;
        }
        let alpha_start = ((1.0 - evaluate_fcurve(&action_curve.curve, t0).clamp(0.0, 1.0)) * 255.0)
            .round() as u8;
        let alpha_end = ((1.0 - evaluate_fcurve(&action_curve.curve, t1).clamp(0.0, 1.0)) * 255.0)
            .round() as u8;
        tags.push(to_static_tag(make_anim(alpha_start)));
        // 与标量同理：量化后端点相同不等于段内不动，误差必须无条件测量才能逼 DP 切开。
        let fit = fit_scalar_segment(&action_curve.curve, t0, t1, FIT_SAMPLES);
        let scale = fit.max_err / granges[prop];
        if scale > max_err {
            max_err = scale;
        }
        if alpha_start != alpha_end {
            tags.push(StyleTag::Transform {
                start_time: Some(TimeDelta::microseconds(
                    (time.to_strip(t0) * 1_000_000.0) as i64,
                )),
                end_time: Some(TimeDelta::microseconds(
                    (time.to_strip(t1) * 1_000_000.0) as i64,
                )),
                acceleration: fit.accel.map(Acceleration::abs).map(|a| a.value()),
                tags: vec![make_anim(alpha_end)],
            });
        }
    }
    (tags, FitDiagnostics { max_err })
}

// 转换标签与聚合误差
fn try_build_segment(
    t0: f64,
    t1: f64,
    curve_map: &std::collections::HashMap<PropertyPath, &ActionCurve>,
    granges: &std::collections::HashMap<PropertyPath, f64>,
    time: &TimeMapper,
    cursor: &mut Cursor,
) -> (Vec<StyleTag>, FitDiagnostics) {
    let mut tags = Vec::new();
    let mut max_err: f64 = 0.0;

    // 颜色会就地写回 cursor（跨段继承基色）；位置与标量只读基值，彼此无先后依赖。
    for (mut built_tags, diagnostics) in [
        build_position(curve_map, t0, t1, time, cursor),
        build_colors(curve_map, granges, cursor, t0, t1, time),
        build_scalars(curve_map, granges, t0, t1, time),
        build_alphas(curve_map, granges, t0, t1, time),
    ] {
        tags.append(&mut built_tags);
        max_err = max_err.max(diagnostics.max_err);
    }

    (tags, FitDiagnostics { max_err })
}

/// 曲线在 `[t_start, t_end]` 上的总摆幅（最大值 - 最小值）。误差按这个全局值域
/// 归一化，因此 tolerance 的含义是「偏差 < 整个动画的 X%」，与局部段的陡峭程度
/// 无关。
pub(crate) fn curve_global_range(curve: &FCurve, t_start: f64, t_end: f64) -> f64 {
    let mut vmin = f64::MAX;
    let mut vmax = f64::MIN;
    let mut bounds: Vec<f64> = Vec::with_capacity(curve.keyframes.len() + 2);
    bounds.push(t_start);
    for k in &curve.keyframes {
        if k.time > t_start && k.time < t_end {
            bounds.push(k.time);
        }
    }
    bounds.push(t_end);
    let intervals = bounds.len() - 1;
    let per_interval = RANGE_SAMPLES_PER_INTERVAL
        .min(RANGE_SAMPLES_MAX / intervals.max(1))
        .max(1);
    for w in bounds.windows(2) {
        let a = w[0];
        let b = w[1];
        for i in 0..=per_interval {
            let v = evaluate_fcurve(curve, a + (b - a) * (i as f64 / per_interval as f64));
            vmin = vmin.min(v);
            vmax = vmax.max(v);
        }
    }
    (vmax - vmin).max(1e-12)
}

fn to_static_tag(anim: AnimatedStyleTag) -> StyleTag {
    match anim {
        AnimatedStyleTag::FontSize(v) => StyleTag::FontSize(v),
        AnimatedStyleTag::FontSpacing(v) => StyleTag::FontSpacing(v),
        AnimatedStyleTag::PrimaryColor(c) => StyleTag::PrimaryColor(c),
        AnimatedStyleTag::SecondaryColor(c) => StyleTag::SecondaryColor(c),
        AnimatedStyleTag::OutlineColor(c) => StyleTag::OutlineColor(c),
        AnimatedStyleTag::BackColor(c) => StyleTag::BackColor(c),
        AnimatedStyleTag::AllAlpha(a) => StyleTag::AllAlpha(a),
        AnimatedStyleTag::PrimaryAlpha(a) => StyleTag::PrimaryAlpha(a),
        AnimatedStyleTag::SecondaryAlpha(a) => StyleTag::SecondaryAlpha(a),
        AnimatedStyleTag::OutlineAlpha(a) => StyleTag::OutlineAlpha(a),
        AnimatedStyleTag::BackAlpha(a) => StyleTag::BackAlpha(a),
        AnimatedStyleTag::FontScaleX(v) => StyleTag::FontScaleX(v),
        AnimatedStyleTag::FontScaleY(v) => StyleTag::FontScaleY(v),
        AnimatedStyleTag::FontRotationX(v) => StyleTag::FontRotationX(v),
        AnimatedStyleTag::FontRotationY(v) => StyleTag::FontRotationY(v),
        AnimatedStyleTag::FontRotationZ(v) => StyleTag::FontRotationZ(v),
        AnimatedStyleTag::FactorX(v) => StyleTag::FactorX(v),
        AnimatedStyleTag::FactorY(v) => StyleTag::FactorY(v),
        AnimatedStyleTag::Bord(v) => StyleTag::Bord(v),
        AnimatedStyleTag::XBord(v) => StyleTag::XBord(v),
        AnimatedStyleTag::YBord(v) => StyleTag::YBord(v),
        AnimatedStyleTag::Shadow(v) => StyleTag::Shadow(v),
        AnimatedStyleTag::XShadow(v) => StyleTag::XShadow(v),
        AnimatedStyleTag::YShadow(v) => StyleTag::YShadow(v),
        AnimatedStyleTag::Blur(v) => StyleTag::Blur(v),
        AnimatedStyleTag::BlurEdges(v) => StyleTag::BlurEdges(v),
        _ => unimplemented!("AnimatedStyleTag variant has no direct StyleTag equivalent"),
    }
}

fn has_modifiers(fcurve: &FCurve) -> bool {
    !fcurve.modifiers.is_empty()
}

// ── 颜色转换辅助 ─────────────────────────────────────────────────────────

fn srgba_to_ass_rgb(color: &Srgba) -> AssColorRgb {
    AssColorRgb::new(
        (color.red.clamp(0.0, 1.0) * 255.0).round() as u8,
        (color.green.clamp(0.0, 1.0) * 255.0).round() as u8,
        (color.blue.clamp(0.0, 1.0) * 255.0).round() as u8,
    )
}

fn srgba_to_ass_alpha(color: &Srgba) -> u8 {
    ((1.0 - color.alpha.clamp(0.0, 1.0)) * 255.0).round() as u8
}

// ── 测试 ─────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::convert::convert_anim_data;
    use super::*;
    use aweft_model::animation::action::{Action, ActionCurve};
    use aweft_model::animation::fcurve::FCurve;
    use aweft_model::animation::keyframe::Keyframe;
    use aweft_model::animation::strip::{NlaStrip, NlaTrack, StripPlacement};
    use aweft_model::animation::AnimData;
    use aweft_model::ids::{ActionId, AnimDataId};
    use aweft_model::node_graph::Vec2;

    /// 测试用排版尺寸：分数位置通道乘的就是它。
    const LAYOUT: (u32, u32) = (1920, 1080);

    fn kf(count: usize, duration: f64, y0: f64, ys: &[f64]) -> Vec<(f64, f64)> {
        (0..count)
            .map(|i| {
                (
                    i as f64 / (count - 1) as f64 * duration,
                    if i == 0 { y0 } else { ys[i - 1] },
                )
            })
            .collect()
    }

    fn linear_curve(property: PropertyPath, kfs: &[(f64, f64)]) -> ActionCurve {
        ActionCurve::new(
            property,
            FCurve {
                keyframes: kfs.iter().map(|&(t, v)| Keyframe::linear(t, v)).collect(),
                ..Default::default()
            },
        )
    }

    /// 把这些曲线包成单条 action strip，按 `layout` 转成标签段。
    fn convert_curves(curves: Vec<ActionCurve>, layout: (u32, u32)) -> AnimAssResult {
        convert_curves_with(curves, layout, &ConvertOptions::default())
    }

    fn convert_curves_with(
        curves: Vec<ActionCurve>,
        layout: (u32, u32),
        opts: &ConvertOptions,
    ) -> AnimAssResult {
        let duration = curves
            .iter()
            .flat_map(|curve| curve.curve.keyframes.iter())
            .map(|key| key.time)
            .fold(0.0f64, f64::max)
            .max(0.001);
        let mut lib = AnimationLibrary::default();
        let id = ActionId::new();
        lib.insert_action(Action {
            id,
            name: "a".into(),
            duration,
            curves,
        });
        let style = StyleResult::default();
        convert_anim_data(
            &style,
            &mut Cursor::from_style(&style, layout),
            &AnimData {
                id: AnimDataId::new(),
                name: "a".into(),
                base_action: None,
                temporary_edit: false,
                tracks: vec![NlaTrack {
                    name: "t".into(),
                    muted: false,
                    solo: false,
                    strips: vec![NlaStrip::new_action("s", id)],
                }],
            },
            &lib,
            duration,
            opts,
        )
    }

    /// 结果里所有 `\move` 的端点：[x0, y0, x1, y1]。
    fn moves(result: &AnimAssResult) -> Vec<[f64; 4]> {
        result
            .segments
            .iter()
            .flat_map(|segment| segment.tags.iter())
            .filter_map(|tag| match tag {
                StyleTag::Move { start, end, .. } => Some([start.x, start.y, end.x, end.y]),
                _ => None,
            })
            .collect()
    }

    fn convert(kfs: &[(f64, f64)], tolerance: f64) -> AnimAssResult {
        let duration = kfs.last().map(|(t, _)| *t).unwrap_or(1.0).max(0.001);
        let mut lib = AnimationLibrary::default();
        let id = ActionId::new();
        lib.insert_action(Action {
            id,
            name: "a".into(),
            duration,
            curves: vec![ActionCurve::new(
                PropertyPath::FontSize,
                FCurve {
                    keyframes: kfs.iter().map(|&(t, v)| Keyframe::linear(t, v)).collect(),
                    ..Default::default()
                },
            )],
        });
        convert_anim_data(
            &StyleResult::default(),
            &mut Cursor::from_style(&StyleResult::default(), LAYOUT),
            &AnimData {
                id: AnimDataId::new(),
                name: "a".into(),
                base_action: None,
                temporary_edit: false,
                tracks: vec![NlaTrack {
                    name: "t".into(),
                    muted: false,
                    solo: false,
                    strips: vec![NlaStrip::new_action("s", id)],
                }],
            },
            &lib,
            duration,
            &ConvertOptions {
                error_tolerance: tolerance,
                ..Default::default()
            },
        )
    }

    // ── style_to_static_tags ────────────────────────────────────────────────
    #[test]
    fn static_tags() {
        let style = StyleResult {
            font_size: Some(30.0),
            ..Default::default()
        };
        assert!(style_to_static_tags(&style).contains(&StyleTag::FontSize(30.0)));

        let style = StyleResult {
            bold: Some(true),
            ..Default::default()
        };
        assert!(style_to_static_tags(&style)
            .iter()
            .any(|t| matches!(t, StyleTag::Bold(_))));

        let style = StyleResult {
            position: Some(Vec2 { x: 100.0, y: 200.0 }),
            ..Default::default()
        };
        assert!(!style_to_static_tags(&style)
            .iter()
            .any(|t| matches!(t, StyleTag::Position(_))));

        assert!(style_to_static_tags(&StyleResult::default()).is_empty());
    }

    // ── 幂函数拟合 ──────────────────────────────────────────────────────────
    #[test]
    fn power_law_fit() {
        use super::fit::{fit_power_law, FitSample};
        for (e, name) in [
            (0.5, "sqrt"),
            (1.0, "linear"),
            (3.0, "cubic"),
            (10.0, "steep"),
        ] {
            let n = 128;
            let samples: Vec<_> = (0..n)
                .map(|i| {
                    let p = i as f64 / (n - 1) as f64;
                    FitSample {
                        progress: p,
                        value: p.powf(e),
                    }
                })
                .collect();
            let fit = fit_power_law(&samples);
            let accel = fit.accel.expect("应有拟合结果");
            let rel = (accel.value() / e - 1.0).abs();
            assert!(
                rel < 0.1,
                "{}: e={}→{} rel_err={:.4}",
                name,
                e,
                accel.value(),
                rel
            );
            assert!(fit.max_err < 0.01, "{}: max_err={}", name, fit.max_err);
        }
    }

    // ── 弦误差 ──────────────────────────────────────────────────────────────
    #[test]
    fn chord_error() {
        use super::fit::chord_error;
        assert!(chord_error(|_| [0.0, 0.0], 0.0, 1.0, 32, 1.0) < 1e-3);
        assert!(chord_error(|t| [t, t * 2.0], 0.0, 1.0, 32, 1.0) < 0.01);
        let err = chord_error(|t| [t, (t - 0.5).powi(2) * 4.0], 0.0, 1.0, 64, 1.0);
        assert!(err > 0.01, "弯曲路径应有弦误差");
        // `norm` 只做线性缩放，误差仍是「偏基准的百分之几」。
        let scaled = chord_error(|t| [t, (t - 0.5).powi(2) * 4.0], 0.0, 1.0, 64, 10.0);
        assert!(
            (scaled - err / 10.0).abs() < 1e-9,
            "norm 应只缩放误差：{scaled} vs {err}"
        );
        // 往复转折点附近弦长趋零，误差不得因此发散（按基准而非弦长归一）。
        let turn = chord_error(
            |t| [0.0, (t - 0.5).powi(2) * 4.0 - 1.0],
            0.0,
            1.0,
            64,
            100.0,
        );
        assert!(turn < 0.05, "零长弦的弯曲量应与画面尺寸同量级，实际 {turn}");
    }

    // ── 分段转换 ────────────────────────────────────────────────────────────
    #[test]
    fn scalar_conversion() {
        for kfs in [
            kf(2, 2.0, 10.0, &[20.0]),
            kf(4, 2.0, 50.0, &[40.0, 20.0, 10.0]),
        ] {
            let result = convert(&kfs, 0.01);
            assert_eq!(result.segments.len(), 1);
            assert!(
                result.segments[0]
                    .tags
                    .iter()
                    .any(|t| matches!(t, StyleTag::Transform { .. })),
                "纯线性字号应有 \\t"
            );
            assert!(result.fallbacks.is_empty());
        }
        // 往返曲线（0→30→0）起点=终点，不产生 Transform（已有行为限制）
        let result = convert(&kf(3, 2.0, 0.0, &[30.0, 0.0]), 0.01);
        assert_eq!(result.segments.len(), 1);
        assert!(result.segments[0]
            .tags
            .iter()
            .any(|t| matches!(t, StyleTag::FontSize(_))));
    }

    #[test]
    fn position_conversion() {
        let kfs = kf(3, 2.0, 0.0, &[100.0, 200.0]);
        let mut lib = AnimationLibrary::default();
        let id = ActionId::new();
        lib.insert_action(Action {
            id,
            name: "p".into(),
            duration: 2.0,
            curves: vec![
                ActionCurve::new(
                    PropertyPath::PositionX,
                    FCurve {
                        keyframes: kfs.iter().map(|&(t, v)| Keyframe::linear(t, v)).collect(),
                        ..Default::default()
                    },
                ),
                ActionCurve::new(
                    PropertyPath::PositionY,
                    FCurve {
                        keyframes: kfs.iter().map(|&(t, v)| Keyframe::linear(t, v)).collect(),
                        ..Default::default()
                    },
                ),
            ],
        });
        let result = convert_anim_data(
            &StyleResult::default(),
            &mut Cursor::from_style(&StyleResult::default(), LAYOUT),
            &AnimData {
                id: AnimDataId::new(),
                name: "p".into(),
                base_action: None,
                temporary_edit: false,
                tracks: vec![NlaTrack {
                    name: "t".into(),
                    muted: false,
                    solo: false,
                    strips: vec![NlaStrip::new_action("s", id)],
                }],
            },
            &lib,
            2.0,
            &ConvertOptions::default(),
        );
        for seg in &result.segments {
            assert_eq!(
                seg.tags
                    .iter()
                    .filter(|t| matches!(t, StyleTag::Move { .. }))
                    .count(),
                1
            );
        }
    }

    /// 分数通道：排版尺寸翻倍，锚点像素也翻倍——这就是「位置跟视口走」的判据。
    #[test]
    fn a_ratio_channel_scales_with_the_layout() {
        let curves = vec![
            linear_curve(PropertyPath::PositionRatioX, &[(0.0, 0.5), (2.0, 0.5)]),
            linear_curve(PropertyPath::PositionRatioY, &[(0.0, 0.7), (2.0, 0.9)]),
        ];
        let one = moves(&convert_curves(curves.clone(), LAYOUT));
        let two = moves(&convert_curves(curves, (3840, 2160)));

        assert_eq!(one.len(), 1, "线性分数曲线应只有一个 \\move");
        assert_eq!(one.len(), two.len(), "换排版尺寸改变了分段数");
        for (index, (small, large)) in one[0].into_iter().zip(two[0]).enumerate() {
            assert!(
                (large - small * 2.0).abs() <= 1e-6,
                "第 {index} 个端点没有按排版尺寸翻倍：{small} → {large}"
            );
        }
        // 1920×1080 下的期望锚点：水平居中、纵向 70% → 90%
        let [x0, y0, x1, y1] = one[0];
        for (index, (got, want)) in [(x0, 960.0), (y0, 756.0), (x1, 960.0), (y1, 972.0)]
            .into_iter()
            .enumerate()
        {
            assert!(
                (got - want).abs() <= 1e-6,
                "第 {index} 个端点 {got} 不是排版尺寸的分数位置 {want}"
            );
        }
    }

    /// 绝对通道：换排版尺寸一个像素都不动。
    #[test]
    fn an_absolute_channel_is_the_same_pixels_at_any_layout() {
        let curves = vec![linear_curve(
            PropertyPath::PositionY,
            &[(0.0, 700.0), (2.0, 900.0)],
        )];
        let one = moves(&convert_curves(curves.clone(), LAYOUT));
        let two = moves(&convert_curves(curves, (3840, 2160)));
        assert_eq!(one, two, "排版像素通道被排版尺寸改动了");
    }

    /// 同一轴上两种单位并存：分数部分按比例缩放，绝对部分作固定偏移。
    #[test]
    fn a_ratio_and_an_absolute_channel_on_the_same_axis_add() {
        let curves = vec![
            linear_curve(PropertyPath::PositionRatioY, &[(0.0, 0.5), (2.0, 0.5)]),
            linear_curve(PropertyPath::PositionY, &[(0.0, -40.0), (2.0, -40.0)]),
        ];
        let one = moves(&convert_curves(curves.clone(), LAYOUT));
        let two = moves(&convert_curves(curves, (3840, 2160)));
        assert!(
            (one[0][1] - 500.0).abs() <= 1e-6,
            "1080 高时 0.5 + (-40px) 应为 500：{}",
            one[0][1]
        );
        assert!(
            (two[0][1] - 1040.0).abs() <= 1e-6,
            "2160 高时同一个通道组合应为 1040：{}",
            two[0][1]
        );
    }

    /// 单轴往复：一个来回的起点=终点，弦退化成一个点。退化分支若直接按零误差放行，
    /// 整段会被判成完美拟合、位置被拍平成一个静态 `\move`，字幕看着就是不动。
    #[test]
    fn a_round_trip_on_one_axis_is_still_split_into_motion() {
        let curves = vec![
            linear_curve(PropertyPath::PositionRatioX, &[(0.0, 0.5), (2.0, 0.5)]),
            linear_curve(
                PropertyPath::PositionRatioY,
                &[
                    (0.0, 0.72),
                    (0.5, 0.60),
                    (1.0, 0.72),
                    (1.5, 0.60),
                    (2.0, 0.72),
                ],
            ),
        ];
        let result = convert_curves(curves, LAYOUT);
        let moves = moves(&result);
        let ys: Vec<f64> = moves.iter().flat_map(|m| [m[1], m[3]]).collect();

        assert!(moves.len() > 1, "一个往复应拆成多段 \\move，实际 {moves:?}");
        // 位置误差按排版尺寸归一（默认 1%），端点只需在容差内逼近波的上下两端：
        // 0.72 → 777.6、0.60 → 648。
        let tol = ConvertOptions::default().error_tolerance
            * f64::max(f64::from(LAYOUT.0), f64::from(LAYOUT.1));
        for extreme in [777.6, 648.0] {
            assert!(
                ys.iter().any(|&y| (y - extreme).abs() <= tol),
                "上下两端都应在 `\\move` 端点的 {tol}px 邻域内出现，实际 y 端点 {ys:?}"
            );
        }
    }

    /// 输出里出现过的所有 `\fs` 值：既包括静态标签，也包括 `\t` 的目标值。
    fn font_sizes(result: &AnimAssResult) -> Vec<f64> {
        let mut out = Vec::new();
        for segment in &result.segments {
            for tag in &segment.tags {
                match tag {
                    StyleTag::FontSize(v) => out.push(*v),
                    StyleTag::Transform { tags, .. } => {
                        for anim in tags {
                            if let AnimatedStyleTag::FontSize(v) = anim {
                                out.push(*v);
                            }
                        }
                    }
                    _ => {}
                }
            }
        }
        out
    }

    /// 标量版的同一个坑：字号一个来回的起止也同值，退化分支同样会把它拍平成一根静态
    /// `\fs`。波峰必须仍然出现在输出里（允许 tolerance × 全局值域的误差）。
    #[test]
    fn a_round_trip_scalar_still_reaches_its_peak() {
        let curves = vec![linear_curve(
            PropertyPath::FontSize,
            &[
                (0.0, 40.0),
                (0.5, 60.0),
                (1.0, 40.0),
                (1.5, 60.0),
                (2.0, 40.0),
            ],
        )];
        let sizes = font_sizes(&convert_curves(curves, LAYOUT));
        let peak = sizes.iter().cloned().fold(f64::MIN, f64::max);
        let tol = ConvertOptions::default().error_tolerance * 20.0;
        assert!(
            (peak - 60.0).abs() <= tol,
            "往复的波峰 60 应出现在输出里（±{tol}），实际 {sizes:?}"
        );
    }

    /// 轻量进度事件是界面浮层唯一的输入：每条动画一份曲线图、按格点推进到满格、收尾
    /// 给出分段路径，而且不带调试版那张逐起点有效性表。发不发进度不得改变转换结果。
    #[test]
    fn light_progress_events_drive_the_ui_overlay() {
        let keyframes = &[(0.0, 40.0), (0.5, 60.0), (1.0, 40.0)];
        let build = || linear_curve(PropertyPath::FontSize, keyframes);

        let (sender, receiver) = std::sync::mpsc::channel();
        let opts = ConvertOptions {
            progress_sender: Some(sender),
            ..Default::default()
        };
        let with_progress = font_sizes(&convert_curves_with(vec![build()], LAYOUT, &opts));
        let events: Vec<ProgressEvent> = receiver.try_iter().collect();

        let plain = font_sizes(&convert_curves(vec![build()], LAYOUT));
        assert_eq!(with_progress, plain, "进度上报不得改变拟合结果");

        let started: Vec<&ProgressEvent> = events
            .iter()
            .filter(|event| matches!(event, ProgressEvent::ActionStarted { .. }))
            .collect();
        assert_eq!(started.len(), 1, "一条动画一份曲线图，实际 {started:?}");
        let ProgressEvent::ActionStarted {
            name,
            n,
            grid_times,
            curve_samples,
            ..
        } = started[0]
        else {
            unreachable!()
        };
        assert_eq!(name, "a");
        assert_eq!(grid_times.len(), n + 1, "格点时间必须覆盖 0..=n");
        assert!(!curve_samples.is_empty(), "曲线图不能是空的");

        let steps: Vec<usize> = events
            .iter()
            .filter_map(|event| match event {
                ProgressEvent::DpProgress { end, .. } => Some(*end),
                _ => None,
            })
            .collect();
        assert!(
            !steps.is_empty() && steps[steps.len() - 1] == *n,
            "进度要一路报满 {n} 格，实际 {steps:?}"
        );
        assert!(
            events.iter().any(
                |event| matches!(event, ProgressEvent::SegmentsFound { path } if !path.is_empty())
            ),
            "收尾要给出最终分段路径"
        );
        assert!(
            events.iter().all(
                |event| !matches!(event, ProgressEvent::DpProgress { valid, .. } if !valid.is_empty())
            ),
            "轻量模式不该带逐起点有效性表"
        );
    }

    /// 诊断详略下界面拿两样东西：逐起点有效性表画色带，本格起点画候选段。
    /// 已锁定路径不随事件传，消费端按 `end` 顺序记 `chosen` 自己回溯——这条测试就是
    /// 在钉住「记下来的链」和「最终导出的路径」是同一份东西。
    #[test]
    fn diagnostic_progress_rebuilds_the_final_path() {
        let keyframes = &[(0.0, 40.0), (0.5, 60.0), (1.0, 40.0)];
        let (sender, receiver) = std::sync::mpsc::channel();
        let opts = ConvertOptions {
            progress_sender: Some(sender),
            progress_detail: ProgressDetail::Diagnostic,
            ..Default::default()
        };
        let _ = convert_curves_with(
            vec![linear_curve(PropertyPath::FontSize, keyframes)],
            LAYOUT,
            &opts,
        );
        let events: Vec<ProgressEvent> = receiver.try_iter().collect();

        let mut parents: Vec<usize> = Vec::new();
        let mut n = 0;
        let mut final_path = Vec::new();
        for event in &events {
            match event {
                ProgressEvent::DpProgress {
                    end,
                    n: en,
                    chosen,
                    valid,
                } => {
                    assert!(
                        !valid.is_empty(),
                        "诊断详略下第 {end} 格要带有效性表，实际长度 {}",
                        valid.len()
                    );
                    assert_eq!(valid.len(), *end, "有效性表逐起点，长度 = end");
                    parents.push(chosen.unwrap_or(usize::MAX));
                    assert_eq!(parents.len(), *end, "每格报一次、按序报");
                    n = *en;
                }
                ProgressEvent::SegmentsFound { path } => final_path = path.clone(),
                _ => {}
            }
        }
        assert_eq!(parents.len(), n, "报满 {n} 格");

        let mut rebuilt = Vec::new();
        let mut end = n;
        while end != 0 {
            let start = parents[end - 1];
            assert_ne!(start, usize::MAX, "格点 {end} 不可达，链回溯不回去");
            rebuilt.push((start, end));
            end = start;
        }
        rebuilt.reverse();
        assert_eq!(rebuilt, final_path, "按 chosen 重建的链就是导出用的路径");
    }

    /// 可重复的拟合开销测量：与初始化播种的「上下浮动」同形（贝塞尔往复 + 分数位置）。
    /// 预览卡住主线程的就是这个数，调网格/采样常量前后都回来跑一次：
    /// `cargo test -p aweft-convert --lib -- --ignored --nocapture`。
    #[test]
    #[ignore = "手动跑的耗时测量，不进常规用例"]
    fn measure_fit_cost_of_seeded_float() {
        use aweft_model::animation::keyframe::BezierHandle;

        let handle = 0.5 / 3.0;
        let ease = |time, value| {
            Keyframe::bezier(
                time,
                value,
                BezierHandle::new(-handle, 0.0),
                BezierHandle::new(handle, 0.0),
            )
        };
        let eased = |property, keyframes: Vec<Keyframe>| {
            ActionCurve::new(
                property,
                FCurve {
                    keyframes,
                    ..Default::default()
                },
            )
        };
        let curves = vec![
            eased(
                PropertyPath::PositionRatioX,
                vec![ease(0.0, 0.5), ease(2.0, 0.5)],
            ),
            eased(
                PropertyPath::PositionRatioY,
                vec![
                    ease(0.0, 0.72),
                    ease(0.5, 0.60),
                    ease(1.0, 0.72),
                    ease(1.5, 0.60),
                    ease(2.0, 0.72),
                ],
            ),
        ];
        let started = std::time::Instant::now();
        let result = convert_curves(curves, LAYOUT);
        let moves = moves(&result);
        println!(
            "位置 {} 行（{} 个 \\move），用时 {:.0}ms",
            result.segments.len(),
            moves.len(),
            started.elapsed().as_secs_f64() * 1000.0
        );

        // 标量通道走的是幂函数拟合（minimax 指数网格）这条完全不同的代码路径，
        // 因此单独计时，才能看出指数网格这个旋钮的代价。
        let scalar = eased(
            PropertyPath::FontSize,
            vec![
                ease(0.0, 40.0),
                ease(0.5, 60.0),
                ease(1.0, 40.0),
                ease(1.5, 60.0),
                ease(2.0, 40.0),
            ],
        );
        let started = std::time::Instant::now();
        let scalar_result = convert_curves(vec![scalar], LAYOUT);
        let sizes = font_sizes(&scalar_result);
        let transforms = scalar_result
            .segments
            .iter()
            .flat_map(|segment| segment.tags.iter())
            .filter(|tag| matches!(tag, StyleTag::Transform { .. }))
            .count();
        println!(
            "标量 {} 行（{transforms} 个连挂 \\t），峰值 {:.1}（应≈60），用时 {:.0}ms",
            scalar_result.segments.len(),
            sizes.iter().cloned().fold(f64::MIN, f64::max),
            started.elapsed().as_secs_f64() * 1000.0
        );
    }

    /// 位置真的一动不动时仍只输出一段：退化分支不能把静态位置也切成细条。
    #[test]
    fn a_still_position_stays_a_single_move() {
        let curves = vec![linear_curve(
            PropertyPath::PositionRatioY,
            &[(0.0, 0.72), (2.0, 0.72)],
        )];
        let moves = moves(&convert_curves(curves, LAYOUT));
        assert_eq!(moves.len(), 1, "静态位置不该被拆分：{moves:?}");
        assert_eq!((moves[0][1], moves[0][3]), (777.6, 777.6));
    }

    /// 解析规则本身：样式的分数基值与像素基值相加，没有曲线的轴也走同一条解析。
    #[test]
    fn a_style_anchor_is_its_ratio_plus_its_offset() {
        use std::collections::HashMap;

        let style = StyleResult {
            position: Some(Vec2 { x: -40.0, y: 20.0 }),
            position_ratio: Some(Vec2 { x: 0.5, y: 0.25 }),
            ..Default::default()
        };
        let cursor = Cursor::from_style(&style, LAYOUT);
        let no_curves: HashMap<PropertyPath, &ActionCurve> = HashMap::new();
        let [x, y] = cursor.anchor_at(Cursor::position_curves(&no_curves), 0.0);
        assert!((x - (960.0 - 40.0)).abs() <= 1e-9, "x 锚点 {x}");
        assert!((y - (270.0 + 20.0)).abs() <= 1e-9, "y 锚点 {y}");
    }

    #[test]
    fn empty_data() {
        let result = convert_anim_data(
            &StyleResult::default(),
            &mut Cursor::from_style(&StyleResult::default(), LAYOUT),
            &AnimData::new("e"),
            &AnimationLibrary::default(),
            5.0,
            &ConvertOptions::default(),
        );
        assert!(result.segments.is_empty());
    }

    // ── 容差单调 ────────────────────────────────────────────────────────────
    #[test]
    fn tolerance_monotonic() {
        let kfs = kf(3, 2.0, 0.0, &[80.0, 20.0]);
        let count = |tol: f64| -> usize {
            convert(&kfs, tol)
                .segments
                .iter()
                .flat_map(|s| &s.tags)
                .filter(|t| matches!(t, StyleTag::Transform { .. }))
                .count()
        };
        assert!(count(1.0) >= 1);
        assert!(count(0.01) >= count(1.0));
        assert!(count(0.001) >= count(0.01));
    }

    // ── 颜色 ────────────────────────────────────────────────────────────────
    #[test]
    fn color_partial_channel() {
        let mut lib = AnimationLibrary::default();
        let id = ActionId::new();
        lib.insert_action(Action {
            id,
            name: "c".into(),
            duration: 2.0,
            curves: vec![ActionCurve::new(
                PropertyPath::ColorPrimaryR,
                FCurve {
                    keyframes: vec![Keyframe::linear(0.0, 0.0), Keyframe::linear(2.0, 0.8)],
                    ..Default::default()
                },
            )],
        });
        let result = convert_anim_data(
            &StyleResult::default(),
            &mut Cursor::from_style(&StyleResult::default(), LAYOUT),
            &AnimData {
                id: AnimDataId::new(),
                name: "c".into(),
                base_action: None,
                temporary_edit: false,
                tracks: vec![NlaTrack {
                    name: "t".into(),
                    muted: false,
                    solo: false,
                    strips: vec![NlaStrip::new_action("s", id)],
                }],
            },
            &lib,
            2.0,
            &ConvertOptions::default(),
        );
        let c = result.segments[0]
            .tags
            .iter()
            .find_map(|t| match t {
                StyleTag::PrimaryColor(c) => Some(c),
                _ => None,
            })
            .expect("应有静态主颜色");
        assert_eq!(c.green, 255);
        assert_eq!(c.blue, 255);
    }

    #[test]
    fn color_flat_channel() {
        let mut lib = AnimationLibrary::default();
        let id = ActionId::new();
        lib.insert_action(Action {
            id,
            name: "cf".into(),
            duration: 2.0,
            curves: vec![
                ActionCurve::new(
                    PropertyPath::ColorPrimaryR,
                    FCurve {
                        keyframes: vec![Keyframe::linear(0.0, 0.0), Keyframe::linear(2.0, 0.8)],
                        ..Default::default()
                    },
                ),
                ActionCurve::new(
                    PropertyPath::ColorPrimaryG,
                    FCurve {
                        keyframes: vec![Keyframe::linear(0.0, 0.5), Keyframe::linear(2.0, 0.5)],
                        ..Default::default()
                    },
                ),
            ],
        });
        let result = convert_anim_data(
            &StyleResult::default(),
            &mut Cursor::from_style(&StyleResult::default(), LAYOUT),
            &AnimData {
                id: AnimDataId::new(),
                name: "cf".into(),
                base_action: None,
                temporary_edit: false,
                tracks: vec![NlaTrack {
                    name: "t".into(),
                    muted: false,
                    solo: false,
                    strips: vec![NlaStrip::new_action("s", id)],
                }],
            },
            &lib,
            2.0,
            &ConvertOptions::default(),
        );
        let c = result.segments[0]
            .tags
            .iter()
            .find_map(|t| match t {
                StyleTag::PrimaryColor(c) => Some(c),
                _ => None,
            })
            .expect("应有静态主颜色");
        assert!((c.green as f64 - 128.0).abs() < 1.0, "flat G≠128");
        assert_eq!(result.segments.len(), 1, "平坦通道不应导致多段");
    }

    #[test]
    fn color_cross_strip() {
        let mut lib = AnimationLibrary::default();
        let r = ActionId::new();
        lib.insert_action(Action {
            id: r,
            name: "r".into(),
            duration: 2.0,
            curves: vec![ActionCurve::new(
                PropertyPath::ColorPrimaryR,
                FCurve {
                    keyframes: vec![Keyframe::linear(0.0, 0.0), Keyframe::linear(2.0, 0.5)],
                    ..Default::default()
                },
            )],
        });
        let g = ActionId::new();
        lib.insert_action(Action {
            id: g,
            name: "g".into(),
            duration: 2.0,
            curves: vec![ActionCurve::new(
                PropertyPath::ColorPrimaryG,
                FCurve {
                    keyframes: vec![Keyframe::linear(0.0, 0.0), Keyframe::linear(2.0, 1.0)],
                    ..Default::default()
                },
            )],
        });
        let result = convert_anim_data(
            &StyleResult::default(),
            &mut Cursor::from_style(&StyleResult::default(), LAYOUT),
            &AnimData {
                id: AnimDataId::new(),
                name: "2".into(),
                base_action: None,
                temporary_edit: false,
                tracks: vec![NlaTrack {
                    name: "t".into(),
                    muted: false,
                    solo: false,
                    strips: vec![
                        NlaStrip {
                            placement: StripPlacement::Absolute {
                                start: 0.0,
                                end: 2.0,
                            },
                            ..NlaStrip::new_action("s1", r)
                        },
                        NlaStrip {
                            placement: StripPlacement::Absolute {
                                start: 2.0,
                                end: 4.0,
                            },
                            ..NlaStrip::new_action("s2", g)
                        },
                    ],
                }],
            },
            &lib,
            4.0,
            &ConvertOptions::default(),
        );
        let c = result.segments[1]
            .tags
            .iter()
            .find_map(|t| match t {
                StyleTag::PrimaryColor(c) => Some(c),
                _ => None,
            })
            .expect("第二条应有颜色");
        assert_eq!(c.red, 128, "应从第一条继承");
        assert_eq!(c.blue, 255);
    }

    // ── match_to_ass_tags ───────────────────────────────────────────────────
    #[test]
    fn test_match_to_ass_tags() {
        let mut lib = AnimationLibrary::default();
        let id = ActionId::new();
        lib.insert_action(Action {
            id,
            name: "a".into(),
            duration: 1.0,
            curves: vec![ActionCurve::new(
                PropertyPath::FontSize,
                FCurve {
                    keyframes: vec![Keyframe::linear(0.0, 24.0), Keyframe::linear(1.0, 36.0)],
                    ..Default::default()
                },
            )],
        });
        let aid = AnimDataId::new();
        lib.insert_anim_data(AnimData {
            id: aid,
            name: "d".into(),
            base_action: None,
            temporary_edit: false,
            tracks: vec![NlaTrack {
                name: "t".into(),
                muted: false,
                solo: false,
                strips: vec![NlaStrip::new_action("s", id)],
            }],
        });
        let result = match_to_ass_tags(
            &aweft_model::application_graph::ApplicationGraphMatch {
                style: StyleResult {
                    font_size: Some(24.0),
                    bold: Some(true),
                    ..Default::default()
                },
                animation: Some(aid),
            },
            &lib,
            LAYOUT,
            2.0,
            &ConvertOptions::default(),
        );
        let tags = &result.segments[0].tags;
        assert!(tags.iter().any(|t| matches!(t, StyleTag::Bold(_))));
        assert!(tags.iter().any(|t| matches!(t, StyleTag::FontSize(_))));
        assert!(tags.iter().any(|t| matches!(t, StyleTag::Transform { .. })));
    }
}
