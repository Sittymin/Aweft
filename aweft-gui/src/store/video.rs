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

use crate::bridge::types::{SlintAudioTrack, SlintVideoPos, SlintVideoTimeBar};

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
    /// 波形的持久增量模型；`SlintAudioWaveform.amplitudes` 只引用这些模型。
    /// 后台任务按批次 push/set_row_data，不重复构造完整快照。
    pub waveform_amplitudes: Vec<Vec<Rc<VecModel<f32>>>>,
    /// 与每个视频对应的持久多分辨率峰值层（2/5/10/20/50ms……）。
    /// 缩放只切换读取层级，不重新聚合或替换模型。
    pub waveform_level_amplitudes: Vec<Vec<Vec<Rc<VecModel<f32>>>>>,
}

/// 将后台波形批次原地写入基础模型，尽量把连续追加合并成一次 Slint
/// `row_added` 通知。只有覆盖已有行的修订才需要逐行 `row_changed`。
pub(crate) fn ui_apply_waveform_batch(values: &VecModel<f32>, start: usize, amplitudes: &[f32]) {
    if amplitudes.is_empty() {
        return;
    }
    let current_len = values.row_count();
    if start > current_len {
        values.extend(std::iter::repeat_n(0.0, start - current_len));
    }

    let overlap_end = (start + amplitudes.len()).min(values.row_count());
    for (row, amplitude) in (start..overlap_end).zip(amplitudes.iter().copied()) {
        values.set_row_data(row, amplitude);
    }

    let append_start = start.max(values.row_count());
    if append_start < start + amplitudes.len() {
        let offset = append_start - start;
        values.extend(amplitudes[offset..].iter().copied());
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

/// 按变更区间增量更新 1/2/5×10^k 峰值层。每一层只遍历受影响的聚合桶，
/// 而不是让每个基础桶都穿过全部 24 层；长音频批次因此保持近似线性成本。
pub(crate) fn update_peak_pyramid(
    base: &VecModel<f32>,
    levels: &[Rc<VecModel<f32>>],
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
                0.0,
                last_bucket + 1 - target.row_count(),
            ));
        }
        for bucket in first_bucket..=last_bucket {
            let base_start = (bucket * bucket_ms).max(start);
            let base_end = ((bucket + 1) * bucket_ms).min(end);
            let peak = (base_start..base_end)
                .filter_map(|row| base.row_data(row))
                .fold(0.0f32, f32::max);
            if peak > target.row_data(bucket).unwrap_or(0.0) {
                target.set_row_data(bucket, peak);
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn peak_pyramid_preserves_transients_and_reuses_models() {
        let base = VecModel::from(vec![0.1, 0.9, 0.2, 0.3, 0.4, 0.5, 0.8, 0.1]);
        let levels = (0..3)
            .map(|_| Rc::new(VecModel::default()))
            .collect::<Vec<_>>();
        let identities = levels.iter().map(Rc::as_ptr).collect::<Vec<_>>();

        update_peak_pyramid(&base, &levels, 0, base.row_count());
        assert_eq!(
            levels[0].iter().collect::<Vec<_>>(),
            vec![0.9, 0.3, 0.5, 0.8]
        );
        assert_eq!(levels[1].iter().collect::<Vec<_>>(), vec![0.9, 0.8]);
        assert_eq!(levels[2].iter().collect::<Vec<_>>(), vec![0.9]);

        base.set_row_data(2, 1.0);
        update_peak_pyramid(&base, &levels, 2, 1);
        assert_eq!(levels[0].row_data(1), Some(1.0));
        assert_eq!(levels[1].row_data(0), Some(1.0));
        assert_eq!(levels[2].row_data(0), Some(1.0));
        assert_eq!(
            identities,
            levels.iter().map(Rc::as_ptr).collect::<Vec<_>>()
        );
    }

    #[test]
    fn waveform_batch_uses_one_append_notification_and_preserves_gaps() {
        let values = VecModel::default();
        ui_apply_waveform_batch(&values, 2, &[0.25, 0.5]);
        assert_eq!(values.iter().collect::<Vec<_>>(), vec![0.0, 0.0, 0.25, 0.5]);

        ui_apply_waveform_batch(&values, 1, &[0.75, 0.8, 0.9]);
        assert_eq!(values.iter().collect::<Vec<_>>(), vec![0.0, 0.75, 0.8, 0.9],);
    }

    #[test]
    fn peak_pyramid_keeps_a_larger_peak_from_an_earlier_batch() {
        let base = VecModel::from(vec![0.9]);
        let levels = vec![Rc::new(VecModel::default())];
        update_peak_pyramid(&base, &levels, 0, 1);

        base.push(0.2);
        update_peak_pyramid(&base, &levels, 1, 1);

        assert_eq!(levels[0].iter().collect::<Vec<_>>(), vec![0.9]);
    }
}
