//! 框选公共工具：供曲线/摄影表/时间线/NLA 复用。

/// 归一化时间范围：小在前。
pub fn normalize_time(x_lo: f32, x_hi: f32) -> (f32, f32) {
    if x_lo <= x_hi {
        (x_lo, x_hi)
    } else {
        (x_hi, x_lo)
    }
}

/// 归一化值范围：小在前。
pub fn normalize_value(y_lo: f32, y_hi: f32) -> (f32, f32) {
    if y_lo <= y_hi {
        (y_lo, y_hi)
    } else {
        (y_hi, y_lo)
    }
}

/// 行区间 clamp 到 [0, n-1]，**求交**语义：框选竖直范围与有效行区间无交集时
/// 返回 None（空框），调用方可据此在 `None=替换` 时仍清空选择（对齐
/// 曲线/摄影表"空白框选=清除"），而不是把空框 clamp 到首/末行误选。
/// 已在 `graph/marquee.rs:dope_marquee_select` 验证（`r_lo > r_hi` 空区间）。
pub fn clamp_rows(row_lo: i32, row_hi: i32, n: i32) -> Option<(usize, usize)> {
    if n <= 0 {
        return None;
    }
    let (mut lo, mut hi) = if row_lo <= row_hi {
        (row_lo, row_hi)
    } else {
        (row_hi, row_lo)
    };
    let orig_lo = lo;
    let orig_hi = hi;
    if orig_hi < 0 || orig_lo >= n {
        return None;
    }
    lo = lo.max(0).min(n - 1);
    hi = hi.max(0).min(n - 1);
    if lo > hi {
        None
    } else {
        Some((lo as usize, hi as usize))
    }
}
