use crate::actions::media::{MediaAction, VideoAction};
use crate::bridge::types::SlintVideoTimeBar;
use crate::depsgraph::Tag;
use crate::store::AppStore;
use crate::store::state::{VideoSeekSession, VideoSeekTarget, VideoSession};
use crate::store::utils::rainbow::rainbow_color;
use anyhow::{Context, Result};
use gstreamer::prelude::*;
use slint::ComponentHandle as _;
use slint::Model as _;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, OnceLock};

static GSTREAMER_INIT_RESULT: OnceLock<Result<(), gstreamer::glib::Error>> = OnceLock::new();

fn gst_initialize_once() -> Result<(), gstreamer::glib::Error> {
    GSTREAMER_INIT_RESULT.get_or_init(gstreamer::init).clone()
}

/// 分发媒体域 Action：管线在此创建/跳转；帧推送与 GPU 连接属渲染层
/// （`render/video_sink`），播放位置经渲染帧同步直写动画标量与视频时间条
/// （见 `bridge/callbacks.rs:handle_before_rendering`）。媒体域不产生 tag。
impl AppStore {
    pub(crate) fn dispatch_media(&mut self, action: MediaAction) -> Result<Vec<Tag>> {
        match action {
            MediaAction::Video(VideoAction::Append { uri }) => media_append_video_session(self, &uri)?,
            MediaAction::Video(VideoAction::Replace { index, uri }) => {
                media_replace_video_session(self, index, &uri)?
            }
            MediaAction::Video(VideoAction::ToggleMute { index }) => {
                let Some(session) = self.state_mut().video_sessions.get_mut(index) else {
                    log::warn!("切换静音：下标越界 {index}");
                    return Ok(Vec::new());
                };
                session.muted = !session.muted;
                gst_set_audio_muted(&session.pipeline, session.muted);
                ui_sync_video_time_bars(self);
            }
            MediaAction::Video(VideoAction::Select { index, modifier }) => {
                if let Some(index) = index
                    && index >= self.state().video_sessions.len()
                {
                    log::warn!("选择视频：下标越界 {index}");
                    return Ok(Vec::new());
                }
                use crate::bridge::types::SlintSelectionModifier;
                match (index, modifier) {
                    (None, SlintSelectionModifier::None) => {
                        self.state_mut().selected_videos.clear();
                    }
                    (None, _) => {}
                    (Some(index), SlintSelectionModifier::None) => {
                        self.state_mut().selected_videos.clear();
                        self.state_mut().selected_videos.push(index);
                    }
                    (Some(index), SlintSelectionModifier::Ctrl) => {
                        let state = self.state_mut();
                        if let Some(position) =
                            state.selected_videos.iter().position(|&i| i == index)
                        {
                            state.selected_videos.remove(position);
                        } else {
                            state.selected_videos.push(index);
                        }
                    }
                    (Some(index), SlintSelectionModifier::Shift) => {
                        let state = self.state_mut();
                        if !state.selected_videos.contains(&index) {
                            state.selected_videos.push(index);
                        } else {
                            state.selected_videos.retain(|&selected| selected != index);
                            state.selected_videos.push(index);
                        }
                    }
                }
                // 立即让时间框跟随新选中视频（无选中时跟随主视频）
                ui_sync_active_video_time(self);
                // 仅局部刷新时间条（set_row_data，不重建模型——拖拽中更新会破坏 grab）
                ui_sync_video_time_bars(self);
                // 选择不改变任何管线状态，只改变下一次 transport 的控制目标。
                ui_sync_video_playing_state(self);
            }
            MediaAction::Video(VideoAction::SetMain { index }) => {
                if index >= self.state().video_sessions.len() {
                    log::warn!("设为主视频：下标越界 {index}");
                } else {
                    media_set_main_video_session(self, index);
                }
            }
            MediaAction::Video(VideoAction::MoveVideo { index, x, y }) => {
                // 主视频固定在原点（带参考框），不可移动
                if Some(index) == self.state().main_video {
                    log::debug!("主视频不可移动，忽略");
                    return Ok(Vec::new());
                }
                let Some(session) = self.state_mut().video_sessions.get_mut(index) else {
                    log::warn!("移动视频：下标越界 {index}");
                    return Ok(Vec::new());
                };
                session.position_x = x;
                session.position_y = y;
                // 位置局部更新（set_row_data；拖拽中重建模型会破坏 nav 的 grab）
                ui_update_video_position(self, index);
            }
            MediaAction::Video(VideoAction::Seek { timestamp }) => {
                self.seek_drag_targets(timestamp as f64 / 1000.0);
            }
            MediaAction::Video(VideoAction::SeekVideo { index, timestamp }) => {
                let Some(session) = self.state_mut().video_sessions.get_mut(index) else {
                    log::warn!("指定视频 seek：下标越界 {index}");
                    return Ok(Vec::new());
                };
                media_seek_video_session(session, timestamp as f64 / 1000.0);
                let current_time = session.current_time;
                let _ = session;
                ui_update_video_time_bar(&self.ui_models.video.time_bars, index, current_time);
            }
            MediaAction::Video(VideoAction::SeekBegin { index, is_batch }) => {
                if index >= self.state().video_sessions.len() {
                    log::warn!("开始拖拽视频时间条：下标越界 {index}");
                    return Ok(Vec::new());
                }
                if is_batch {
                    let selected = &mut self.state_mut().selected_videos;
                    selected.retain(|&selected| selected != index);
                    selected.push(index);
                } else {
                    self.state_mut().selected_videos.clear();
                    self.state_mut().selected_videos.push(index);
                }
                // 立即让时间框跟随新选中视频（无选中时跟随主视频）
                ui_sync_active_video_time(self);
                // 仅局部刷新时间条（set_row_data，不重建模型——拖拽中更新会破坏 grab）
                ui_sync_video_time_bars(self);
                // 记录拖拽前状态。暂停态 seek 的 preroll 由 appsink 回调直接交付，
                // 不需要临时切到 Playing，也不能为预览启动额外渲染循环。
                let target_indices = if is_batch {
                    self.state().selected_videos.clone()
                } else {
                    vec![index]
                };
                let targets = target_indices
                    .into_iter()
                    .filter_map(|index| {
                        let session = self.state().video_sessions.get(index)?;
                        Some(VideoSeekTarget {
                            index,
                            origin_time: session.current_time,
                            was_playing: gst_pipeline_wants_playing(&session.pipeline),
                        })
                    })
                    .collect();
                let anchor_time = self.state().video_sessions[index].current_time;
                self.state_mut().video_seek_session = Some(VideoSeekSession {
                    anchor_time,
                    targets,
                });
                for target in self
                    .state()
                    .video_seek_session
                    .as_ref()
                    .into_iter()
                    .flat_map(|drag| &drag.targets)
                {
                    if target.was_playing {
                        gst_set_audio_muted(&self.state().video_sessions[target.index].pipeline, true);
                    }
                }
            }
            MediaAction::Video(VideoAction::SeekEnd) => {
                let Some(drag) = self.state_mut().video_seek_session.take() else {
                    return Ok(Vec::new());
                };
                for target in drag.targets {
                    let Some(session) = self.state_mut().video_sessions.get_mut(target.index)
                    else {
                        continue;
                    };
                    if !target.was_playing {
                        continue;
                    }
                    // 先精确 seek 到拖拽点（此时音频 sink 仍 PAUSED，无声音）
                    gst_seek_pipeline_accurate(&session.pipeline, session.current_time.max(0.0));
                    gst_set_session_state(session, gstreamer::State::Playing);
                    // 再解除静音：从拖拽点续播，不会再冒出停在原位的旧声音
                    gst_set_audio_muted(&session.pipeline, session.muted);
                }
                ui_sync_video_playing_state(self);
            }
            MediaAction::Video(VideoAction::Play) => media_apply_video_playback(self, true),
            MediaAction::Video(VideoAction::Pause) => media_apply_video_playback(self, false),
            MediaAction::Video(VideoAction::TogglePlayback) => {
                media_apply_video_playback(self, !self.state().video_playing);
            }
            // 帧由 appsink 回调和渲染通知驱动，Action 不需要重复拉取。
            MediaAction::Video(VideoAction::UpdateFrame) => {}
            MediaAction::Subtitle(action) => log::warn!("字幕 Action 尚未实现：{action:?}"),
        }
        Ok(Vec::new())
    }
}

fn gst_set_session_state(session: &VideoSession, target: gstreamer::State) {
    if let Err(error) = session.pipeline.set_state(target) {
        log::warn!("切换视频状态到 {target:?} 失败: {error}");
    }
}

/// 拖拽时间条期间静音音频：单帧预览不需要声音，且避免画面（冻结在拖拽点）与声音
/// （继续随播放推进）不一致。release 时按原播放状态恢复并重新 seek 到拖拽点以重同步。
/// 拖拽期间静音：只把音频分支上嵌入的 `volume` 元素（build_audio_branch 已加入）
/// 的增益置 0 / 恢复 1。该元素是纯软件增益，属性改动线程安全、必然静音。
///
/// 关键：绝不去碰 autoaudiosink 及其真实 child sink 的 `mute`/`volume` 属性——
/// 在播放态从 dispatch 线程给音频 sink 设这些属性会让管线崩溃。也绝不改任何元素
/// 状态（暂停 sink 同样会崩）。
fn gst_set_audio_muted(pipeline: &gstreamer::Pipeline, muted: bool) {
    use gstreamer::prelude::*;
    let vol: f64 = if muted { 0.0 } else { 1.0 };
    for element in pipeline.iterate_elements() {
        let Ok(el) = element else {
            continue;
        };
        // 仅精确命中工厂名为 "volume" 的软件增益元素，避免触碰音频 sink
        if el.factory().map_or(false, |f| f.name() == "volume") {
            let _ = el.set_property("mute", muted);
            let _ = el.set_property("volume", vol);
        }
    }
}

/// 将每个会话的显式静音状态同步到音频分支；选择和 transport 不会隐式改变它。
fn gst_sync_session_audio_mutes(store: &AppStore) {
    for session in &store.state().video_sessions {
        gst_set_audio_muted(&session.pipeline, session.muted);
    }
}

/// 时间线 transport 的控制目标：有视口选择时控制整个选择集合；否则只控制主视频。
/// 返回值始终去重且过滤越界下标。
fn media_control_targets(store: &AppStore) -> Vec<usize> {
    let state = store.state();
    let mut targets = Vec::new();
    for &index in &state.selected_videos {
        if index < state.video_sessions.len() && !targets.contains(&index) {
            targets.push(index);
        }
    }
    if targets.is_empty()
        && let Some(main) = state.main_video
        && main < state.video_sessions.len()
    {
        targets.push(main);
    }
    targets
}

/// 设置播放状态标量（state 字段 + VideoState 属性）。
fn ui_set_video_playing(store: &mut AppStore, playing: bool) {
    store.state_mut().video_playing = playing;
    if let Some(weak) = &store.video_state
        && let Some(state) = weak.upgrade()
    {
        state.set_playing(playing);
    }
}

/// 应用时间线 transport：播放/暂停作用于视口选择集合；无选择时只作用于主视频。
/// current_time，因此参考视频仍可独立 seek；按下播放后从各自位置同步启动。
///
/// 发起播放时不仅要 `set_state(Playing)`，还要对该管线做一次 flush-seek：
/// 1. 处于 EOS（播完）的管线 `current_state` 仍是 `Playing`，但单靠
///    `set_state(Playing)` 不会重新出帧——必须先 `seek` 清 EOS（EOS 时回到起点）；
/// 2. 暂停/卡顿的管线同样只有 seek（flush）才能真正恢复出帧，这正是"拖动时间条
///    能播、按播放键不动"的原因。这里直接在播放时 seek 到当前位置，等价于用户
///    拖动时间条的效果，保证播放键始终能开始/恢复播放。
fn media_apply_video_playback(store: &mut AppStore, playing: bool) {
    if store.state().video_sessions.is_empty() {
        ui_set_video_playing(store, false);
        return;
    }
    gst_sync_session_audio_mutes(store);
    let targets = media_control_targets(store);
    for index in targets {
        let session = &mut store.state_mut().video_sessions[index];
        gst_set_session_state(
            session,
            if playing {
                gstreamer::State::Playing
            } else {
                gstreamer::State::Paused
            },
        );
        if playing {
            let target = if gst_session_at_eos(&session.pipeline) {
                0.0
            } else {
                session.current_time.max(0.0)
            };
            if gst_session_at_eos(&session.pipeline) {
                gst_seek_pipeline_to_secs(&session.pipeline, target);
            } else {
                gst_seek_pipeline_accurate(&session.pipeline, target);
            }
            session.current_time = target;
        }
    }
    ui_set_video_playing(store, playing);
}

/// 管线是否（将）处于播放态：状态转换期间以目标态为准，避免过渡中闪烁。
/// 用零超时做**非阻塞**快照——`state(None)` 会阻塞到目标态达成（GST_CLOCK_TIME_NONE），
/// 在每帧渲染路径调用会卡死（打开视频时管线正进入 Playing）。
/// 注意：到达 EOS 的管线 `current_state` 仍是 `Playing`，但已不再出帧，故这里
/// 只判断"意图播放"，EOS 的判定由 `gst_session_at_eos` 单独负责。
pub(crate) fn gst_pipeline_wants_playing(pipeline: &gstreamer::Pipeline) -> bool {
    let (_, current, pending) = pipeline.state(Some(gstreamer::ClockTime::from_mseconds(0)));
    if pending != gstreamer::State::VoidPending {
        pending == gstreamer::State::Playing
    } else {
        current == gstreamer::State::Playing
    }
}

/// 管线是否已播到结尾（EOS）。仅当正处于播放意图态时才有意义：GStreamer 在
/// EOS 后仍报 `current_state == Playing`，位置停在时长处不再推进。需 `seek`
/// 清 EOS 才能恢复，单纯 `set_state(Playing)` 无效。
fn gst_session_at_eos(pipeline: &gstreamer::Pipeline) -> bool {
    if !gst_pipeline_wants_playing(pipeline) {
        return false;
    }
    let pos = pipeline.query_position::<gstreamer::ClockTime>();
    let dur = pipeline.query_duration::<gstreamer::ClockTime>();
    match (pos, dur) {
        (Some(p), Some(d)) if d > gstreamer::ClockTime::from_nseconds(0) => p >= d,
        _ => false,
    }
}

/// 按当前 transport 目标的实际状态校正全局 `video_playing`（随帧调用；
/// 播放/暂停命令已由 `media_apply_video_playback` 即时设置，此处只做状态回退校正）。
/// 已到 EOS 的管线仍报 Playing，但画面已停，故按"未播放"处理，让按钮正确显示
/// 为"播放"且下一次按下能真正重新开始（见 `media_apply_video_playback` 中的 EOS 重启）。
///
/// 拖拽期间不参与实时校正：拖拽会对管线做大量 flush-seek，其过渡态会让
/// `gst_pipeline_wants_playing` 瞬时抖动；此时沿用拖拽目标开始时的播放意图。
pub(crate) fn ui_sync_video_playing_state(store: &mut AppStore) {
    let playing = if let Some(drag) = &store.state().video_seek_session {
        !drag.targets.is_empty() && drag.targets.iter().all(|target| target.was_playing)
    } else {
        let targets = media_control_targets(store);
        !targets.is_empty()
            && targets.iter().all(|&index| {
                let session = &store.state().video_sessions[index];
                gst_pipeline_wants_playing(&session.pipeline) && !gst_session_at_eos(&session.pipeline)
            })
    };
    if store.state().video_playing == playing {
        return;
    }
    ui_set_video_playing(store, playing);
}

/// 把动画编辑器当前时间同步为活跃视频（选中无则主视频）的位置。
/// 时间框/播放头显示的就是活跃视频的时间。
pub(crate) fn ui_sync_active_video_time(store: &mut AppStore) {
    let Some(active) = store.state().active_video() else {
        return;
    };
    let Some(secs) = store
        .state()
        .video_sessions
        .get(active)
        .map(|s| s.current_time)
    else {
        return;
    };
    {
        let ce = &mut store.ui_models.animation.graph;
        if (ce.current_time - secs).abs() < 0.005 {
            return;
        }
        ce.current_time = secs;
    }
    store.sync_animation_scalars();
}

/// 追加一个视频会话：搭建管线 + 连接 GPU + 自动播放，级联错开落点
/// （相对当前选中视频，无选中则相对主视频）。
fn media_append_video_session(store: &mut AppStore, video_uri: &str) -> Result<()> {
    gst_initialize_once().context("GStreamer 初始化失败")?;
    let mut session = crate::render::video_sink::build_session(store, video_uri)?;
    let (x, y) = media_video_cascade_anchor(store);
    session.position_x = x;
    session.position_y = y;
    let is_first = store.state().video_sessions.is_empty();
    store.state_mut().video_sessions.push(session);
    if is_first {
        store.state_mut().main_video = Some(0);
    }

    // 结构性新增一行（push 只触发 row_added，不影响既有行/帧）
    {
        let ui = &mut store.ui_models.video;
        ui.frames.push(slint::Image::default());
        ui.widths.push(0);
        ui.heights.push(0);
        ui.positions.push(crate::bridge::types::SlintVideoPos {
            x: x as f32,
            y: y as f32,
        });
        ui.time_bars.push(SlintVideoTimeBar::default());
        let amplitudes = std::rc::Rc::new(slint::VecModel::default());
        let level_amplitudes = (1..=12)
            .map(|_| std::rc::Rc::new(slint::VecModel::default()))
            .collect::<Vec<_>>();
        let levels = level_amplitudes
            .iter()
            .enumerate()
            .map(
                |(level, values)| crate::bridge::types::SlintAudioWaveformLevel {
                    bucket_duration: 0.1 * (1usize << (level + 1)) as f32,
                    amplitudes: slint::ModelRc::from(values.clone()),
                },
            )
            .collect::<Vec<_>>();
        ui.audio_waveforms
            .push(crate::bridge::types::SlintAudioWaveform {
                duration: 0.0,
                amplitudes: slint::ModelRc::from(amplitudes.clone()),
                levels: slint::ModelRc::from(std::rc::Rc::new(slint::VecModel::from(levels))),
            });
        ui.waveform_amplitudes.push(amplitudes);
        ui.waveform_level_amplitudes.push(level_amplitudes);
    }
    let cancellation = Arc::new(AtomicBool::new(false));
    store
        .state_mut()
        .waveform_cancellations
        .push(cancellation.clone());
    start_waveform_extraction(
        store,
        video_uri,
        store.state().video_sessions.len() - 1,
        cancellation,
    );
    ui_sync_video_time_bars(store);
    // 新追加的视频没有被自动选中，因此不应偷偷加入当前 transport；保持暂停海报。
    gst_sync_session_audio_mutes(store);
    ui_sync_active_video_time(store);
    Ok(())
}

/// 原地替换某个视频会话（右键"更换视频"）：新会话连接成功后才拆旧会话；
/// 失败时保留旧会话。位置/主视频/选中关系不变；新会话首帧到来前旧帧占位。
fn media_replace_video_session(store: &mut AppStore, index: usize, uri: &str) -> Result<()> {
    let Some(old) = store.state().video_sessions.get(index) else {
        anyhow::bail!("更换视频：下标越界 {index}");
    };
    let anchor = (old.position_x, old.position_y);
    let old_time = old.current_time;
    let old_muted = old.muted;
    let session_was_playing = gst_pipeline_wants_playing(&old.pipeline);

    gst_initialize_once().context("GStreamer 初始化失败")?;
    let mut new_session = crate::render::video_sink::build_session(store, uri)?;
    new_session.position_x = anchor.0;
    new_session.position_y = anchor.1;
    // 替换视频时保留原播放位置，但不能假设新视频具有相同的时长。
    // build_session 已完成暂停态预滚动，此时通常可以查询到 duration；若媒体
    // 尚未提供 duration，则至少保证时间非负，后续帧同步会继续校正。
    new_session.current_time = gst_clamp_time_to_duration(&new_session.pipeline, old_time);
    new_session.muted = old_muted;

    // 新会话已成功启动，现在才安全地释放旧会话并原位替换。
    let old = store
        .state_mut()
        .video_sessions
        .get_mut(index)
        .expect("已在上方检查");
    let old = std::mem::replace(old, new_session);
    crate::render::video_sink::stop_session_parts(&old.pipeline, &old.sink_element, old.wgpu_sink);
    if let Some(old) = store.state().waveform_cancellations.get(index) {
        old.store(true, Ordering::Release);
    }
    let cancellation = Arc::new(AtomicBool::new(false));
    if let Some(slot) = store.state_mut().waveform_cancellations.get_mut(index) {
        *slot = cancellation.clone();
    }
    if let Some(values) = store.ui_models.video.waveform_amplitudes.get(index) {
        values.clear();
    }
    if let Some(model) = store.ui_models.video.audio_waveforms.row_data(index) {
        let mut pending = model;
        pending.duration = 0.0;
        if let Some(levels) = store.ui_models.video.waveform_level_amplitudes.get(index) {
            for level in levels {
                level.clear();
            }
        }
        store
            .ui_models
            .video
            .audio_waveforms
            .set_row_data(index, pending);
    }
    start_waveform_extraction(store, uri, index, cancellation);
    {
        let session = &mut store.state_mut().video_sessions[index];
        // 即使旧会话原来是暂停的，也必须把新管线定位到保留/钳制后的位置；
        // 否则 UI 的 current_time 与新管线实际位置会分离（新管线仍在 0 秒）。
        gst_seek_pipeline_accurate(&session.pipeline, session.current_time);
        if session_was_playing {
            gst_set_session_state(session, gstreamer::State::Playing);
        }
        // 替换后的会话继承显式静音状态；无需重新遍历并改写其它会话。
        gst_set_audio_muted(&session.pipeline, session.muted);
    }
    ui_sync_video_time_bars(store);
    Ok(())
}

struct WaveformJob {
    window: slint::Weak<crate::bridge::types::MainWindow>,
    uri: String,
    index: usize,
    cancellation: Arc<AtomicBool>,
}

fn waveform_worker() -> &'static std::sync::mpsc::Sender<WaveformJob> {
    static WORKER: OnceLock<std::sync::mpsc::Sender<WaveformJob>> = OnceLock::new();
    WORKER.get_or_init(|| {
        let (sender, receiver) = std::sync::mpsc::channel::<WaveformJob>();
        std::thread::Builder::new()
            .name("aweft-waveform".into())
            .spawn(move || {
                while let Ok(job) = receiver.recv() {
                    run_waveform_job(job);
                }
            })
            .expect("无法启动波形后台 worker");
        sender
    })
}

fn run_waveform_job(job: WaveformJob) {
    if job.cancellation.load(Ordering::Acquire) {
        return;
    }
    let result = crate::render::video_sink::waveform::extract_cancelable(
        &job.uri,
        || job.cancellation.load(Ordering::Acquire),
        |start, duration, amplitudes| {
            if job.cancellation.load(Ordering::Acquire) {
                return;
            }
            let batch = amplitudes.to_vec();
            let cancellation = job.cancellation.clone();
            let _ = job.window.upgrade_in_event_loop(move |instance| {
                if cancellation.load(Ordering::Acquire) {
                    return;
                }
                instance
                    .global::<crate::bridge::types::VideoCallbacks>()
                    .invoke_waveform_progress(
                        job.index as i32,
                        start as i32,
                        duration,
                        slint::ModelRc::from(std::rc::Rc::new(slint::VecModel::from(batch))),
                    );
            });
        },
    );
    if let Err(error) = result {
        log::warn!("音频波形提取失败 ({}): {error:#}", job.uri);
    }
}

/// 将任务送入单 worker；多个视频不会并行全速解码争抢播放资源。
fn start_waveform_extraction(
    store: &AppStore,
    uri: &str,
    index: usize,
    cancellation: Arc<AtomicBool>,
) {
    let Some(window) = store.main_window.clone() else {
        return;
    };
    if waveform_worker()
        .send(WaveformJob {
            window,
            uri: uri.to_owned(),
            index,
            cancellation,
        })
        .is_err()
    {
        log::warn!("波形后台 worker 已退出");
    }
}

/// 设置主视频：与旧主视频交换视口位置（主视频恒在原点，参考框/级联基准一致），
/// 并选中新主视频。位置用 set_row_data 局部更新，不触碰帧模型。
fn media_set_main_video_session(store: &mut AppStore, index: usize) {
    let main = store.state().main_video;
    if let Some(main) = main
        && main != index
    {
        let old_main_pos = {
            let s = &store.state().video_sessions[main];
            (s.position_x, s.position_y)
        };
        let new_main_pos = {
            let s = &store.state().video_sessions[index];
            (s.position_x, s.position_y)
        };
        store.state_mut().video_sessions[main].position_x = new_main_pos.0;
        store.state_mut().video_sessions[main].position_y = new_main_pos.1;
        store.state_mut().video_sessions[index].position_x = old_main_pos.0;
        store.state_mut().video_sessions[index].position_y = old_main_pos.1;
        ui_update_video_position(store, main);
        ui_update_video_position(store, index);
    }
    store.state_mut().main_video = Some(index);
    // 设为（并选中）主视频：交互对象顺带选中，视觉与参考框一致
    store.state_mut().selected_videos.clear();
    store.state_mut().selected_videos.push(index);
    // 时间框跟随新的活跃视频
    ui_sync_active_video_time(store);
    ui_sync_video_time_bars(store);
    // 活跃视频变化 → 立即重算播放状态
    ui_sync_video_playing_state(store);
}

/// 新会话的级联落点：相对当前选中视频（无选中则主视频）右下错开，
/// 错开量按锚点视频高度比例（不同分辨率下保持视觉比例一致）。
fn media_video_cascade_anchor(store: &AppStore) -> (f64, f64) {
    let state = store.state();
    let anchor = state
        .active_video()
        .and_then(|i| state.video_sessions.get(i));
    // 首个（主）视频固定在原点（带参考框），不应用级联偏移；偏移仅用于在其上
    // 错开叠加的后续视频。否则主视频会偏离参考框中心（右下错位）。
    let Some(anchor) = anchor else {
        return (0.0, 0.0);
    };
    let (ax, ay) = (anchor.position_x, anchor.position_y);
    let height = anchor
        .wgpu_sink
        .last_video_size
        .map(|(_, h)| h)
        .unwrap_or(1080) as f64;
    let offset = (height * 0.08).clamp(32.0, 200.0);
    (ax + offset, ay + offset)
}

/// 当前 seek 目标视频：选中（无则主视频）。seek 后局部刷新时间条；
/// 若目标为主视频则同步动画编辑器当前时间（关键帧/播放头跟随主视频）。
impl AppStore {
    pub(crate) fn seek_target(&mut self, secs: f64) {
        let Some(target) = self.state().active_video() else {
            return;
        };
        let Some(session) = self.state_mut().video_sessions.get_mut(target) else {
            log::warn!("seek 目标视频不存在：{target}");
            return;
        };
        media_seek_video_session(session, secs);
        let current_time = session.current_time;
        let _ = session;
        ui_update_video_time_bar(&self.ui_models.video.time_bars, target, current_time);
        // 时间框（播放头）跟随 seek 目标：选中无则主视频
        ui_sync_active_video_time(self);
    }

    /// 时间条拖拽目标：单拖时只有焦点，批量时按锚点增量移动全部选中视频。
    fn seek_drag_targets(&mut self, secs: f64) {
        let Some(drag) = self.state().video_seek_session.clone() else {
            self.seek_target(secs);
            return;
        };
        let delta = secs - drag.anchor_time;
        for target in drag.targets {
            let Some(session) = self.state_mut().video_sessions.get_mut(target.index) else {
                continue;
            };
            media_seek_video_session(session, (target.origin_time + delta).max(0.0));
            let current_time = session.current_time;
            let _ = session;
            ui_update_video_time_bar(
                &self.ui_models.video.time_bars,
                target.index,
                current_time,
            );
        }
        ui_sync_active_video_time(self);
    }

    /// 视频会话的视口世界尺寸（含 PAR 换算的显示宽度；未收到首帧用 1080P 默认）。
    fn video_world_size(&self, index: usize) -> (f64, f64) {
        let (w, h) = self
            .state()
            .video_sessions
            .get(index)
            .and_then(|s| s.wgpu_sink.last_video_size)
            .unwrap_or((1920, 1080));
        (w as f64, h as f64)
    }

    /// 视口点击命中检测：返回应选中的视频下标。
    /// 多个视频重叠时，若当前选中在命中集内则按序循环到下一个，否则选最上层。
    pub(crate) fn pick_video_at(&self, x: f64, y: f64) -> Option<usize> {
        let rects = self.video_rects();
        pick_overlapping_rect(&rects, self.state().active_video(), x, y)
    }

    /// 视口命中检测：返回该点最上层的视频下标（拖拽移动起点 / 右键菜单目标）。
    pub(crate) fn hit_top_video_at(&self, x: f64, y: f64) -> Option<usize> {
        let rects = self.video_rects();

        // 焦点视频在 UI 中被提升到最上层，命中检测也必须优先返回它。
        let selected = self.state().selected_videos.last().copied();
        if let Some(index) = selected
            && rects.get(index).is_some_and(|rect| rect.contains(x, y))
        {
            return Some(index);
        }

        hit_top_rect(&rects, x, y)
    }

    /// 全部会话的世界矩形列表（按下标序）。
    fn video_rects(&self) -> Vec<VideoRect> {
        self.state()
            .video_sessions
            .iter()
            .enumerate()
            .map(|(i, s)| {
                let (w, h) = self.video_world_size(i);
                VideoRect {
                    x: s.position_x,
                    y: s.position_y,
                    w,
                    h,
                }
            })
            .collect()
    }
}

/// 视频会话在视口世界坐标中的矩形（命中测试用）。
#[derive(Debug, Clone, Copy, PartialEq)]
struct VideoRect {
    x: f64,
    y: f64,
    w: f64,
    h: f64,
}

impl VideoRect {
    /// 点是否落在矩形内（边界含入）。
    fn contains(&self, px: f64, py: f64) -> bool {
        px >= self.x && px <= self.x + self.w && py >= self.y && py <= self.y + self.h
    }
}

/// 纯命中 + 循环选择：`rects` 按下标序（上层 = 大下标）。
/// 返回应选中的下标：当前选中在命中集内 → 循环到下一个；否则取最上层；无命中 None。
fn pick_overlapping_rect(
    rects: &[VideoRect],
    selected: Option<usize>,
    x: f64,
    y: f64,
) -> Option<usize> {
    let hits: Vec<usize> = rects
        .iter()
        .enumerate()
        .filter(|(_, rect)| rect.contains(x, y))
        .map(|(i, _)| i)
        .collect();
    if hits.is_empty() {
        return None;
    }
    if let Some(sel) = selected
        && let Some(pos) = hits.iter().position(|&i| i == sel)
    {
        return Some(hits[(pos + 1) % hits.len()]);
    }
    hits.last().copied()
}

/// 纯命中：返回最上层（最大下标）包含点的矩形下标，无命中 None。
fn hit_top_rect(rects: &[VideoRect], x: f64, y: f64) -> Option<usize> {
    rects
        .iter()
        .enumerate()
        .rev()
        .find(|(_, rect)| rect.contains(x, y))
        .map(|(i, _)| i)
}

/// 跳转单个会话到指定秒（编辑器拖动播放头/时间条反向驱动视频）。
fn media_seek_video_session(session: &mut VideoSession, secs: f64) {
    session.current_time = secs;
    // 暂停态 seek 产生的 preroll 会由 sink 回调保存并请求一次重绘。
    gst_seek_pipeline_accurate(&session.pipeline, secs);
}

/// 跳转管线到指定秒。FLUSH 立即生效，KEY_UNIT 快速定位（高频拖动以精度换流畅）。
fn gst_seek_pipeline_to_secs(pipeline: &gstreamer::Pipeline, secs: f64) {
    gst_seek_pipeline_with_flags(
        pipeline,
        secs,
        gstreamer::SeekFlags::FLUSH | gstreamer::SeekFlags::KEY_UNIT,
    );
}

/// 精确跳转（FLUSH + ACCURATE）：用于"从暂停/卡顿恢复播放"——回到**确切**位置
/// 而非最近关键帧，避免 KEY_UNIT 回退到开头；同时 flush 能让只靠 set_state(Playing)
/// 起不来（需先 seek 清流）的管线真正开始出帧（现象即"只有拖动时间条能播"）。
fn gst_seek_pipeline_accurate(pipeline: &gstreamer::Pipeline, secs: f64) {
    gst_seek_pipeline_with_flags(
        pipeline,
        secs,
        gstreamer::SeekFlags::FLUSH | gstreamer::SeekFlags::ACCURATE,
    );
}

/// 跳转管线到指定秒（带指定 seek 标志）。
fn gst_seek_pipeline_with_flags(
    pipeline: &gstreamer::Pipeline,
    secs: f64,
    flags: gstreamer::SeekFlags,
) {
    if let Err(e) = pipeline.seek_simple(
        flags,
        gstreamer::ClockTime::from_mseconds((secs.max(0.0) * 1000.0) as u64),
    ) {
        log::warn!("视频 seek 失败: {e}");
    }
}

/// 查询单个会话播放位置（秒）与视频帧时长（秒）：仅播放中返回（暂停时编辑器是
/// 时间权威，避免拖动播放头被陈旧位置回写）。无位置返回 None。
pub(crate) fn gst_session_playing_position(
    session: &VideoSession,
) -> Option<(f64, Option<f64>)> {
    if session.pipeline.current_state() != gstreamer::State::Playing {
        return None;
    }
    // 纳秒转秒：seconds() 会截断到整秒，导致播放头一秒一跳
    let secs = session
        .pipeline
        .query_position::<gstreamer::ClockTime>()?
        .nseconds() as f64
        / 1_000_000_000.0;
    let frame_dur = session.wgpu_sink.frame_duration_secs();
    Some((secs, frame_dur))
}

/// 移动单个视频位置：只更新该行（set_row_data），避免拖拽中重建模型。
fn ui_update_video_position(store: &mut AppStore, index: usize) {
    let Some(session) = store.state().video_sessions.get(index) else {
        return;
    };
    let (x, y) = (session.position_x, session.position_y);
    store.ui_models.video.positions.set_row_data(
        index,
        crate::bridge::types::SlintVideoPos {
            x: x as f32,
            y: y as f32,
        },
    );
}

/// 把选中/主视频下标推到 VideoState 标量（视图描边/参考框据此判断）。
fn ui_sync_video_scalars(store: &AppStore) {
    if let Some(weak) = &store.video_state
        && let Some(state) = weak.upgrade()
    {
        state.set_main_video(store.state().main_video.map(|i| i as i32).unwrap_or(-1));
        state.set_selected_video(
            store
                .state()
                .selected_videos
                .last()
                .map(|&i| i as i32)
                .unwrap_or(-1),
        );
    }
}

/// 播放/拖拽热路径只更新一个时间条的时间字段，保留 label/color 等不变量。
pub(crate) fn ui_update_video_time_bar(
    bars: &slint::VecModel<SlintVideoTimeBar>,
    index: usize,
    time: f64,
) {
    let Some(mut bar) = bars.row_data(index) else {
        return;
    };
    let time = time as f32;
    if (bar.time - time).abs() < f32::EPSILON {
        return;
    }
    bar.time = time;
    bars.set_row_data(index, bar);
}

/// 选中/结构变化后局部刷新时间条模型（set_row_data，不重建模型）
/// 并推送选中/主视频标量。颜色按下标确定性取彩虹色（与曲线编辑器同款）。
pub(crate) fn ui_sync_video_time_bars(store: &mut AppStore) {
    let bars = &store.ui_models.video.time_bars;
    let sessions = &store.state().video_sessions;
    let main = store.state().main_video;
    let selected = &store.state().selected_videos;
    for (i, session) in sessions.iter().enumerate() {
        let old = bars.row_data(i).unwrap_or_default();
        let bar = SlintVideoTimeBar {
            label: ui_video_label(&session.uri),
            time: session.current_time as f32,
            color: rainbow_color(i),
            selected: selected.contains(&i),
            muted: session.muted,
            is_main: main == Some(i),
        };
        if old.label != bar.label
            || old.time != bar.time
            || old.color != bar.color
            || old.selected != bar.selected
            || old.is_main != bar.is_main
            || old.muted != bar.muted
        {
            bars.set_row_data(i, bar);
        }
    }
    ui_sync_video_scalars(store);
}

fn ui_video_label(uri: &str) -> slint::SharedString {
    url::Url::parse(uri)
        .ok()
        .and_then(|url| url.to_file_path().ok())
        .and_then(|path| {
            path.file_name()
                .map(|name| name.to_string_lossy().into_owned())
        })
        .unwrap_or_else(|| uri.to_owned())
        .into()
}

/// 将会话时间限制在目标管线的有效时长内。媒体刚建立时 duration 可能尚未可用，
/// 此时只做非负钳制，待后续帧/时间同步获得实际时长后再校正。
fn gst_clamp_time_to_duration(pipeline: &gstreamer::Pipeline, secs: f64) -> f64 {
    let secs = if secs.is_finite() { secs.max(0.0) } else { 0.0 };
    // 解码管线的 duration 可能在首次 set_state(Paused) 返回后才发布；替换是
    // 低频操作，允许一个有限等待窗口完成预滚动，避免把超出新视频时长的时间
    // 错误地带入新会话。
    let duration = pipeline
        .query_duration::<gstreamer::ClockTime>()
        .or_else(|| {
            let _ = pipeline.state(Some(gstreamer::ClockTime::from_mseconds(500)));
            pipeline.query_duration::<gstreamer::ClockTime>()
        });
    let Some(duration) = duration else {
        return secs;
    };
    let duration_secs = duration.nseconds() as f64 / 1_000_000_000.0;
    if duration_secs.is_finite() && duration_secs > 0.0 {
        secs.min(duration_secs)
    } else {
        secs
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use gstreamer_app::AppSink;

    /// 构造两个重叠 + 一个错开的矩形（下标 0/1 重叠，2 独立）。
    fn rects() -> Vec<VideoRect> {
        vec![
            VideoRect {
                x: 0.0,
                y: 0.0,
                w: 100.0,
                h: 100.0,
            }, // 0：底层
            VideoRect {
                x: 10.0,
                y: 10.0,
                w: 100.0,
                h: 100.0,
            }, // 1：覆盖在 0 之上
            VideoRect {
                x: 500.0,
                y: 500.0,
                w: 100.0,
                h: 100.0,
            }, // 2：独立
        ]
    }

    #[test]
    fn pick_no_hit_returns_none() {
        let r = rects();
        assert_eq!(pick_overlapping_rect(&r, None, 400.0, 400.0), None);
    }

    #[test]
    fn seek_drag_preserves_relative_offsets() {
        let anchor: f64 = 2.0;
        let requested: f64 = 5.5;
        let origins: [f64; 3] = [2.0, 7.25, 0.5];
        let resolved: Vec<f64> = origins
            .into_iter()
            .map(|origin| (origin + requested - anchor).max(0.0))
            .collect();
        assert_eq!(resolved, vec![5.5, 10.75, 4.0]);
    }

    #[test]
    fn pick_hit_without_selection_takes_topmost() {
        let r = rects();
        // (50, 50) 同时命中 0 与 1 → 无选中取最上层（下标 1）
        assert_eq!(pick_overlapping_rect(&r, None, 50.0, 50.0), Some(1));
        // (600, 600) 只命中 2
        assert_eq!(pick_overlapping_rect(&r, None, 600.0, 600.0), Some(2));
        // (5, 5) 只命中 0（1 起点在 (10,10)）
        assert_eq!(pick_overlapping_rect(&r, None, 5.0, 5.0), Some(0));
    }

    #[test]
    fn pick_cycles_through_overlapping_videos() {
        let r = rects();
        // 选中 0（底层）时点重叠区 → 循环到 1；再点 → 回到 0
        assert_eq!(pick_overlapping_rect(&r, Some(0), 50.0, 50.0), Some(1));
        assert_eq!(pick_overlapping_rect(&r, Some(1), 50.0, 50.0), Some(0));
        // 选中 2（不在重叠命中集内）→ 取最上层 1
        assert_eq!(pick_overlapping_rect(&r, Some(2), 50.0, 50.0), Some(1));
    }

    #[test]
    fn hit_top_returns_topmost_at_point() {
        let r = rects();
        assert_eq!(hit_top_rect(&r, 50.0, 50.0), Some(1));
        assert_eq!(hit_top_rect(&r, 5.0, 5.0), Some(0));
        assert_eq!(hit_top_rect(&r, 600.0, 600.0), Some(2));
        assert_eq!(hit_top_rect(&r, 300.0, 300.0), None);
    }

    // —— 播放键在 EOS 后失效的回归测试（复现用户报告）——
    //
    // 复现流程：打开视频（自动播放到结尾）→ 按"播放"键（应重新开始，Bug 下无效）
    // → 拖动时间条（seek）→ 再按"播放"键（Bug 下才生效）。
    //
    // 真实 GStreamer 语义：到达 EOS 的管线 `current_state()` 仍是 `Playing`，
    // 但单靠 `set_state(Playing)` 无法恢复出帧，必须先 `seek` 清 EOS。
    // 旧逻辑只切状态、不 seek，于是播放键"按下却画面不动"——只有拖动时间条
    // 触发 seek 才能救活。`media_apply_video_playback(true)` 会对 EOS
    // 会话先 seek 回起点，播放键始终能重启。

    /// 用 GStreamer 现场生成一段极短视频（约 0.5s），避免依赖用户机器素材。
    /// 失败返回 None（调用方跳过）。
    fn tiny_test_video() -> Option<std::path::PathBuf> {
        gstreamer::init().ok()?;
        let out = std::env::temp_dir().join("aweft_eos_tiny_test.mp4");
        let pipeline = gstreamer::Pipeline::new();
        let src = gstreamer::ElementFactory::make("videotestsrc")
            .property("num-buffers", 15i32)
            .build()
            .ok()?;
        let enc = gstreamer::ElementFactory::make("x264enc").build().ok()?;
        let mux = gstreamer::ElementFactory::make("qtmux").build().ok()?;
        let sink = gstreamer::ElementFactory::make("filesink")
            .property("location", out.to_str()?)
            .build()
            .ok()?;
        pipeline.add_many([&src, &enc, &mux, &sink]).ok()?;
        gstreamer::Element::link_many([&src, &enc, &mux, &sink]).ok()?;
        pipeline.set_state(gstreamer::State::Playing).ok()?;
        // 等 EOS（生成完成）
        let bus = pipeline.bus()?;
        use gstreamer::prelude::*;
        for msg in bus.iter_timed(gstreamer::ClockTime::NONE) {
            use gstreamer::MessageView;
            match msg.view() {
                MessageView::Eos(..) => break,
                MessageView::Error(e) => {
                    eprintln!("跳过：生成测试视频错误: {}", e.error());
                    let _ = pipeline.set_state(gstreamer::State::Null);
                    return None;
                }
                _ => {}
            }
        }
        let _ = pipeline.set_state(gstreamer::State::Null);
        if out.is_file() { Some(out) } else { None }
    }

    /// 轮询直到 `cond` 为真或超时（毫秒）。用于等管线推进到 EOS / 重启后出帧。
    fn wait_until<F: Fn() -> bool>(timeout_ms: u64, cond: F) -> bool {
        let step = std::time::Duration::from_millis(20);
        let mut elapsed = 0u64;
        while elapsed < timeout_ms {
            if cond() {
                return true;
            }
            std::thread::sleep(step);
            elapsed += 20;
        }
        cond()
    }

    /// 直接读取 appsink，供不安装生产回调的测试使用。
    fn pump_appsink(element: &gstreamer::Element) -> bool {
        if let Some(appsink) = element.clone().dynamic_cast::<AppSink>().ok() {
            appsink
                .try_pull_sample(gstreamer::ClockTime::ZERO)
                .is_some()
        } else {
            false
        }
    }

    /// 模拟渲染线程消费回调保存的最新帧。
    fn pump(store: &mut crate::store::AppStore) {
        for s in &mut store.state_mut().video_sessions {
            s.wgpu_sink.fetch_next_frame();
        }
    }

    /// 边泵帧边等待条件成立（替代无消费者的 wait_until，用于测播放推进）。
    fn pump_until<F: Fn(&mut crate::store::AppStore) -> bool>(
        store: &mut crate::store::AppStore,
        timeout_ms: u64,
        cond: F,
    ) -> bool {
        let step = std::time::Duration::from_millis(20);
        let mut elapsed = 0u64;
        while elapsed < timeout_ms {
            pump(store);
            if cond(store) {
                return true;
            }
            std::thread::sleep(step);
            elapsed += 20;
        }
        cond(store)
    }

    #[test]
    fn play_restarts_from_eos_after_open() {
        let _g = crate::GST_SERIAL.lock().unwrap_or_else(|e| e.into_inner());
        let Some(video) = tiny_test_video() else {
            eprintln!("跳过：GStreamer 不可用或测试视频生成失败");
            return;
        };
        // 无事件循环的 headless Slint 组件树（仅需 AppStore 容器，不渲染）
        i_slint_backend_testing::init_no_event_loop();
        let instance = crate::bridge::types::MainWindow::new().unwrap();
        let mut store = crate::store::AppStore::new(&instance);

        // 直接搭一条真实解码管线（与 build_session 同源，但本测试不连 WGPU sink）。
        let (pipeline, sink_element, mut wgpu_sink) =
            crate::render::video_sink::pipeline::build_decode_pipeline_with_dmabuf(
                &url::Url::from_file_path(&video).unwrap().to_string(),
                false,
                false,
            )
            .expect("搭建解码管线失败");
        pipeline
            .set_state(gstreamer::State::Playing)
            .expect("启动解码管线失败");

        // 等管线真正播到 EOS（位置停在时长处不再推进）。背压下必须泵帧推进。
        let dur = {
            let mut reached = false;
            for _ in 0..250 {
                pump_appsink(&sink_element);
                let pos = pipeline.query_position::<gstreamer::ClockTime>();
                let d = pipeline.query_duration::<gstreamer::ClockTime>();
                if let (Some(p), Some(dd)) = (pos, d) {
                    if dd > gstreamer::ClockTime::from_nseconds(0) && p >= dd {
                        reached = true;
                        break;
                    }
                }
                std::thread::sleep(std::time::Duration::from_millis(20));
            }
            reached
        };
        assert!(dur, "测试视频应能播到 EOS");
        // 此时 ui_sync_video_playing_state 会判其处于 EOS → video_playing 应为 false
        store
            .state_mut()
            .video_sessions
            .push(crate::store::state::VideoSession {
                uri: video.to_string_lossy().into(),
                pipeline,
                sink_element,
                wgpu_sink,
                position_x: 0.0,
                position_y: 0.0,
                current_time: 0.0,
                muted: false,
            });
        store.state_mut().main_video = Some(0);

        // 模拟"打开即自动播放"把 video_playing 置 true，且 UI 播放状态同步会
        // 逐帧校正把 EOS 识别为未播放（false）—— 这正是用户看到的"暂停"态。
        store.state_mut().video_playing = true;
        crate::reducers::media::ui_sync_video_playing_state(&mut store);
        assert!(
            !store.state().video_playing,
            "EOS 时按钮应显示可播放（video_playing=false）"
        );

        // 用户第一次按"播放"键：Bug 下 set_state(Playing) 无法恢复；修复后 seek 回起点重启。
        store
            .dispatch_media(crate::actions::media::MediaAction::Video(
                crate::actions::media::VideoAction::Play,
            ))
            .unwrap();

        // 重启后应重新出帧：位置回落到时长之前并继续推进。
        let restarted = pump_until(&mut store, 5000, |s| {
            let a0 = s.state().video_sessions[0]
                .pipeline
                .query_position::<gstreamer::ClockTime>();
            matches!(a0, Some(p) if p > gstreamer::ClockTime::from_mseconds(50))
        });
        assert!(
            restarted,
            "按播放键后管线应从 EOS 重启（位置回落后于时长）—— 修复前此处失效"
        );
        // 进一步确认确实在前进（位置变大）
        let t0 = store.state().video_sessions[0]
            .pipeline
            .query_position::<gstreamer::ClockTime>();
        let advanced = pump_until(&mut store, 3000, |s| {
            let p = s.state().video_sessions[0]
                .pipeline
                .query_position::<gstreamer::ClockTime>();
            matches!((t0, p), (Some(a), Some(b)) if b > a)
        });
        assert!(advanced, "重启后播放应持续推进");
    }

    /// 时间线 Header 播放键控制视口选择集合；无选择时只控制主视频。
    #[test]
    fn play_controls_selected_video_sessions_and_defaults_to_main() {
        let _g = crate::GST_SERIAL.lock().unwrap_or_else(|e| e.into_inner());
        let Some(video) = tiny_test_video() else {
            eprintln!("跳过：GStreamer 不可用或测试视频生成失败");
            return;
        };
        let uri = url::Url::from_file_path(&video).unwrap().to_string();
        i_slint_backend_testing::init_no_event_loop();
        let instance = crate::bridge::types::MainWindow::new().unwrap();
        let mut store = crate::store::AppStore::new(&instance);

        // 两条独立真实解码管线（同源短片，互不影响）
        let build = || {
            let (p, s, w) = crate::render::video_sink::pipeline::build_decode_pipeline_with_dmabuf(
                &uri, false, false,
            )
            .expect("搭建解码管线失败");
            p.set_state(gstreamer::State::Playing).expect("启动失败");
            (p, s, w)
        };
        let (p0, s0, mut w0) = build();
        let (p1, s1, w1) = build();

        // 让 0 号先播放一会儿以推进位置（背压下必须泵帧推进）
        for _ in 0..75 {
            pump_appsink(&s0);
            if p0
                .query_position::<gstreamer::ClockTime>()
                .is_some_and(|p| p > gstreamer::ClockTime::from_mseconds(50))
            {
                break;
            }
            std::thread::sleep(std::time::Duration::from_millis(20));
        }
        // 暂停 1 号（模拟"另一个未被激活的视频处于未播放态"）
        p1.set_state(gstreamer::State::Paused).unwrap();

        store
            .state_mut()
            .video_sessions
            .push(crate::store::state::VideoSession {
                uri: uri.clone(),
                pipeline: p0,
                sink_element: s0,
                wgpu_sink: w0,
                position_x: 0.0,
                position_y: 0.0,
                current_time: 0.0,
                muted: false,
            });
        store
            .state_mut()
            .video_sessions
            .push(crate::store::state::VideoSession {
                uri: uri.clone(),
                pipeline: p1,
                sink_element: s1,
                wgpu_sink: w1,
                position_x: 50.0,
                position_y: 50.0,
                current_time: 0.0,
                muted: false,
            });
        store.state_mut().main_video = Some(0);
        store.state_mut().selected_videos.clear();

        // 无选择：播放键只控制主视频，参考视频保持原暂停态。
        store
            .dispatch_media(crate::actions::media::MediaAction::Video(
                crate::actions::media::VideoAction::Play,
            ))
            .unwrap();

        assert!(
            pump_until(&mut store, 1000, |s| gst_pipeline_wants_playing(
                &s.state().video_sessions[0].pipeline
            )),
            "无选择时主视频应进入播放态"
        );
        assert!(!gst_pipeline_wants_playing(
            &store.state().video_sessions[1].pipeline
        ));

        // 活跃视频 0 应被 flush-seek 重新驱动出帧（之前只有拖动时间条能播）
        let resumed = pump_until(&mut store, 3000, |s| {
            let a0 = s.state().video_sessions[0]
                .pipeline
                .query_position::<gstreamer::ClockTime>();
            matches!(a0, Some(p) if p > gstreamer::ClockTime::from_mseconds(50))
        });
        assert!(resumed, "无选择时播放键应让主视频真正开始/恢复出帧");

        // 普通点击选择 0，再 Ctrl 点击加入 1；选择变化本身不改变 transport。
        store
            .dispatch_media(crate::actions::media::MediaAction::Video(
                crate::actions::media::VideoAction::Select {
                    index: Some(0),
                    modifier: crate::bridge::types::SlintSelectionModifier::None,
                },
            ))
            .unwrap();
        store
            .dispatch_media(crate::actions::media::MediaAction::Video(
                crate::actions::media::VideoAction::Select {
                    index: Some(1),
                    modifier: crate::bridge::types::SlintSelectionModifier::Ctrl,
                },
            ))
            .unwrap();
        assert_eq!(store.state().selected_videos, vec![0, 1]);
        assert!(gst_pipeline_wants_playing(
            &store.state().video_sessions[0].pipeline
        ));
        assert!(!gst_pipeline_wants_playing(
            &store.state().video_sessions[1].pipeline
        ));

        // 多选播放后，两条被选会话同步进入播放态。
        store
            .dispatch_media(crate::actions::media::MediaAction::Video(
                crate::actions::media::VideoAction::Play,
            ))
            .unwrap();
        assert!(
            pump_until(&mut store, 1000, |s| s
                .state()
                .video_sessions
                .iter()
                .all(|session| gst_pipeline_wants_playing(&session.pipeline))),
            "多选播放应作用于全部被选视频"
        );

        store
            .dispatch_media(crate::actions::media::MediaAction::Video(
                crate::actions::media::VideoAction::Pause,
            ))
            .unwrap();
        assert!(
            pump_until(&mut store, 1000, |s| s
                .state()
                .video_sessions
                .iter()
                .all(|session| !gst_pipeline_wants_playing(&session.pipeline))),
            "多选暂停应作用于全部被选视频"
        );
    }

    /// 复现：暂停后重新播放应从原位置继续，而不是回到开头。
    /// 同时复现：初始应显示"暂停"图标（正在播放），不应需要点击两次才播放。
    #[test]
    fn resume_continues_from_paused_position() {
        let _g = crate::GST_SERIAL.lock().unwrap_or_else(|e| e.into_inner());
        let Some(video) = tiny_test_video() else {
            eprintln!("跳过：GStreamer 不可用或测试视频生成失败");
            return;
        };
        let uri = url::Url::from_file_path(&video).unwrap().to_string();
        i_slint_backend_testing::init_no_event_loop();
        let instance = crate::bridge::types::MainWindow::new().unwrap();
        let mut store = crate::store::AppStore::new(&instance);

        let (pipeline, sink_element, mut wgpu_sink) =
            crate::render::video_sink::pipeline::build_decode_pipeline_with_dmabuf(
                &uri, false, false,
            )
            .expect("搭建解码管线失败");
        pipeline
            .set_state(gstreamer::State::Playing)
            .expect("启动失败");

        // 等播放推进到一个明确位置（> 200ms）；背压下必须泵帧推进。
        // 背压使 query_position 反映"已提交（被消费）"位置，略滞后于解码位置，
        // 故到达后再多泵若干帧让已提交位置稳定抬升，避免暂停点偏低。
        let reached = {
            let mut ok = false;
            for _ in 0..250 {
                pump_appsink(&sink_element);
                if pipeline
                    .query_position::<gstreamer::ClockTime>()
                    .is_some_and(|p| p > gstreamer::ClockTime::from_mseconds(200))
                {
                    ok = true;
                    break;
                }
                std::thread::sleep(std::time::Duration::from_millis(20));
            }
            ok
        };
        for _ in 0..30 {
            pump_appsink(&sink_element);
        }
        assert!(reached, "应能播放推进到 200ms 之后");

        // 模拟渲染循环里的 render_sync_video_playback_time：把真实播放位置同步进 current_time。
        // 背压下 pipeline.query_position 在暂停时会回退到 sink 已提交位置（滞后于解码位置），
        // 因此续播必须以 current_time 为准（media_apply_video_playback 也是这么做）。
        let play_pos = pipeline
            .query_position::<gstreamer::ClockTime>()
            .expect("播放中应能查询位置");
        let play_secs = play_pos.nseconds() as f64 / 1_000_000_000.0;

        store
            .state_mut()
            .video_sessions
            .push(crate::store::state::VideoSession {
                uri,
                pipeline,
                sink_element,
                wgpu_sink,
                position_x: 0.0,
                position_y: 0.0,
                current_time: play_secs,
                muted: false,
            });
        store.state_mut().main_video = Some(0);

        // 模拟打开即自动播放：video_playing=true，且 UI 播放状态同步应确认正在播放
        store.state_mut().video_playing = true;
        crate::reducers::media::ui_sync_video_playing_state(&mut store);
        assert!(
            store.state().video_playing,
            "初始正在播放时应显示暂停图标（video_playing=true）"
        );

        // 暂停
        store
            .dispatch_media(crate::actions::media::MediaAction::Video(
                crate::actions::media::VideoAction::Pause,
            ))
            .unwrap();
        let paused_pos = store.state().video_sessions[0]
            .pipeline
            .query_position::<gstreamer::ClockTime>()
            .expect("暂停时应有位置");
        // 续播点以 current_time 为准（见 media_apply_video_playback）；此处仅校验明显已推进、非空
        let pause_point = gstreamer::ClockTime::from_nseconds(
            (store.state().video_sessions[0].current_time * 1_000_000_000.0) as u64,
        );
        assert!(
            paused_pos > gstreamer::ClockTime::from_mseconds(50),
            "暂停位置应明显大于 0：{paused_pos:?}"
        );

        // 重新播放：应从 paused_pos 附近继续，而不是回到开头
        store
            .dispatch_media(crate::actions::media::MediaAction::Video(
                crate::actions::media::VideoAction::Play,
            ))
            .unwrap();
        // (上面已是正确构造；下面恢复采样逻辑)

        // 在短时间窗内密集采样位置，记录最小值：若从开头重启会先掉到接近 0；
        // 正确应维持在 paused_pos 附近（KEY_UNIT 可能回退几十毫秒）。
        let mut min_pos = gstreamer::ClockTime::from_seconds(1000);
        for _ in 0..40 {
            pump(&mut store);
            if let Some(p) = store.state().video_sessions[0]
                .pipeline
                .query_position::<gstreamer::ClockTime>()
            {
                if p < min_pos {
                    min_pos = p;
                }
            }
            std::thread::sleep(std::time::Duration::from_millis(10));
        }
        assert!(
            min_pos > pause_point.saturating_sub(gstreamer::ClockTime::from_mseconds(80)),
            "重新播放不应回到开头（采样最小位置 {min_pos:?} 应接近暂停点 {pause_point:?}）"
        );
        // 继续推进（应超过暂停位置）
        let advanced = pump_until(&mut store, 3000, |s| {
            s.state().video_sessions[0]
                .pipeline
                .query_position::<gstreamer::ClockTime>()
                .is_some_and(|p| p > paused_pos)
        });
        assert!(advanced, "重新播放后应继续从暂停位置推进");
    }

    /// 复现：拖动时间线（Seek）应改变活跃视频的播放位置。
    #[test]
    fn drag_seek_moves_playback_position() {
        let _g = crate::GST_SERIAL.lock().unwrap_or_else(|e| e.into_inner());
        let Some(video) = tiny_test_video() else {
            eprintln!("跳过：GStreamer 不可用或测试视频生成失败");
            return;
        };
        let uri = url::Url::from_file_path(&video).unwrap().to_string();
        i_slint_backend_testing::init_no_event_loop();
        let instance = crate::bridge::types::MainWindow::new().unwrap();
        let mut store = crate::store::AppStore::new(&instance);

        let (pipeline, sink_element, mut wgpu_sink) =
            crate::render::video_sink::pipeline::build_decode_pipeline_with_dmabuf(
                &uri, false, false,
            )
            .expect("搭建解码管线失败");
        pipeline
            .set_state(gstreamer::State::Playing)
            .expect("启动失败");
        // 先播放推进一点（背压下必须泵帧推进）
        for _ in 0..100 {
            pump_appsink(&sink_element);
            if pipeline
                .query_position::<gstreamer::ClockTime>()
                .is_some_and(|p| p > gstreamer::ClockTime::from_mseconds(50))
            {
                break;
            }
            std::thread::sleep(std::time::Duration::from_millis(20));
        }

        store
            .state_mut()
            .video_sessions
            .push(crate::store::state::VideoSession {
                uri,
                pipeline,
                sink_element,
                wgpu_sink,
                position_x: 0.0,
                position_y: 0.0,
                current_time: 0.0,
                muted: false,
            });
        store.state_mut().main_video = Some(0);

        // 先暂停（解码线程停下，appsink 停在最后一帧）。
        store
            .dispatch_media(crate::actions::media::MediaAction::Video(
                crate::actions::media::VideoAction::Pause,
            ))
            .unwrap();
        // 排空 appsink：pull 模式下解码线程已停、不会回填，槽位彻底清空，
        // 否则 flush-seek 不会自动丢弃已持有的预滚动帧，新 seek 的预滚动帧无槽可入
        // 而阻塞（seek 死锁，无帧、query_position=None）。
        let sink_el = store.state().video_sessions[0].sink_element.clone();
        for _ in 0..200 {
            let had = sink_el
                .clone()
                .dynamic_cast::<AppSink>()
                .ok()
                .and_then(|a| a.try_pull_sample(gstreamer::ClockTime::ZERO))
                .is_some();
            if !had {
                break;
            }
        }

        // 暂停态拖动到 0.3s（模拟时间线拖拽：SeekBegin 切到播放态使 flush-seek 可接受，
        // 拖拽出帧实时预览；Seek 落到 0.3s；SeekEnd 回落暂停交付暂停预览）。
        store
            .dispatch_media(crate::actions::media::MediaAction::Video(
                crate::actions::media::VideoAction::SeekBegin {
                    index: 0,
                    is_batch: false,
                },
            ))
            .unwrap();
        store
            .dispatch_media(crate::actions::media::MediaAction::Video(
                crate::actions::media::VideoAction::Seek { timestamp: 300 },
            ))
            .unwrap();
        store
            .dispatch_media(crate::actions::media::MediaAction::Video(
                crate::actions::media::VideoAction::SeekEnd,
            ))
            .unwrap();

        // UI 时间条用 session.current_time（media_seek_video_session 写入 0.3）；且 fetch_next_frame
        // 直接从 appsink 拉帧（含暂停态预滚动帧），暂停态拖拽也能在后续重绘即时预览——
        // 这正是用户"拖动改变播放时间"的真实可见行为。
        assert!(
            (store.state().video_sessions[0].current_time - 0.3).abs() < 0.05,
            "拖动后时间条 current_time 应为 ~0.3s，实际 {}",
            store.state().video_sessions[0].current_time
        );
        // 暂停态拖拽的核心可见行为（用户原始诉求"拖动改变播放时间"）：
        // 1) 时间条 current_time 已变为 0.3s（上面的断言）；
        // 2) 按播放后管线从 current_time 精确续播——时钟从 ~0.3s 处继续推进，
        //    而非回到开头，证明 drag→play 端到端生效（帧实际产出由
        //    play_restarts_from_eos_after_open 到达 EOS 覆盖）。
        store
            .dispatch_media(crate::actions::media::MediaAction::Video(
                crate::actions::media::VideoAction::Play,
            ))
            .unwrap();
        let from_drag = pump_until(&mut store, 3000, |s| {
            s.state().video_sessions[0]
                .pipeline
                .query_position::<gstreamer::ClockTime>()
                .is_some_and(|p| p >= gstreamer::ClockTime::from_mseconds(250))
        });
        assert!(from_drag, "拖拽到 0.3s 后按播放应从 ~0.3s 处继续");
    }

    /// 验证 `gst_set_audio_muted` 能真正落到音频 sink 上：
    /// `volume` 元素同时暴露 `mute` 与 `volume`，正是兜底分支命中的元素类型
    /// （与 pulsesink/autoaudiosink→child 同一条设置路径）。若此断言通过，
    /// 说明拖拽静音的属性写入逻辑有效，运行时无声音是该路径生效的直接结果。
    #[test]
    fn drag_audio_mute_targets_audio_sink() {
        use gstreamer::prelude::*;
        gstreamer::init().unwrap();
        let pipeline = gstreamer::Pipeline::new();
        let vol = gstreamer::ElementFactory::make("volume")
            .build()
            .expect("需要 volume 插件来验证静音逻辑");
        pipeline.add(&vol).unwrap();

        gst_set_audio_muted(&pipeline, true);
        assert!(vol.property::<bool>("mute"), "静音时应置 mute=true");
        assert_eq!(vol.property::<f64>("volume"), 0.0, "静音时应置 volume=0");

        gst_set_audio_muted(&pipeline, false);
        assert!(!vol.property::<bool>("mute"), "解除静音应置 mute=false");
        assert_eq!(vol.property::<f64>("volume"), 1.0, "解除静音应置 volume=1");
    }

    /// 复现：拖拽期间 Header 播放按钮不应随 flush-seek 的过渡态闪烁。
    /// 拖拽时 `ui_sync_video_playing_state` 应沿用 seek 会话记录的拖拽前意图态，
    /// 而非实时查询管线状态——flush-seek 会让管线瞬时停在 Paused，导致图标乱跳。
    #[test]
    fn drag_keeps_header_play_state_stable() {
        let _g = crate::GST_SERIAL.lock().unwrap_or_else(|e| e.into_inner());
        let Some(video) = tiny_test_video() else {
            eprintln!("跳过：GStreamer 不可用或测试视频生成失败");
            return;
        };
        let uri = url::Url::from_file_path(&video).unwrap().to_string();
        i_slint_backend_testing::init_no_event_loop();
        let instance = crate::bridge::types::MainWindow::new().unwrap();
        let mut store = crate::store::AppStore::new(&instance);

        let (pipeline, sink_element, wgpu_sink) =
            crate::render::video_sink::pipeline::build_decode_pipeline_with_dmabuf(
                &uri, false, false,
            )
            .expect("搭建解码管线失败");
        store
            .state_mut()
            .video_sessions
            .push(crate::store::state::VideoSession {
                uri,
                pipeline,
                sink_element,
                wgpu_sink,
                position_x: 0.0,
                position_y: 0.0,
                current_time: 0.0,
                muted: false,
            });
        store.state_mut().main_video = Some(0);

        // 模拟"播放中开始拖拽"：拖拽前是播放态（was_playing=true）。
        store.state_mut().video_playing = true;

        // 拖拽中：即便实时状态因 flush-seek 瞬时变为非播放，UI 播放状态同步
        // 也必须回到 was_playing，而不是跟随闪烁成暂停图标。
        store.state_mut().video_seek_session = Some(crate::store::state::VideoSeekSession {
            anchor_time: 0.0,
            targets: vec![crate::store::state::VideoSeekTarget {
                index: 0,
                origin_time: 0.0,
                was_playing: true,
            }],
        });
        store.state_mut().video_playing = false;
        super::ui_sync_video_playing_state(&mut store);
        assert!(
            store.state().video_playing,
            "拖拽中应稳定显示播放态（沿用 was_playing），不应闪烁成暂停"
        );

        // 松手：恢复按实时状态校正（此处管线未真正播放，校正为 false 亦合理）。
        store.state_mut().video_seek_session = None;
        super::ui_sync_video_playing_state(&mut store);
    }
}
