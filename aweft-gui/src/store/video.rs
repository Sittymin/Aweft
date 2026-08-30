//! 视频域 UI 状态：每个视频会话派生到 Slint `VideoState` 的模型集合。
//! Rust 侧持有 VecModel（共享 inner 模型，变更自动通知 Slint）：
//! - `frames`/`widths`/`heights`：最新帧与尺寸（渲染线程逐帧 `set_row_data`）
//! - `positions`：视口世界坐标位置（级联错开/拖拽移动）
//! - `time_bars`：各面板控制时间条（选中/主视频/时间，reducer 与每帧同步写）
//!
//! 会话增删等结构变化由 `reducers/media` 用 `push`/`set_row_data` 局部更新
//! （不重建既有行，避免拖拽/帧模型被打断）；时间条逐帧更新走 `update_time_bars`。

use std::rc::Rc;

use slint::{Image, Model, VecModel};

use crate::bridge::types::{SlintAudioWaveform, SlintVideoPos, SlintVideoTimeBar};

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
    /// 每个会话解码后的音频峰值包络。
    pub audio_waveforms: Rc<VecModel<SlintAudioWaveform>>,
    /// 波形的持久增量模型；`SlintAudioWaveform.amplitudes` 只引用这些模型。
    /// 后台任务按批次 push/set_row_data，不重复构造完整快照。
    pub waveform_amplitudes: Vec<Rc<VecModel<f32>>>,
    /// 与每个视频对应的持久多分辨率峰值层（200ms、400ms……）。
    /// 缩放只切换读取层级，不重新聚合或替换模型。
    pub waveform_level_amplitudes: Vec<Vec<Rc<VecModel<f32>>>>,
}

/// 按变更区间增量更新二倍峰值金字塔。每层只读取上一层相邻的两个桶，
/// 不从基础模型反复扫描整段数据；所有层合计工作量小于基础变更量的两倍。
/// 即根据base来计算下一层粗粒度的新增变化
pub(crate) fn update_peak_pyramid(
    base: &VecModel<f32>,
    levels: &[Rc<VecModel<f32>>],
    start: usize,
    changed_count: usize,
) {
    if changed_count == 0 {
        return;
    }
    let mut source_start = start;
    let mut source_end = start + changed_count;

    for (level_index, target) in levels.iter().enumerate() {
        let first = source_start / 2;
        let last = source_end.saturating_sub(1) / 2;
        let source: &VecModel<f32> = if level_index == 0 {
            base
        } else {
            levels[level_index - 1].as_ref()
        };

        for bucket in first..=last {
            let left = source.row_data(bucket * 2).unwrap_or(0.0);
            let right = source.row_data(bucket * 2 + 1).unwrap_or(0.0);
            let peak = left.max(right);
            if bucket < target.row_count() {
                target.set_row_data(bucket, peak);
            } else if bucket == target.row_count() {
                target.push(peak);
            }
        }

        source_start = first;
        source_end = last + 1;
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
}
