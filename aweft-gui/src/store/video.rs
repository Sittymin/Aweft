//! 视频域 UI 状态：每个视频会话派生到 Slint `VideoState` 的模型集合。
//! Rust 侧持有 VecModel（共享 inner 模型，变更自动通知 Slint）：
//! - `frames`/`widths`/`heights`：最新帧与尺寸（渲染线程逐帧 `set_row_data`）
//! - `positions`：视口世界坐标位置（级联错开/拖拽移动）
//! - `time_bars`：各面板控制时间条（普通 Action 由投影写，播放时钟逐帧增量写）
//!
//! 会话结构与普通状态变化由 `projections/video` 用 `push`/`set_row_data` 局部更新
//! （不重建既有行，避免拖拽/帧模型被打断）；时间条逐帧更新走投影提供的
//! 实时专用出口。

use std::rc::Rc;

use slint::{Image, Model, VecModel};

use crate::bridge::types::{
    SlintAudioTrack, SlintVideoPos, SlintVideoTimeBar, SlintWaveformBucket,
};

#[derive(Default)]
pub struct VideoUiState {
    /// 每个会话的最新视频帧（下标 = 会话下标；首帧前为默认空图）
    pub frames: Rc<VecModel<Image>>,
    /// 每个会话的视频宽度（0 = 尚未收到首帧）
    pub widths: Rc<VecModel<i32>>,
    /// 每个会话的视频高度（0 = 尚未收到首帧）
    pub heights: Rc<VecModel<i32>>,
    /// 每个会话的视口世界坐标位置
    pub positions: Rc<VecModel<SlintVideoPos>>,
    /// 每个会话的控制时间条（彩虹色 + 选中/主视频标识）
    pub time_bars: Rc<VecModel<SlintVideoTimeBar>>,
    /// 每个视频内部的音轨行模型；TimeBar.audio_tracks 引用对应模型。
    pub audio_track_models: Vec<Rc<VecModel<SlintAudioTrack>>>,
    /// 波形的持久增量模型；`SlintAudioWaveform.buckets` 只引用这些模型。
    /// 后台任务按批次 push/set_row_data，不重复构造完整快照。
    pub waveform_buckets: Vec<Vec<Rc<VecModel<SlintWaveformBucket>>>>,
    /// 与每个视频对应的持久多分辨率 min/max 层（2/5/10/20/50ms……）。
    /// 缩放只切换读取层级，不重新聚合或替换模型。
    pub waveform_level_buckets: Vec<Vec<Vec<Rc<VecModel<SlintWaveformBucket>>>>>,
}

/// 将后台波形批次原地写入基础模型，尽量把连续追加合并成一次 Slint
/// `row_added` 通知。只有覆盖已有行的修订才需要逐行 `row_changed`。
/// 间隙用零桶（静音）填充，与提取端空桶语义一致。
pub(crate) fn ui_apply_waveform_batch(
    values: &VecModel<SlintWaveformBucket>,
    start: usize,
    buckets: &[SlintWaveformBucket],
) {
    if buckets.is_empty() {
        return;
    }
    let current_len = values.row_count();
    if start > current_len {
        values.extend(std::iter::repeat_n(
            SlintWaveformBucket::default(),
            start - current_len,
        ));
    }

    let overlap_end = (start + buckets.len()).min(values.row_count());
    for (row, bucket) in (start..overlap_end).zip(buckets.iter().cloned()) {
        values.set_row_data(row, bucket);
    }

    let append_start = start.max(values.row_count());
    if append_start < start + buckets.len() {
        let offset = append_start - start;
        values.extend(buckets[offset..].iter().cloned());
    }
}

/// 标尺兼容的波形桶宽（毫秒）：2、5、10、20、50、100……。
pub(crate) const fn waveform_level_duration_ms(level: usize) -> usize {
    if level == 0 {
        return 2;
    }
    if level == 1 {
        return 5;
    }
    let group = (level - 2) / 3;
    let multiplier = match (level - 2) % 3 {
        0 => 1,
        1 => 2,
        _ => 5,
    };
    multiplier * 10usize.pow((group + 1) as u32)
}

/// 将 `[range_start, range_end)` 内基础行的 min/max 取极值、rms 按平方加权平均；
/// 区间内无有效行返回 `None`。
fn fold_waveform_bucket(
    base: &VecModel<SlintWaveformBucket>,
    range_start: usize,
    range_end: usize,
) -> Option<SlintWaveformBucket> {
    let mut folded_min = f32::INFINITY;
    let mut folded_max = f32::NEG_INFINITY;
    let mut folded_sumsq = 0.0f64;
    let mut folded_count = 0usize;
    for row in range_start..range_end {
        if let Some(entry) = base.row_data(row) {
            folded_min = folded_min.min(entry.min);
            folded_max = folded_max.max(entry.max);
            folded_sumsq += (entry.rms as f64) * (entry.rms as f64);
            folded_count += 1;
        }
    }
    if folded_count == 0 {
        return None;
    }
    Some(SlintWaveformBucket {
        min: folded_min,
        max: folded_max,
        rms: (folded_sumsq / folded_count as f64).sqrt() as f32,
    })
}

/// 按变更区间增量更新 1/2/5×10^k 波形层。每一层只重算受影响的聚合桶
/// （而不是整层），但每个受影响的桶都按其覆盖的全部基础行全区间重算：
/// min/max 取极值，rms 按 `rms²×名义跨度` 加权（Audacity `processBlock` 同款；
/// 尾部不完整桶用名义跨度近似，误差只影响边缘一桶的 rms 显示）。
/// 全区间重算的代价是桶宽×受影响桶数（最粗层每批次约 5 万行求和），
/// 远小于一批次的解码量，因此不需要单调合并那套增量逻辑，结果恒精确。
/// 只在三值任一变化时写模型，避免冗余 `row_changed` 通知。
pub(crate) fn update_waveform_pyramid(
    base: &VecModel<SlintWaveformBucket>,
    levels: &[Rc<VecModel<SlintWaveformBucket>>],
    start: usize,
    changed_count: usize,
) {
    if changed_count == 0 {
        return;
    }
    let end = start.saturating_add(changed_count).min(base.row_count());
    for (level_index, target) in levels.iter().enumerate() {
        let bucket_ms = waveform_level_duration_ms(level_index);
        let first_bucket = start / bucket_ms;
        let last_bucket = end.saturating_sub(1) / bucket_ms;
        if target.row_count() <= last_bucket {
            target.extend(std::iter::repeat_n(
                SlintWaveformBucket::default(),
                last_bucket + 1 - target.row_count(),
            ));
        }
        for bucket in first_bucket..=last_bucket {
            let range_start = bucket * bucket_ms;
            let range_end = ((bucket + 1) * bucket_ms).min(base.row_count());
            let Some(next) = fold_waveform_bucket(base, range_start, range_end) else {
                continue;
            };
            let current = target.row_data(bucket).unwrap_or_default();
            if next.min != current.min || next.max != current.max || next.rms != current.rms {
                target.set_row_data(bucket, next);
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn bucket(min: f32, max: f32, rms: f32) -> SlintWaveformBucket {
        SlintWaveformBucket { min, max, rms }
    }

    /// 层 rms 的期望值：子桶 `rms²` 的均方根（名义等权，与实现同公式；
    /// 测的是“哪些行被聚合、除数是否正确”，不是 sqrt 本身）。
    fn rms(values: &[f32]) -> f32 {
        (values.iter().map(|v| (v * v) as f64).sum::<f64>() / values.len() as f64).sqrt() as f32
    }

    fn summarize(model: &VecModel<SlintWaveformBucket>) -> Vec<(f32, f32, f32)> {
        model.iter().map(|b| (b.min, b.max, b.rms)).collect()
    }

    fn assert_summary_eq(actual: &[(f32, f32, f32)], expected: &[(f32, f32, f32)]) {
        assert_eq!(
            actual.len(),
            expected.len(),
            "bucket count mismatch: {actual:?}"
        );
        for (i, ((amin, amax, arms), (emin, emax, erms))) in
            actual.iter().zip(expected.iter()).enumerate()
        {
            assert_eq!(
                (*amin, *amax),
                (*emin, *emax),
                "bucket {i} min/max mismatch"
            );
            assert!(
                (arms - erms).abs() < 1e-5,
                "bucket {i} rms mismatch: {arms} vs {erms}"
            );
        }
    }

    #[test]
    fn waveform_pyramid_folds_min_max_and_rms_exactly() {
        let base = VecModel::from(vec![
            bucket(-0.1, 0.1, 0.1),
            bucket(-0.2, 0.9, 0.5),
            bucket(-0.2, 0.2, 0.2),
            bucket(-0.3, 0.3, 0.4),
            bucket(-0.4, 0.4, 0.3),
            bucket(-0.1, 0.5, 0.5),
            bucket(-0.8, 0.8, 0.6),
            bucket(-0.1, 0.1, 0.2),
        ]);
        let levels = (0..3)
            .map(|_| Rc::new(VecModel::default()))
            .collect::<Vec<_>>();
        let identities = levels.iter().map(Rc::as_ptr).collect::<Vec<_>>();

        update_waveform_pyramid(&base, &levels, 0, base.row_count());
        assert_summary_eq(
            &summarize(&levels[0]),
            &[
                (-0.2, 0.9, rms(&[0.1, 0.5])),
                (-0.3, 0.3, rms(&[0.2, 0.4])),
                (-0.4, 0.5, rms(&[0.3, 0.5])),
                (-0.8, 0.8, rms(&[0.6, 0.2])),
            ],
        );
        assert_summary_eq(
            &summarize(&levels[1]),
            &[
                (-0.4, 0.9, rms(&[0.1, 0.5, 0.2, 0.4, 0.3])),
                (-0.8, 0.8, rms(&[0.5, 0.6, 0.2])),
            ],
        );
        assert_summary_eq(
            &summarize(&levels[2]),
            &[(-0.8, 0.9, rms(&[0.1, 0.5, 0.2, 0.4, 0.3, 0.5, 0.6, 0.2]))],
        );

        // 修订已有行：层桶按覆盖全区间重算，三个值一起更新。
        base.set_row_data(2, bucket(-1.0, 1.0, 0.9));
        update_waveform_pyramid(&base, &levels, 2, 1);
        assert_summary_eq(
            &summarize(&levels[0]),
            &[
                (-0.2, 0.9, rms(&[0.1, 0.5])),
                (-1.0, 1.0, rms(&[0.9, 0.4])),
                (-0.4, 0.5, rms(&[0.3, 0.5])),
                (-0.8, 0.8, rms(&[0.6, 0.2])),
            ],
        );
        assert_summary_eq(
            &summarize(&levels[1]),
            &[
                (-1.0, 1.0, rms(&[0.1, 0.5, 0.9, 0.4, 0.3])),
                (-0.8, 0.8, rms(&[0.5, 0.6, 0.2])),
            ],
        );
        assert_summary_eq(
            &summarize(&levels[2]),
            &[(-1.0, 1.0, rms(&[0.1, 0.5, 0.9, 0.4, 0.3, 0.5, 0.6, 0.2]))],
        );
        assert_eq!(
            identities,
            levels.iter().map(Rc::as_ptr).collect::<Vec<_>>()
        );
    }

    #[test]
    fn waveform_batch_uses_one_append_notification_and_preserves_gaps() {
        let values = VecModel::default();
        ui_apply_waveform_batch(
            &values,
            2,
            &[bucket(-0.25, 0.25, 0.2), bucket(-0.5, 0.5, 0.4)],
        );
        assert_summary_eq(
            &summarize(&values),
            &[
                (0.0, 0.0, 0.0),
                (0.0, 0.0, 0.0),
                (-0.25, 0.25, 0.2),
                (-0.5, 0.5, 0.4),
            ],
        );

        ui_apply_waveform_batch(
            &values,
            1,
            &[
                bucket(-0.75, 0.75, 0.6),
                bucket(-0.8, 0.8, 0.7),
                bucket(-0.9, 0.9, 0.8),
            ],
        );
        assert_summary_eq(
            &summarize(&values),
            &[
                (0.0, 0.0, 0.0),
                (-0.75, 0.75, 0.6),
                (-0.8, 0.8, 0.7),
                (-0.9, 0.9, 0.8),
            ],
        );
    }

    #[test]
    fn waveform_pyramid_recomputes_rms_for_later_batches() {
        let base = VecModel::from(vec![bucket(-0.1, 0.9, 0.5)]);
        let levels = vec![Rc::new(VecModel::default())];
        update_waveform_pyramid(&base, &levels, 0, 1);

        // 后续批次只抬高 max：min 保持，rms 按两行重算（非单调合并）。
        base.push(bucket(0.0, 0.2, 0.1));
        update_waveform_pyramid(&base, &levels, 1, 1);
        assert_summary_eq(&summarize(&levels[0]), &[(-0.1, 0.9, rms(&[0.5, 0.1]))]);

        // 新桶落到下一个 2ms 层桶：独立聚合，不污染前一个桶。
        base.push(bucket(-0.5, 0.0, 0.4));
        update_waveform_pyramid(&base, &levels, 2, 1);
        assert_summary_eq(
            &summarize(&levels[0]),
            &[(-0.1, 0.9, rms(&[0.5, 0.1])), (-0.5, 0.0, 0.4)],
        );
    }
}
