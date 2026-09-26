//! 音频域 UI 状态：每个视频会话的音轨行与波形桶模型集合，派生到 Slint `AudioState`。
//! Rust 侧持有 VecModel（共享 inner 模型，变更自动通知 Slint）：
//! - `tracks`：每个视频的音轨行（外层下标 = 会话下标，内层 = 该容器内的音轨）
//! - `track_models`：与 `tracks` 各行共享 inner 的持久行模型
//! - `buckets`：每个音轨的 1ms 基础桶（后台任务按批次 push/set_row_data，耐久层）
//!
//! 可见列不在这里存储：Slint 侧按当前缩放把可视窗口（桶宽、首桶、列数）发给
//! `ui_compute_waveform_columns` 现算（Audacity 式按需列），不预建全文件金字塔。
//!
//! 音频模型的结构变化由 `projections/audio` 用 `push`/`set_row_data` 局部更新
//! （不重建既有行，避免波形模型被打断）；波形后台批次属于实时渲染增量例外，
//! 只允许行级更新（见 `docs/architecture/video-playback.md`）。

use std::cell::RefCell;
use std::collections::HashMap;
use std::rc::Rc;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};

use futures::StreamExt as _;
use slint::{Model, VecModel};

use crate::bridge::types::{SlintAreaType, SlintAudioTrack, SlintWaveformBucket};
use crate::screen::area::ScrAreaKey;
use crate::store::AppStore;

/// 单个波形 Area 上报的可视窗口：列宽（基础桶数）、首列、列数。
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct AreaWaveformView {
    pub(crate) video_index: usize,
    pub(crate) track_index: usize,
    pub(crate) bucket_ms: usize,
    pub(crate) first: i32,
    pub(crate) count: usize,
}

#[derive(Default)]
pub struct AudioUiState {
    /// 每个视频的音轨行（外层下标 = 会话下标；行内嵌该视频各音轨的波形模型）。
    /// 与 Slint `AudioState.audio-tracks` 共享 inner。
    pub tracks: Rc<VecModel<slint::ModelRc<SlintAudioTrack>>>,
    /// 每个视频内部的音轨行模型；`tracks` 各行引用对应模型。
    pub track_models: Vec<Rc<VecModel<SlintAudioTrack>>>,
    /// 波形的持久增量模型；可见列计算的数据源。
    /// 后台任务按批次 push/set_row_data，不重复构造完整快照。
    pub buckets: Vec<Vec<Rc<VecModel<SlintWaveformBucket>>>>,
    /// 各波形 Area 上报的可视窗口（`slotmap` 键标识 Area）。
    pub views: HashMap<ScrAreaKey, AreaWaveformView>,
    /// 各波形 Area 的持久列模型（与上报同键，Slint 绑定恒等，行级增量更新）。
    pub columns: HashMap<ScrAreaKey, Rc<VecModel<SlintWaveformBucket>>>,
    /// 各视频会话当前播放音轨下标模型（与 Slint AudioState.playback-tracks 共享 inner）。
    pub playback_tracks: Rc<VecModel<i32>>,
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

/// 按可视窗口现算波形列（Audacity 式按需列）：第 `j` 列聚合基础行
/// `[(first + j) * bucket_ms, (first + j + 1) * bucket_ms)`。
/// 窗口完全落在 `[0, base_len)` 之外的列填零桶（静音，显示为中心零线）；
/// 部分相交的列只聚合实际存在的行，显示值精确对应其覆盖区间。
/// 返回列数恒等于 `count`，调用方按下标直读，无需对齐。
pub(crate) fn ui_compute_waveform_columns(
    base: &VecModel<SlintWaveformBucket>,
    bucket_ms: usize,
    first: i32,
    count: usize,
) -> Vec<SlintWaveformBucket> {
    let bucket_ms = bucket_ms.max(1);
    let base_len = base.row_count() as i64;
    let mut columns = Vec::with_capacity(count);
    for j in 0..count {
        let range_start = (first as i64 + j as i64) * bucket_ms as i64;
        let range_end = range_start + bucket_ms as i64;
        if range_end <= 0 || range_start >= base_len {
            columns.push(SlintWaveformBucket::default());
            continue;
        }
        let range_start = range_start.max(0) as usize;
        let range_end = range_end.min(base_len) as usize;
        columns.push(fold_waveform_bucket(base, range_start, range_end).unwrap_or_default());
    }
    columns
}

/// 按视图同步单个 Area 的持久列模型：数量变化走 `insert`/`remove` 局部增删
/// （存量组件复用），行值走 `set_row_data` 原地更新，模型标识恒不变，
/// Slint 侧不重建槽位组件。返回该 Area 的列模型。
/// 视图越界（视频/音轨下标非法、基础模型缺失）时返回空模型，不建注册项。
pub(crate) fn ui_sync_waveform_columns(
    audio: &mut AudioUiState,
    key: ScrAreaKey,
    view: AreaWaveformView,
) -> Rc<VecModel<SlintWaveformBucket>> {
    let columns = audio
        .columns
        .entry(key)
        .or_insert_with(|| Rc::new(VecModel::default()));
    let Some(base) = audio
        .buckets
        .get(view.video_index)
        .and_then(|tracks| tracks.get(view.track_index))
    else {
        columns.clear();
        return Rc::clone(columns);
    };
    let fresh = ui_compute_waveform_columns(&base, view.bucket_ms, view.first, view.count);
    while columns.row_count() > fresh.len() {
        columns.remove(columns.row_count() - 1);
    }
    while columns.row_count() < fresh.len() {
        columns.push(SlintWaveformBucket::default());
    }
    for (row, bucket) in fresh.iter().enumerate() {
        if columns.row_data(row).as_ref() != Some(bucket) {
            columns.set_row_data(row, bucket.clone());
        }
    }
    audio.views.insert(key, view);
    Rc::clone(columns)
}

/// 裁剪已不存在或已切走的波形 Area 注册项（视图与列模型一并丢弃）。
/// 无头测试与 Area 关闭后调用，pump 每批顺带执行，开销与 Area 数成正比。
pub(crate) fn ui_prune_waveform_views(store: &mut AppStore) {
    let dead: Vec<ScrAreaKey> = store
        .ui_models
        .audio
        .views
        .keys()
        .filter_map(|key| {
            let alive = store
                .ui_models
                .scrarea_index_of(key)
                .and_then(|index| store.ui_models.scrarea_model.row_data(index))
                .map(|area| area.area_type == SlintAreaType::AudioWaveform)
                .unwrap_or(false);
            if alive { None } else { Some(*key) }
        })
        .collect();
    if dead.is_empty() {
        return;
    }
    let audio = &mut store.ui_models.audio;
    for key in dead {
        audio.views.remove(&key);
        audio.columns.remove(&key);
    }
}

/// 波形后台 worker 发往 UI 线程的批次（`Send`，经无界通道投递）。
/// 体积与媒体时长成线性（约四秒音频一批），顺序与 worker 发送顺序一致。
pub struct WaveformBatch {
    pub(crate) video_index: usize,
    pub(crate) track_index: usize,
    pub(crate) start: usize,
    pub(crate) duration: f32,
    pub(crate) buckets: Vec<SlintWaveformBucket>,
    pub(crate) cancellation: Arc<AtomicBool>,
    pub(crate) completed: bool,
}

/// 波形批次的 UI 线程 pump 任务（`bridge` 建通道并 `spawn_local` 启动）。
/// 逐条做取消检查后行级写模型：逻辑与原 `on_waveform_progress` 处理函数一致，
/// 只是入口从 Slint 回调换成了 Rust 通道。`store` 每次按消息现借，保证替换媒体后
/// 写到最新的模型上。
pub(crate) async fn ui_pump_waveform_batches(
    store: Rc<RefCell<AppStore>>,
    mut batches: futures::channel::mpsc::UnboundedReceiver<WaveformBatch>,
) {
    let mut ui_log_started_at = std::time::Instant::now();
    let mut ui_batch_count = 0usize;
    let mut ui_row_count = 0usize;
    while let Some(batch) = batches.next().await {
        let ui_batch_started_at = std::time::Instant::now();
        if batch.cancellation.load(Ordering::Acquire) {
            continue;
        }
        // 基础模型经共享 Rc 原地写，不借 store；替换媒体后旧模型已整体重建，
        // 此处拿到的 Rc 恒为当前会话的模型。
        let Some(base) = store
            .borrow()
            .ui_models
            .audio
            .buckets
            .get(batch.video_index)
            .and_then(|tracks| tracks.get(batch.track_index))
            .cloned()
        else {
            log::warn!(
                "丢弃越界波形结果: video-index={}, track-index={}",
                batch.video_index,
                batch.track_index
            );
            continue;
        };
        ui_apply_waveform_batch(&base, batch.start, &batch.buckets);
        {
            let store = store.borrow();
            let audio = &store.ui_models.audio;
            if let Some(model) = audio.track_models.get(batch.video_index)
                && let Some(mut track) = model.row_data(batch.track_index)
            {
                let mut changed = false;
                if (track.waveform.duration - batch.duration).abs() > f32::EPSILON {
                    track.waveform.duration = batch.duration;
                    changed = true;
                }
                let target_status = if batch.completed {
                    crate::bridge::types::AudioTrackWaveformStatus::Completed
                } else {
                    crate::bridge::types::AudioTrackWaveformStatus::Processing
                };
                if track.waveform_status != target_status {
                    track.waveform_status = target_status;
                    changed = true;
                }
                if changed {
                    model.set_row_data(batch.track_index, track);
                }
            }
        }
        // 重算上报过且同视频同音轨的视图列模型，并裁剪失效 Area。
        {
            let mut store = store.borrow_mut();
            ui_prune_waveform_views(&mut store);
            let views: Vec<(ScrAreaKey, AreaWaveformView)> = store
                .ui_models
                .audio
                .views
                .iter()
                .map(|(key, view)| (*key, *view))
                .collect();
            for (key, view) in views {
                if view.video_index != batch.video_index || view.track_index != batch.track_index {
                    continue;
                }
                ui_sync_waveform_columns(&mut store.ui_models.audio, key, view);
            }
        }
        ui_batch_count += 1;
        ui_row_count += batch.buckets.len();
        if ui_log_started_at.elapsed().as_secs_f32() >= 1.0 {
            log::info!(
                "波形 UI 更新进度: batches={}, rows={}, last_batch_rows={}, apply_wall={:.3}ms",
                ui_batch_count,
                ui_row_count,
                batch.buckets.len(),
                ui_batch_started_at.elapsed().as_secs_f64() * 1_000.0,
            );
            ui_batch_count = 0;
            ui_row_count = 0;
            ui_log_started_at = std::time::Instant::now();
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

    fn summarize_vec(columns: &[SlintWaveformBucket]) -> Vec<(f32, f32, f32)> {
        columns.iter().map(|b| (b.min, b.max, b.rms)).collect()
    }

    #[test]
    fn compute_columns_folds_visible_window_exactly() {
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
        // 2ms 桶宽取四列：每列聚合两行，min/max 取极值、rms 取均方根。
        assert_summary_eq(
            &summarize_vec(&ui_compute_waveform_columns(&base, 2, 0, 4)),
            &[
                (-0.2, 0.9, rms(&[0.1, 0.5])),
                (-0.3, 0.3, rms(&[0.2, 0.4])),
                (-0.4, 0.5, rms(&[0.3, 0.5])),
                (-0.8, 0.8, rms(&[0.6, 0.2])),
            ],
        );
        // 1ms 桶宽即恒等拷贝。
        assert_summary_eq(
            &summarize_vec(&ui_compute_waveform_columns(&base, 1, 2, 3)),
            &[(-0.2, 0.2, 0.2), (-0.3, 0.3, 0.4), (-0.4, 0.4, 0.3)],
        );
        // 窗口中段：首列从行 3 起。
        assert_summary_eq(
            &summarize_vec(&ui_compute_waveform_columns(&base, 2, 1, 2)),
            &[(-0.3, 0.3, rms(&[0.2, 0.4])), (-0.4, 0.5, rms(&[0.3, 0.5]))],
        );
    }

    #[test]
    fn compute_columns_pads_outside_with_silence() {
        let base = VecModel::from(vec![bucket(-0.5, 0.5, 0.4); 4]);
        // 首桶为负：左侧越界列填零桶，首个相交列只聚合实际存在的行。
        assert_summary_eq(
            &summarize_vec(&ui_compute_waveform_columns(&base, 2, -1, 3)),
            &[(0.0, 0.0, 0.0), (-0.5, 0.5, 0.4), (-0.5, 0.5, 0.4)],
        );
        // 尾部越界列填零桶。
        assert_summary_eq(
            &summarize_vec(&ui_compute_waveform_columns(&base, 2, 0, 3)),
            &[(-0.5, 0.5, 0.4), (-0.5, 0.5, 0.4), (0.0, 0.0, 0.0)],
        );
        // 空基础全填零桶，返回列数仍等于请求数。
        let empty = VecModel::default();
        let padded = ui_compute_waveform_columns(&empty, 2, 0, 3);
        assert_eq!(padded.len(), 3);
        assert_summary_eq(
            &summarize_vec(&padded),
            &[(0.0, 0.0, 0.0), (0.0, 0.0, 0.0), (0.0, 0.0, 0.0)],
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

    fn test_key(n: u64) -> ScrAreaKey {
        slotmap::KeyData::from_ffi(n).into()
    }

    fn sync_case(base_rows: usize) -> (AudioUiState, ScrAreaKey) {
        let mut audio = AudioUiState::default();
        audio.buckets.push(vec![Rc::new(VecModel::from(vec![
            bucket(-0.5, 0.5, 0.4);
            base_rows
        ]))]);
        (audio, test_key(1))
    }

    #[test]
    fn sync_columns_reuses_model_and_updates_in_place() {
        let (mut audio, key) = sync_case(8);
        let view = AreaWaveformView {
            video_index: 0,
            track_index: 0,
            bucket_ms: 2,
            first: 0,
            count: 4,
        };
        let first = ui_sync_waveform_columns(&mut audio, key, view);
        assert_summary_eq(
            &summarize(&first),
            &[
                (-0.5, 0.5, 0.4),
                (-0.5, 0.5, 0.4),
                (-0.5, 0.5, 0.4),
                (-0.5, 0.5, 0.4),
            ],
        );
        // 同一视图再次同步：模型标识不变（Slint 侧不重建）。
        let second = ui_sync_waveform_columns(&mut audio, key, view);
        assert!(Rc::ptr_eq(&first, &second));
        assert_eq!(second.row_count(), 4);
    }

    #[test]
    fn sync_columns_grows_and_shrinks_by_splice() {
        let (mut audio, key) = sync_case(8);
        let narrow = AreaWaveformView {
            video_index: 0,
            track_index: 0,
            bucket_ms: 2,
            first: 0,
            count: 2,
        };
        let model = ui_sync_waveform_columns(&mut audio, key, narrow);
        assert_eq!(model.row_count(), 2);
        // 视窗扩大：尾部追加行，存量行复用。
        let wide = AreaWaveformView { count: 4, ..narrow };
        let model = ui_sync_waveform_columns(&mut audio, key, wide);
        assert_eq!(model.row_count(), 4);
        assert!(Rc::ptr_eq(
            &model,
            audio.columns.get(&key).expect("列模型应保留")
        ));
        // 视窗缩小：尾部删除行。
        let model = ui_sync_waveform_columns(&mut audio, key, narrow);
        assert_eq!(model.row_count(), 2);
    }

    #[test]
    fn sync_columns_with_unknown_video_returns_empty() {
        let (mut audio, key) = sync_case(8);
        let view = AreaWaveformView {
            video_index: 3,
            track_index: 0,
            bucket_ms: 2,
            first: 0,
            count: 4,
        };
        let model = ui_sync_waveform_columns(&mut audio, key, view);
        assert_eq!(model.row_count(), 0);
    }
}
