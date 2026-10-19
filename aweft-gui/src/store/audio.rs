//! 音频域 UI 状态：每个视频会话的音轨行与波形桶模型集合，派生到 Slint `AudioState`。
//! Rust 侧持有 VecModel（共享 inner 模型，变更自动通知 Slint）：
//! - `tracks`：每个视频的音轨行（外层下标 = 会话下标，内层 = 该容器内的音轨）
//! - `track_models`：与 `tracks` 各行共享 inner 的持久行模型
//! - `waveform_tracks`：每个视频每条音轨一个 `WaveformTrack`，里面同时装着
//!   1ms 基础桶（后台任务按批次原地写入的普通 `Vec` 耐久层，从不绑定到 Slint，
//!   行读取无锁无通知开销）与它的已折算列缓存
//!
//! 可见列不整窗重算：缓存按键 `(bucket_ms, 绝对列号)` 存已折算的列值，
//! 平移、回移与批次到达都只折算新出现或被新数据覆盖的列（Audacity 式按需列的
//! 增量版），不预建全文件金字塔。
//!
//! 音频模型的结构变化由 `projections/audio` 用 `push`/`set_row_data` 局部更新
//! （不重建既有行，避免波形模型被打断）；波形后台批次属于实时渲染增量例外，
//! 只允许行级更新（见 `docs/architecture/video-playback.md`）。

use std::cell::RefCell;
use std::collections::{BTreeMap, HashMap};
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
    /// 每条音轨的波形：外层下标 = 视频会话，内层下标 = 该视频内的音轨，与
    /// `track_models` 各行一一对应。耐久层与列缓存同持在一个 `WaveformTrack` 里。
    pub waveform_tracks: Vec<Vec<WaveformTrack>>,
    /// 各波形 Area 上报的可视窗口（`slotmap` 键标识 Area）。
    pub views: HashMap<ScrAreaKey, AreaWaveformView>,
    /// 各波形 Area 的持久列模型与当前对应的窗口快照（与上报同键，Slint 绑定恒等，
    /// 行级增量更新）。
    pub columns: HashMap<ScrAreaKey, AreaColumns>,
    /// 各视频会话当前播放音轨下标模型（与 Slint AudioState.playback-tracks 共享 inner）。
    pub playback_tracks: Rc<VecModel<i32>>,
}

/// 单个波形 Area 的持久列模型与模型内容当前对应的窗口快照。
/// `window` 为 `None` 表示内容失效（基础模型已重建或视图未同步过），
/// 下一次刷新必须整窗重填。
pub struct AreaColumns {
    pub(crate) model: Rc<VecModel<SlintWaveformBucket>>,
    pub(crate) window: Option<AreaWaveformView>,
}

/// 一条音轨的波形：1ms 基础桶耐久层，加上按缩放档分组的已折算列。
///
/// 折算列以 `(bucket_ms, 绝对列号)` 为键，平移、回移与缩放档回切命中即免折算。
/// 失效只有一个来源，也只有一个去处：耐久层的行变了（追加与原地修正都算），就把
/// 与之相交的列从缓存里丢掉（`apply_batch`）。所以条目不需要记录折算时的行数或
/// 版本号，读侧也不需要判断「这列还新鲜吗」——查得到就是新鲜的。
///
/// 耐久层与缓存合持在一个对象里、且只有 `apply_batch` 一条写入口，为的是让
/// 「写行」与「丢受影响的列」不可能被分开做、分开忘。
pub struct WaveformTrack {
    /// 耐久层：按行号索引的普通 `Vec`，从不绑定到 Slint，写一行不触发任何通知。
    base: Vec<SlintWaveformBucket>,
    /// 已折算列，外层键 = `bucket_ms`（缩放档）：一档一个范围域。内层用 `BTreeMap`
    /// 是为了让失效能按列号区间只取真正存在的键——追加批次写的那 4096 行通常在
    /// 可见窗口之外，用 `HashMap` 就得对整段区间逐个试探不存在的键。
    folded: HashMap<usize, BTreeMap<i64, CachedColumn>>,
    /// LRU 时钟戳：每次取一列加一，淘汰时按它丢最旧的一半。
    clock: u64,
    /// 见过的最宽视口列数，作为驻留上限的基准（由 `fill_columns`/`refresh_columns`
    /// 在取列之前写入，所以缓存还没有条目时它已经就位）。
    widest_window: usize,
}

struct CachedColumn {
    value: SlintWaveformBucket,
    last_used: u64,
}

/// 每轨驻留多少个「最宽视口」的折算结果：足够两三个缩放档同时留着（切档回切
/// 免折算），上限随实际窗口大小派生，与媒体时长无关。
const WAVEFORM_CACHE_WINDOWS: usize = 8;

impl WaveformTrack {
    /// 新建一条还没有任何基础桶的音轨（`projections/audio` 建模时用）。
    pub(crate) fn new() -> Self {
        Self::with_base(Vec::new())
    }

    fn with_base(base: Vec<SlintWaveformBucket>) -> Self {
        Self {
            base,
            folded: HashMap::new(),
            clock: 0,
            widest_window: 0,
        }
    }

    /// 写入一批基础桶（自行号 `start` 起，间隙补零桶），并立即丢掉与之相交的
    /// 已折算列。返回本批实际写入（含间隙填充段）的行区间 `[lo, hi)`，
    /// 调用方用它决定重绘哪些列。
    #[must_use]
    pub(crate) fn apply_batch(
        &mut self,
        start: usize,
        buckets: &[SlintWaveformBucket],
    ) -> (usize, usize) {
        let (lo, hi) = ui_write_base_rows(&mut self.base, start, buckets);
        self.drop_folded(lo, hi);
        (lo, hi)
    }

    /// 丢掉覆盖到 `[lo, hi)` 内任一行的折算列。列 `c` 覆盖 `[c*b, (c+1)*b)`，与之
    /// 相交即 `c ∈ [lo/b, (hi-1)/b]`，所以每档只摘这一段的键；代价与该档里真正
    /// 相交的条目数成正比，与批次行跨度无关。
    /// 追加而变长的行也落在 `[lo, hi)` 里，所以「半截列补全」不需要额外判据。
    fn drop_folded(&mut self, lo: usize, hi: usize) {
        if hi <= lo {
            return;
        }
        for (bucket_ms, group) in self.folded.iter_mut() {
            let bucket_ms = (*bucket_ms).max(1);
            let column_lo = (lo / bucket_ms) as i64;
            let column_hi = ((hi - 1) / bucket_ms) as i64;
            let doomed: Vec<i64> = group
                .range(column_lo..=column_hi)
                .map(|(column, _)| *column)
                .collect();
            for column in doomed {
                group.remove(&column);
            }
        }
    }

    /// 取一列的折算值：查得到就用，查不到只现折这一列，不碰其它列。
    /// 新鲜性不由读侧判断——被改写过的列在 `apply_batch` 里就已经被丢掉了。
    fn get_or_fold(&mut self, bucket_ms: usize, column: i64) -> SlintWaveformBucket {
        self.clock += 1;
        if let Some(entry) = self
            .folded
            .get_mut(&bucket_ms)
            .and_then(|group| group.get_mut(&column))
        {
            entry.last_used = self.clock;
            return entry.value.clone();
        }
        let value = fold_waveform_column(&self.base, bucket_ms, column);
        self.folded.entry(bucket_ms).or_default().insert(
            column,
            CachedColumn {
                value: value.clone(),
                last_used: self.clock,
            },
        );
        if self.entry_count() > WAVEFORM_CACHE_WINDOWS * self.widest_window.max(1) {
            self.evict_older_half();
        }
        value
    }

    fn entry_count(&self) -> usize {
        self.folded.values().map(|group| group.len()).sum()
    }

    /// 超出驻留上限时按 LRU 时钟戳跨缩放档一起丢掉最旧的一半。
    /// 只要中位数，所以用 `select_nth_unstable` 而不是全排序。
    fn evict_older_half(&mut self) {
        let mut stamps: Vec<u64> = self
            .folded
            .values()
            .flat_map(|group| group.values())
            .map(|entry| entry.last_used)
            .collect();
        let median = stamps.len() / 2;
        stamps.select_nth_unstable(median);
        let threshold = stamps[median];
        for group in self.folded.values_mut() {
            group.retain(|_, entry| entry.last_used > threshold);
        }
    }

    /// 把一列的折算值写进模型第 `j` 槽：值没变就不发通知。
    /// `widest_window` 必须在取列之前抬到位，它决定驻留上限。
    fn set_column(
        &mut self,
        model: &VecModel<SlintWaveformBucket>,
        j: usize,
        view: AreaWaveformView,
    ) {
        let value = self.get_or_fold(view.bucket_ms, view.first as i64 + j as i64);
        if model.row_data(j).as_ref() != Some(&value) {
            model.set_row_data(j, value);
        }
    }

    /// 整窗重填一个 Area 的列模型：逐列经缓存取值（命中免折算），
    /// 行值变化才 `set_row_data`（原地更新，Slint 侧不重建槽位组件）。
    fn fill_columns(&mut self, model: &VecModel<SlintWaveformBucket>, view: AreaWaveformView) {
        self.widest_window = self.widest_window.max(view.count);
        resize_column_model(model, view.count);
        for j in 0..view.count {
            self.set_column(model, j, view);
        }
    }

    /// 只刷新与写入行区间 `[row_lo, row_hi)` 相交的列（相交判据同 `drop_folded`），
    /// 再裁进当前窗口。区间外的列一列不折、一次通知也不发。
    fn refresh_columns(
        &mut self,
        model: &VecModel<SlintWaveformBucket>,
        view: AreaWaveformView,
        row_lo: usize,
        row_hi: usize,
    ) {
        self.widest_window = self.widest_window.max(view.count);
        let bucket_ms = view.bucket_ms.max(1);
        let first = view.first as i64;
        let j_lo = ((row_lo / bucket_ms) as i64 - first).max(0) as usize;
        let j_hi =
            (((row_hi - 1) / bucket_ms) as i64 - first + 1).clamp(0, view.count as i64) as usize;
        for j in j_lo..j_hi {
            self.set_column(model, j, view);
        }
    }
}

/// 把一批桶写进耐久层：间隙用零桶（静音）填充，与提取端空桶语义一致；返回本批
/// 实际写入（含间隙填充）的行区间 `[lo, hi)`。
///
/// 只写行、不碰缓存，所以唯一的调用者是 [`WaveformTrack::apply_batch`]——那里把
/// 写行与丢受影响的列合成一步。别处直接调它就是漏失效。
fn ui_write_base_rows(
    values: &mut Vec<SlintWaveformBucket>,
    start: usize,
    buckets: &[SlintWaveformBucket],
) -> (usize, usize) {
    if buckets.is_empty() {
        return (start, start);
    }
    let old_len = values.len();
    if start > old_len {
        values.resize(start, SlintWaveformBucket::default());
    }
    // 覆盖段：行号落在旧耐久层内的行；追加段：其余行直接接在尾部。
    let overlap_end = (start + buckets.len()).min(old_len.max(start));
    if start < overlap_end {
        values[start..overlap_end].clone_from_slice(&buckets[..overlap_end - start]);
    }
    if start + buckets.len() > overlap_end {
        values.extend_from_slice(&buckets[overlap_end - start..]);
    }
    (start.min(old_len), start + buckets.len())
}

/// 将 `[range_start, range_end)` 内基础行的 min/max 取极值、rms 按平方加权平均；
/// 区间内无有效行返回 `None`。
fn fold_waveform_bucket(
    base: &[SlintWaveformBucket],
    range_start: usize,
    range_end: usize,
) -> Option<SlintWaveformBucket> {
    let mut folded_min = f32::INFINITY;
    let mut folded_max = f32::NEG_INFINITY;
    let mut folded_sumsq = 0.0f64;
    let mut folded_count = 0usize;
    for entry in &base[range_start..range_end] {
        folded_min = folded_min.min(entry.min);
        folded_max = folded_max.max(entry.max);
        folded_sumsq += (entry.rms as f64) * (entry.rms as f64);
        folded_count += 1;
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

/// 折算一个绝对列：第 `column` 列聚合基础行
/// `[column * bucket_ms, (column + 1) * bucket_ms)`。
/// 窗口完全落在 `[0, base_len)` 之外的列填零桶（静音，显示为中心零线）；
/// 部分相交的列只聚合实际存在的行，显示值精确对应其覆盖区间。
fn fold_waveform_column(
    base: &[SlintWaveformBucket],
    bucket_ms: usize,
    column: i64,
) -> SlintWaveformBucket {
    let bucket_ms = bucket_ms.max(1);
    let range_start = column * bucket_ms as i64;
    let range_end = range_start + bucket_ms as i64;
    let base_len = base.len() as i64;
    if range_end <= 0 || range_start >= base_len {
        return SlintWaveformBucket::default();
    }
    let range_start = range_start.max(0) as usize;
    let range_end = range_end.min(base_len) as usize;
    fold_waveform_bucket(base, range_start, range_end).unwrap_or_default()
}

/// 按可视窗口整窗折算列（`fold_waveform_column` 的批量形式）：第 `j` 列聚合基础行
/// `[(first + j) * bucket_ms, (first + j + 1) * bucket_ms)`。返回列数恒等于 `count`，
/// 调用方按下标直读，无需对齐。
///
/// 只有测试用。整窗重填走 [`WaveformTrack::fill_columns`]，它逐列经缓存取值，
/// 命中的列根本不折算，所以不经这里。
#[cfg(test)]
fn ui_compute_waveform_columns(
    base: &[SlintWaveformBucket],
    bucket_ms: usize,
    first: i32,
    count: usize,
) -> Vec<SlintWaveformBucket> {
    (0..count)
        .map(|j| fold_waveform_column(base, bucket_ms, first as i64 + j as i64))
        .collect()
}

/// 把列模型行数对齐到 `count`：数量变化走 `insert`/`remove` 局部增删
/// （存量组件复用），行值由调用方按行更新，模型标识恒不变。
fn resize_column_model(columns: &VecModel<SlintWaveformBucket>, count: usize) {
    while columns.row_count() > count {
        columns.remove(columns.row_count() - 1);
    }
    while columns.row_count() < count {
        columns.push(SlintWaveformBucket::default());
    }
}

/// 按视图同步单个 Area 的持久列模型（Slint `waveform-view` 回调入口）。
/// 视图与上次一致时直接返回（基础桶变化走 `ui_refresh_waveform_columns_for_batch`，
/// 不会经过本函数）；视图变化（平移/缩放/换轨）时整窗经缓存重填，重叠列命中缓存
/// 免折算，只有新进入窗口或长出新数据的列现折。视图越界（视频/音轨下标非法、
/// 基础桶缺失）时清空模型、失效窗口，不建注册项。
pub(crate) fn ui_sync_waveform_columns(
    audio: &mut AudioUiState,
    key: ScrAreaKey,
    view: AreaWaveformView,
) -> Rc<VecModel<SlintWaveformBucket>> {
    let AudioUiState {
        waveform_tracks,
        columns,
        views,
        ..
    } = audio;
    let entry = columns.entry(key).or_insert_with(|| AreaColumns {
        model: Rc::new(VecModel::default()),
        window: None,
    });
    let track = waveform_tracks
        .get_mut(view.video_index)
        .and_then(|tracks| tracks.get_mut(view.track_index));
    if let Some(track) = track {
        // 快路径：视图未变且窗口有效（未失效）时模型内容已是最新的。
        if entry.window != Some(view) {
            track.fill_columns(&entry.model, view);
            entry.window = Some(view);
        }
    } else {
        entry.window = None;
        entry.model.clear();
    }
    views.insert(key, view);
    Rc::clone(&entry.model)
}

/// 波形批次落库后刷新受影响 Area 的列模型（pump 每批调用）：
/// 只重折与新写入行区间 `[row_lo, row_hi)` 相交的列（含此前只聚合到部分行的
/// 边界列），窗口失效（基础模型重建过）的 Area 整窗经缓存重填。
pub(crate) fn ui_refresh_waveform_columns_for_batch(
    audio: &mut AudioUiState,
    video_index: usize,
    track_index: usize,
    row_lo: usize,
    row_hi: usize,
) {
    if row_hi <= row_lo {
        return;
    }
    let AudioUiState {
        waveform_tracks,
        views,
        columns,
        ..
    } = audio;
    let Some(track) = waveform_tracks
        .get_mut(video_index)
        .and_then(|tracks| tracks.get_mut(track_index))
    else {
        return;
    };
    // 失效已经在 `apply_batch` 里做过了：相交列此刻必然不在缓存里。
    // 这里只负责把可见窗口重绘出来，非相交列零折算、零通知。
    // 先摘成 owned 副本：循环里要改 `columns`，`views` 的借用必须提前还掉。
    let visible: Vec<(ScrAreaKey, AreaWaveformView)> = views
        .iter()
        .filter(|(_, view)| view.video_index == video_index && view.track_index == track_index)
        .map(|(key, view)| (*key, *view))
        .collect();
    for (key, view) in visible {
        let Some(entry) = columns.get_mut(&key) else {
            continue;
        };
        if entry.window != Some(view) {
            track.fill_columns(&entry.model, view);
            entry.window = Some(view);
        } else {
            track.refresh_columns(&entry.model, view, row_lo, row_hi);
        }
    }
}

/// 更换视频后作废该视频的全部 Area 列窗口。整条 `WaveformTrack` 已被新建的顶掉，
/// 折算缓存随之是空的；但 Area 的列模型还挂着旧媒体的列值，必须立即清空并把
/// 窗口标记为失效，下一次同步整窗重填。
pub(crate) fn ui_invalidate_waveform_columns_for_video(
    audio: &mut AudioUiState,
    video_index: usize,
) {
    let stale: Vec<ScrAreaKey> = audio
        .views
        .iter()
        .filter(|(_, view)| view.video_index == video_index)
        .map(|(key, _)| *key)
        .collect();
    for key in stale {
        if let Some(entry) = audio.columns.get_mut(&key) {
            entry.window = None;
            entry.model.clear();
        }
    }
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
/// 逐条做取消检查后写基础桶与可见列：批次只写入自己的行区间，可见列只刷新
/// 与该区间相交的列，平移/缩放产生的重叠列由缓存命中承担。`store` 每次按消息
/// 现借，保证替换媒体后写到最新的模型上。
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
        // 写耐久层与丢受影响的列合成一次调用。基础桶不绑 Slint，写它不发任何
        // 通知，所以这里可以借 store 的可变借用；替换媒体后整条 WaveformTrack
        // 已被新建的顶掉，按消息现借也就恒写到当前会话的轨道上。
        // 借用必须在这个块里还掉：下面紧接着还有 `store.borrow()`，
        // RefCell 重复借用是运行时 panic，编译期拦不住。
        let written = {
            let mut store = store.borrow_mut();
            store
                .ui_models
                .audio
                .waveform_tracks
                .get_mut(batch.video_index)
                .and_then(|tracks| tracks.get_mut(batch.track_index))
                .map(|track| track.apply_batch(batch.start, &batch.buckets))
        };
        let Some((row_lo, row_hi)) = written else {
            log::warn!(
                "丢弃越界波形结果: video-index={}, track-index={}",
                batch.video_index,
                batch.track_index
            );
            continue;
        };
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
        // 重算上报过且同视频同音轨的视图列（只碰与新行区间相交的列），并裁剪失效 Area。
        {
            let mut store = store.borrow_mut();
            ui_prune_waveform_views(&mut store);
            ui_refresh_waveform_columns_for_batch(
                &mut store.ui_models.audio,
                batch.video_index,
                batch.track_index,
                row_lo,
                row_hi,
            );
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
        let base = vec![
            bucket(-0.1, 0.1, 0.1),
            bucket(-0.2, 0.9, 0.5),
            bucket(-0.2, 0.2, 0.2),
            bucket(-0.3, 0.3, 0.4),
            bucket(-0.4, 0.4, 0.3),
            bucket(-0.1, 0.5, 0.5),
            bucket(-0.8, 0.8, 0.6),
            bucket(-0.1, 0.1, 0.2),
        ];
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
        let base = vec![bucket(-0.5, 0.5, 0.4); 4];
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
        let empty: Vec<SlintWaveformBucket> = Vec::new();
        let padded = ui_compute_waveform_columns(&empty, 2, 0, 3);
        assert_eq!(padded.len(), 3);
        assert_summary_eq(
            &summarize_vec(&padded),
            &[(0.0, 0.0, 0.0), (0.0, 0.0, 0.0), (0.0, 0.0, 0.0)],
        );
    }

    #[test]
    fn waveform_batch_writes_rows_and_preserves_gaps() {
        let mut values: Vec<SlintWaveformBucket> = Vec::new();
        let (lo, hi) = ui_write_base_rows(
            &mut values,
            2,
            &[bucket(-0.25, 0.25, 0.2), bucket(-0.5, 0.5, 0.4)],
        );
        assert_eq!((lo, hi), (0, 4), "首批从空耐久层开始，写入区间含间隙零桶");
        assert_summary_eq(
            &summarize_vec(&values),
            &[
                (0.0, 0.0, 0.0),
                (0.0, 0.0, 0.0),
                (-0.25, 0.25, 0.2),
                (-0.5, 0.5, 0.4),
            ],
        );

        let (lo, hi) = ui_write_base_rows(
            &mut values,
            1,
            &[
                bucket(-0.75, 0.75, 0.6),
                bucket(-0.8, 0.8, 0.7),
                bucket(-0.9, 0.9, 0.8),
            ],
        );
        assert_eq!((lo, hi), (1, 4), "覆盖已有行时写入区间从覆盖起点开始");
        assert_summary_eq(
            &summarize_vec(&values),
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
        audio
            .waveform_tracks
            .push(vec![WaveformTrack::with_base(vec![
                bucket(-0.5, 0.5, 0.4);
                base_rows
            ])]);
        (audio, test_key(1))
    }

    /// 第 `row` 个基础桶编码行号（min = -row/10，max = row/10），方便断言某列
    /// 到底聚合了哪些行。测试经轨道唯一的写入口 `apply_batch` 把它们送进去。
    fn indexed_batch(start: usize, rows: usize) -> Vec<SlintWaveformBucket> {
        (start..start + rows)
            .map(|row| {
                let peak = row as f32 / 10.0;
                bucket(-peak, peak, peak / 2.0)
            })
            .collect()
    }

    fn indexed_audio(rows: usize) -> AudioUiState {
        let mut audio = AudioUiState::default();
        audio
            .waveform_tracks
            .push(vec![WaveformTrack::with_base(indexed_batch(0, rows))]);
        audio
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
        // 同一视图再次同步：模型标识不变（Slint 侧不重建），内容保持。
        let second = ui_sync_waveform_columns(&mut audio, key, view);
        assert!(Rc::ptr_eq(&first, &second));
        assert_eq!(second.row_count(), 4);
    }

    #[test]
    fn sync_same_view_twice_skips_refill() {
        let (mut audio, key) = sync_case(8);
        let view = AreaWaveformView {
            video_index: 0,
            track_index: 0,
            bucket_ms: 2,
            first: 0,
            count: 4,
        };
        let model = ui_sync_waveform_columns(&mut audio, key, view);
        // 手动污染一行模拟外部写入：同视图再同步必须走快路径不重填。
        model.set_row_data(0, bucket(-9.0, 9.0, 9.0));
        ui_sync_waveform_columns(&mut audio, key, view);
        assert_eq!(
            model.row_data(0),
            Some(bucket(-9.0, 9.0, 9.0)),
            "视图未变化时同步不得整窗重填"
        );
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
            &audio.columns.get(&key).expect("列模型应保留").model
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

    #[test]
    fn batch_refresh_touches_only_intersecting_columns() {
        let mut audio = indexed_audio(4);
        let key = test_key(1);
        let view = AreaWaveformView {
            video_index: 0,
            track_index: 0,
            bucket_ms: 2,
            first: 0,
            count: 4,
        };
        let model = ui_sync_waveform_columns(&mut audio, key, view);
        // 首窗：基础 4 行 → 列 0 聚合行 0-1，列 1 聚合行 2-3，列 2-3 为零桶。
        assert_summary_eq(
            &summarize(&model),
            &[
                (-0.1, 0.1, rms(&[0.0, 0.05])),
                (-0.3, 0.3, rms(&[0.1, 0.15])),
                (0.0, 0.0, 0.0),
                (0.0, 0.0, 0.0),
            ],
        );

        // 新批次追加行 4-7：只应刷新列 2（行 4-5）与列 3（行 6-7）。
        let (lo, hi) = audio.waveform_tracks[0][0].apply_batch(4, &indexed_batch(4, 4));
        ui_refresh_waveform_columns_for_batch(&mut audio, 0, 0, lo, hi);
        assert_summary_eq(
            &summarize(&model),
            &[
                (-0.1, 0.1, rms(&[0.0, 0.05])),
                (-0.3, 0.3, rms(&[0.1, 0.15])),
                (-0.5, 0.5, rms(&[0.2, 0.25])),
                (-0.7, 0.7, rms(&[0.3, 0.35])),
            ],
        );
    }

    #[test]
    fn batch_refresh_refolds_partial_boundary_column() {
        let mut audio = indexed_audio(5);
        let key = test_key(1);
        let view = AreaWaveformView {
            video_index: 0,
            track_index: 0,
            bucket_ms: 2,
            first: 0,
            count: 3,
        };
        let model = ui_sync_waveform_columns(&mut audio, key, view);
        // 列 2 只聚合到 1 行（行 4），列内 rms 缺行 5。
        assert_summary_eq(
            &summarize(&model).last().cloned().map(|v| vec![v]).unwrap(),
            &[(-0.4, 0.4, 0.2)],
        );

        // 新批次写行 5-6：列 2（覆盖行 4-5）是部分列，被写时失效丢掉，
        // 刷新时必然重折补上行 5。
        let (lo, hi) = audio.waveform_tracks[0][0].apply_batch(5, &indexed_batch(5, 2));
        ui_refresh_waveform_columns_for_batch(&mut audio, 0, 0, lo, hi);
        assert_summary_eq(
            &summarize(&model).last().cloned().map(|v| vec![v]).unwrap(),
            &[(-0.5, 0.5, rms(&[0.2, 0.25]))],
        );
    }

    #[test]
    fn batch_refresh_skips_views_of_other_tracks() {
        let mut audio = indexed_audio(4);
        let waveform_key = test_key(1);
        let other_key = test_key(2);
        let view = AreaWaveformView {
            video_index: 0,
            track_index: 0,
            bucket_ms: 2,
            first: 0,
            count: 4,
        };
        let model = ui_sync_waveform_columns(&mut audio, waveform_key, view);
        // 另一条音轨（下标 1）没有基础桶：注册视图后不得被本轨批次触碰。
        let absent = ui_sync_waveform_columns(
            &mut audio,
            other_key,
            AreaWaveformView {
                track_index: 1,
                ..view
            },
        );
        assert_eq!(absent.row_count(), 0);

        let (lo, hi) = audio.waveform_tracks[0][0].apply_batch(4, &indexed_batch(4, 4));
        ui_refresh_waveform_columns_for_batch(&mut audio, 0, 0, lo, hi);
        assert_eq!(absent.row_count(), 0, "无基础桶的视图保持空模型");
        assert_eq!(summarize(&model).len(), 4, "本轨视图保持列数，仅行值更新");
    }

    #[test]
    fn invalidate_for_video_drops_stale_cache_and_windows() {
        let (mut audio, key) = sync_case(8);
        let view = AreaWaveformView {
            video_index: 0,
            track_index: 0,
            bucket_ms: 2,
            first: 0,
            count: 4,
        };
        let model = ui_sync_waveform_columns(&mut audio, key, view);
        assert_eq!(model.row_count(), 4);

        // 模拟更换视频：整条轨道换成新数据（值翻倍），缓存随新轨道归零。
        audio.waveform_tracks[0][0] = WaveformTrack::with_base(vec![bucket(-1.0, 1.0, 0.8); 8]);
        ui_invalidate_waveform_columns_for_video(&mut audio, 0);
        assert_eq!(
            model.row_count(),
            0,
            "作废后列模型立即清空，不再显示旧媒体波形"
        );

        let model = ui_sync_waveform_columns(&mut audio, key, view);
        assert_summary_eq(&summarize(&model), &[(-1.0, 1.0, 0.8); 4]);
    }

    #[test]
    fn column_cache_hits_survive_base_growth_outside_coverage() {
        let mut audio = indexed_audio(4);
        let key = test_key(1);
        let home = AreaWaveformView {
            video_index: 0,
            track_index: 0,
            bucket_ms: 2,
            first: 0,
            count: 2,
        };
        let model = ui_sync_waveform_columns(&mut audio, key, home);
        let expected = summarize(&model);

        // 基础桶追加行 4-11：列 0-1 覆盖的行 0-3 不与写入区间相交，
        // 因而不会被 `drop_folded` 碰到，平移走远再回来必须仍命中旧值。
        let (lo, hi) = audio.waveform_tracks[0][0].apply_batch(4, &vec![bucket(-9.0, 9.0, 9.0); 8]);
        assert_eq!((lo, hi), (4, 12));
        let away = AreaWaveformView { first: 2, ..home };
        let model = ui_sync_waveform_columns(&mut audio, key, away);
        assert_summary_eq(&summarize(&model), &[(-9.0, 9.0, 9.0), (-9.0, 9.0, 9.0)]);
        let model = ui_sync_waveform_columns(&mut audio, key, home);
        assert_summary_eq(&summarize(&model), &expected);
    }

    #[test]
    fn batch_overwrite_same_len_refolds_visible_column() {
        let mut audio = indexed_audio(4);
        let key = test_key(1);
        let view = AreaWaveformView {
            video_index: 0,
            track_index: 0,
            bucket_ms: 2,
            first: 0,
            count: 2,
        };
        let model = ui_sync_waveform_columns(&mut audio, key, view);
        assert_summary_eq(
            &summarize(&model),
            &[
                (-0.1, 0.1, rms(&[0.0, 0.05])),
                (-0.3, 0.3, rms(&[0.1, 0.15])),
            ],
        );

        // 同长度覆盖行 1（半桶补样本 / PTS 回访）：长度不变但值变了，
        // 与之相交的列 0 被写时失效丢掉，刷新时必然重折。
        let (lo, hi) = audio.waveform_tracks[0][0].apply_batch(1, &[bucket(-0.9, 0.9, 0.8)]);
        assert_eq!((lo, hi), (1, 2));
        ui_refresh_waveform_columns_for_batch(&mut audio, 0, 0, lo, hi);
        assert_summary_eq(
            &summarize(&model),
            &[
                (-0.9, 0.9, rms(&[0.0, 0.8])),
                (-0.3, 0.3, rms(&[0.1, 0.15])),
            ],
        );
    }

    #[test]
    fn overwrite_outside_views_heals_on_pan_back() {
        let mut audio = indexed_audio(8);
        let key = test_key(1);
        let home = AreaWaveformView {
            video_index: 0,
            track_index: 0,
            bucket_ms: 2,
            first: 0,
            count: 2,
        };
        let away = AreaWaveformView {
            first: 3,
            count: 1,
            ..home
        };
        // 先去过远端：列 3 按旧值折算并入缓存。
        let model = ui_sync_waveform_columns(&mut audio, key, away);
        assert_summary_eq(&summarize(&model), &[(-0.7, 0.7, rms(&[0.3, 0.35]))]);
        // 回到 home 窗口（列 0-1，行 0-3）。
        let model = ui_sync_waveform_columns(&mut audio, key, home);
        let expected_home = summarize(&model);

        // 改写远端行 6：当前 home 窗口与之不相交，本次刷新不得碰 home 列，
        // 但列 3（覆盖行 6-7）的缓存条目必须被丢掉。
        let (lo, hi) = audio.waveform_tracks[0][0].apply_batch(6, &[bucket(-0.9, 0.9, 0.8)]);
        assert_eq!((lo, hi), (6, 7));
        ui_refresh_waveform_columns_for_batch(&mut audio, 0, 0, lo, hi);
        assert_summary_eq(&summarize(&model), &expected_home);

        // 再平移回列 3（行 6-7）：不能返回改写前的旧折算值。
        let model = ui_sync_waveform_columns(&mut audio, key, away);
        assert_summary_eq(&summarize(&model), &[(-0.9, 0.9, rms(&[0.8, 0.35]))]);
    }

    /// 同一批行被两个缩放档各自折算过：改写其中一行时，两个档的相交列都必须重折。
    #[test]
    fn write_heals_columns_in_every_zoom_group() {
        let mut audio = indexed_audio(8);
        let fine_key = test_key(1);
        let coarse_key = test_key(2);
        let fine = AreaWaveformView {
            video_index: 0,
            track_index: 0,
            bucket_ms: 2,
            first: 1,
            count: 2,
        };
        let coarse = AreaWaveformView {
            bucket_ms: 4,
            first: 0,
            ..fine
        };
        let fine_model = ui_sync_waveform_columns(&mut audio, fine_key, fine);
        let coarse_model = ui_sync_waveform_columns(&mut audio, coarse_key, coarse);
        let before_fine = summarize(&fine_model);
        let before_coarse = summarize(&coarse_model);

        // 覆盖行 3：2ms 档的列 1 覆盖行 2-3，4ms 档的列 0 覆盖行 0-3。
        let (lo, hi) = audio.waveform_tracks[0][0].apply_batch(3, &[bucket(-0.9, 0.9, 0.8)]);
        assert_eq!((lo, hi), (3, 4));
        ui_refresh_waveform_columns_for_batch(&mut audio, 0, 0, lo, hi);

        assert_ne!(
            summarize(&fine_model),
            before_fine,
            "2ms 档的相交列必须重折"
        );
        assert_ne!(
            summarize(&coarse_model),
            before_coarse,
            "4ms 档的相交列必须重折"
        );
    }
}
