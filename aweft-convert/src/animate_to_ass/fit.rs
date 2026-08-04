//! 幂函数拟合：把曲线段拟合到 ASS 可表达的 `\t(..., accel)` 加速形状。
//!
//! 纯数学工具，不依赖动画标签、cursor 或时间映射。多个通道的采样可以合并进
//! 同一份列表，拟合出一个所有通道共享的加速指数。

use aweft_model::animation::fcurve::{evaluate_fcurve, FCurve};

use super::MINIMAX_GRID;

/// 幂函数拟合使用的加速指数，即 ASS `\t(..., accel)` 里的 `accel`，
/// 归一化形状满足 `value ≈ progress^accel`。
#[derive(Debug, Clone, Copy)]
pub(super) struct Acceleration(f64);

impl Acceleration {
    /// 线性（匀速）：`accel = 1`。
    pub(super) fn linear() -> Self {
        Self(1.0)
    }

    /// 取绝对值（标量/alpha 标签只接受正的加速指数）。
    pub(super) fn abs(self) -> Self {
        Self(self.0.abs())
    }

    /// 取内部原始数值。
    pub(super) fn value(self) -> f64 {
        self.0
    }
}

/// 对单通道（标量）段拟合的结果。
pub(super) struct PowerFit {
    /// 最优加速指数；无法拟合时为 None。
    pub(super) accel: Option<Acceleration>,
    /// 拟合曲线相对真实曲线的最大绝对偏差。
    pub(super) max_err: f64,
}

/// 颜色组多通道联合拟合的结果（1..3 个动画通道共享同一个加速指数）。
pub(super) struct ColorFit {
    /// 共享加速指数。
    pub(super) accel: Option<Acceleration>,
    /// 每个输入通道的最大绝对偏差，顺序与传入的通道一致。
    pub(super) channel_errors: Vec<f64>,
}

/// 某指数下全体样本中的最大偏差。
struct WorstDeviation {
    deviation: f64,
}

/// 供幂函数拟合器使用的一个归一化采样点。
///
/// `value` 是该进度下需要被追踪的目标值，按通道自身 range 归一化到 0..1，
/// 因此不同通道的采样可以合并到同一份列表里，用同一个指数 `accel` 拟合。
pub(super) struct FitSample {
    /// 段内归一化进度，0..1。
    pub(super) progress: f64,
    /// 归一化目标值。
    pub(super) value: f64,
}

/// 把单个标量段拟合到 ASS 可表达的幂形状 `start_value + (end-start)*progress^accel`
/// （libass 用 `k = pow(progress, accel)` 求混合系数，因此只有这种正向形状
/// 能用 `\t(..., accel)` 表达）。
///
/// 返回 `PowerFit`，其中 `max_err` 是拟合曲线相对真实曲线的最大绝对偏差（原始值单位）。
pub(super) fn fit_scalar_segment(
    curve: &FCurve,
    t0: f64,
    t1: f64,
    sample_count: usize,
) -> PowerFit {
    let eval = |time: f64| evaluate_fcurve(curve, time);
    let start_value = eval(t0);
    let end_value = eval(t1);
    let value_range = (end_value - start_value).abs();
    if value_range < 1e-12 {
        // 值可能往复震荡；从采样中估计实际波动范围
        let mut sample_range: f64 = 0.0;
        for idx in 0..sample_count {
            let time = t0 + idx as f64 / (sample_count - 1) as f64 * (t1 - t0);
            let value = eval(time);
            sample_range = sample_range.max((value - start_value).abs());
        }
        if sample_range < 1e-12 {
            return PowerFit {
                accel: Some(Acceleration::linear()),
                max_err: 0.0,
            };
        }
        let mut samples = Vec::with_capacity(sample_count);
        for idx in 0..sample_count {
            let progress = idx as f64 / (sample_count - 1) as f64;
            let time = t0 + progress * (t1 - t0);
            let value = (eval(time) - start_value) / sample_range;
            samples.push(FitSample {
                progress,
                value,
            });
        }
        let fit = fit_power_law(&samples);
        return PowerFit {
            accel: fit.accel,
            max_err: fit.max_err * sample_range,
        };
    }

    let mut samples = Vec::with_capacity(sample_count);
    for idx in 0..sample_count {
        let progress = idx as f64 / (sample_count - 1) as f64;
        let time = t0 + progress * (t1 - t0);
        let value = (eval(time) - start_value) / (end_value - start_value);
        samples.push(FitSample {
            progress,
            value,
        });
    }

    // 正向拟合：value ≈ progress^accel（缓入形状）
    let fit = fit_power_law(&samples);
    PowerFit {
        accel: fit.accel,
        max_err: fit.max_err * value_range,
    }
}

/// 对颜色组里一个或多个动画通道（1..3 条曲线）拟合共享的幂指数。
///
/// 所有通道的样本合并进同一份列表一起拟合，因此返回的指数是同时追踪每个
/// 通道的单一 `progress^accel`。段内不移动的通道（|起止差| <= 1e-9）不进合并
/// 样本——它们无法被任何指数匹配，只会向目标函数注入与指数无关的幻影误差——
/// 但仍会报告其拟合误差（平坦通道为 0）供验收判断。
pub(super) fn fit_color_segment(
    curves: &[&FCurve],
    t0: f64,
    t1: f64,
    sample_count: usize,
) -> ColorFit {
    let channel_count = curves.len();
    let mut start_values = Vec::with_capacity(channel_count);
    let mut value_deltas = Vec::with_capacity(channel_count);
    let mut is_moving = Vec::with_capacity(channel_count);
    for curve in curves {
        let start_value = evaluate_fcurve(curve, t0);
        let end_value = evaluate_fcurve(curve, t1);
        start_values.push(start_value);
        value_deltas.push(end_value - start_value);
        is_moving.push((end_value - start_value).abs() > 1e-9);
    }

    let mut samples = Vec::new();
    for (channel_idx, curve) in curves.iter().enumerate() {
        if !is_moving[channel_idx] {
            continue;
        }
        let range = value_deltas[channel_idx].abs().max(1e-12);
        for sample_idx in 0..sample_count {
            let progress = sample_idx as f64 / (sample_count - 1) as f64;
            let time = t0 + progress * (t1 - t0);
            let value = (evaluate_fcurve(curve, time) - start_values[channel_idx]) / range;
            samples.push(FitSample {
                progress,
                value,
            });
        }
    }
    let accel = if samples.is_empty() {
        Some(Acceleration::linear())
    } else {
        fit_power_law(&samples).accel
    };
    let exponent = accel.map_or(1.0, |a| a.value());

    // 各通道的最大绝对偏差（原始值单位）。
    let mut channel_errors = Vec::with_capacity(channel_count);
    for channel_idx in 0..channel_count {
        let mut max_deviation = 0.0f64;
        for sample_idx in 0..sample_count {
            let progress = sample_idx as f64 / (sample_count - 1) as f64;
            let time = t0 + progress * (t1 - t0);
            let predicted =
                start_values[channel_idx] + value_deltas[channel_idx] * progress.powf(exponent);
            max_deviation = max_deviation
                .max((evaluate_fcurve(curves[channel_idx], time) - predicted).abs());
        }
        channel_errors.push(max_deviation);
    }
    ColorFit {
        accel,
        channel_errors,
    }
}

/// 计算弦误差：把段内采样点投影到弦方向上，返回相对弦的最大垂直偏差。
pub(super) fn chord_error(
    eval_x: impl Fn(f64) -> f64,
    eval_y: impl Fn(f64) -> f64,
    t0: f64,
    t1: f64,
    // 时间均匀分割数（用于比较误差值）
    sample_count: usize,
) -> f64 {
    let start_x = eval_x(t0);
    let start_y = eval_y(t0);
    let end_x = eval_x(t1);
    let end_y = eval_y(t1);

    let delta_x = end_x - start_x;
    let delta_y = end_y - start_y;
    let chord_sq = delta_x * delta_x + delta_y * delta_y;
    if chord_sq < 1e-12 {
        return 0.0;
    }
    let chord_inv = 1.0 / chord_sq.sqrt();

    let mut max_deviation = 0.0f64;
    for idx in 0..sample_count {
        let time = t0 + idx as f64 / (sample_count - 1) as f64 * (t1 - t0);
        let sample_x = eval_x(time);
        let sample_y = eval_y(time);
        // 投影到弦方向
        let delta_sample_x = sample_x - start_x;
        let delta_sample_y = sample_y - start_y;
        let projection =
            (delta_sample_x * delta_x + delta_sample_y * delta_y) * chord_inv * chord_inv;
        let proj_x = start_x + projection * delta_x;
        let proj_y = start_y + projection * delta_y;
        let dev_sq = (sample_x - proj_x) * (sample_x - proj_x)
            + (sample_y - proj_y) * (sample_y - proj_y);
        let deviation = dev_sq.sqrt() * chord_inv;
        if deviation > max_deviation {
            max_deviation = deviation;
        }
    }
    max_deviation
}

/// 用最小化最大偏差（切比雪夫 / minimax）的方式，把样本拟合到 `progress^accel`
/// 的幂函数形状——与每段验收用的 L∞ 判据一致。多个通道的样本可以合并进同一份
/// 列表，返回的指数是所有通道共享的单一 `accel`。
pub(super) fn fit_power_law(samples: &[FitSample]) -> PowerFit {
    let init_exponent = log_log_init(samples);
    let best_exponent = minimax_grid_optimize(samples, init_exponent);
    let worst = max_error_at(samples, best_exponent.value());
    PowerFit {
        accel: Some(best_exponent),
        max_err: worst.deviation,
    }
}

/// 在指数空间的对数网格上粗搜，再围绕最优点做局部黄金分割精化，返回使
/// 最大偏差最小的指数。
fn minimax_grid_optimize(samples: &[FitSample], hint: Acceleration) -> Acceleration {
    const GRID: usize = MINIMAX_GRID;
    let log_min = 0.001f64.ln();
    let log_max = 1000.0f64.ln();

    let mut best_exponent = hint.value().clamp(0.001, 1000.0);
    let mut best_error = max_error_at(samples, best_exponent).deviation;

    for grid_idx in 0..=GRID {
        let exponent = (log_min + (log_max - log_min) * grid_idx as f64 / GRID as f64).exp();
        let error = max_error_at(samples, exponent).deviation;
        if error < best_error {
            best_error = error;
            best_exponent = exponent;
        }
    }

    // 局部黄金分割精化
    let golden_ratio = (5.0f64.sqrt() - 1.0) / 2.0;
    let mut lower = (best_exponent / 3.0).max(0.001);
    let mut upper = (best_exponent * 3.0).min(1000.0);
    let mut probe_lower = upper - golden_ratio * (upper - lower);
    let mut probe_upper = lower + golden_ratio * (upper - lower);
    let mut error_lower = max_error_at(samples, probe_lower).deviation;
    let mut error_upper = max_error_at(samples, probe_upper).deviation;

    for _ in 0..20 {
        if error_lower <= error_upper {
            upper = probe_upper;
            probe_upper = probe_lower;
            error_upper = error_lower;
            probe_lower = upper - golden_ratio * (upper - lower);
            error_lower = max_error_at(samples, probe_lower).deviation;
        } else {
            lower = probe_lower;
            probe_lower = probe_upper;
            error_lower = error_upper;
            probe_upper = lower + golden_ratio * (upper - lower);
            error_upper = max_error_at(samples, probe_upper).deviation;
        }
    }

    Acceleration((lower + upper) / 2.0)
}

/// 用对数空间的线性回归求一个粗略的指数初值。
fn log_log_init(samples: &[FitSample]) -> Acceleration {
    let mut log_xy_sum = 0.0;
    let mut log_x_sq_sum = 0.0;
    for sample in samples {
        if sample.progress > 0.01 && sample.progress < 0.99 && sample.value > 1e-6 {
            let log_progress = sample.progress.ln();
            let log_value = sample.value.ln();
            log_xy_sum += log_progress * log_value;
            log_x_sq_sum += log_progress * log_progress;
        }
    }
    if log_x_sq_sum.abs() < 1e-12 {
        Acceleration::linear()
    } else {
        Acceleration((log_xy_sum / log_x_sq_sum).clamp(0.001, 1000.0))
    }
}

/// 返回某指数下所有样本中「value 与 progress^accel 偏差」的最大者。
fn max_error_at(samples: &[FitSample], exponent: f64) -> WorstDeviation {
    let mut worst_deviation = 0.0;
    for sample in samples {
        let deviation = (sample.value - sample.progress.powf(exponent)).abs();
        if deviation > worst_deviation {
            worst_deviation = deviation;
        }
    }
    WorstDeviation {
        deviation: worst_deviation,
    }
}
