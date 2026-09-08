use crate::store::AppStore;
use crate::store::state::VideoSession;
use gstreamer::prelude::*;

pub(super) fn gst_set_video_session_state(session: &mut VideoSession, target: gstreamer::State) {
    if let Err(error) = session.pipeline.set_state(target) {
        log::warn!("切换视频状态到 {target:?} 失败: {error}");
        return;
    }
    session.playing = target == gstreamer::State::Playing;
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
fn media_set_video_playing(store: &mut AppStore, playing: bool) {
    store.state_mut().video_playing = playing;
}

/// 应用时间线 transport：播放/暂停作用于视口选择集合；无选择时只作用于主视频。
/// 播放从各自的 `current_time` 启动，参考视频仍可独立 seek。
///
/// 进入 Playing 前先对管线做一次 flush-seek，把它置回已知良好状态
/// （正向 segment、无 EOS 锁存），再 `set_state(Playing)`。其中 seek 是
/// "必须"的有两种情形：
/// 1. EOS 锁存：播完的管线 `current_state` 仍是 `Playing`，但下游锁存的
///    sticky EOS 只有 flush-seek 能清除——`at_end` 时 seek 回起点重播；
/// 2. 负速率残留：暂停态越界定位会 park 一个 rate=-1 的末帧 segment
///    （见 `media_seek_video_session`），不先发正向 seek 就会倒放或卡住。
/// 除此之外的普通暂停→播放，seek 只是统一的防御措施（等价于用户拖动时间条，
/// 花一次 flush 免去逐个分辨状态），并非 GStreamer 要求。这正是"拖动时间条能播、
/// 旧播放键不动"的原因：
/// 拖动自带 seek 清状态，旧播放键只有 `set_state`，落在锁存态上就是空操作。
pub(super) fn media_apply_video_transport(store: &mut AppStore, playing: bool) -> Vec<usize> {
    if store.state().video_sessions.is_empty() {
        media_set_video_playing(store, false);
        return Vec::new();
    }
    let targets = media_control_targets(store);
    for &index in &targets {
        let session = &mut store.state_mut().video_sessions[index];
        if playing {
            let at_end = media_video_session_at_display_end(session)
                || session.at_eos.load(std::sync::atomic::Ordering::Acquire);
            let start_secs = if at_end {
                0.0
            } else {
                session.current_time.max(0.0)
            };
            // 必须先用正速率 seek 替换可能残留的负速率末帧 segment，再进入
            // Playing；反过来会让管线在两次调用之间短暂倒放。
            // GStreamer 在某些解码器上从 PAUSED 直接切 PLAYING 不会重新
            // 唤醒数据流，必须先 flush-seek。seek 失败只记录日志，不能阻止
            // 后续进入 PLAYING（否则一次瞬时 seek 失败会让 Header 播放永久失效）。
            if at_end {
                let _ = gst_seek_pipeline_key_unit(&session.pipeline, start_secs);
            } else {
                let _ = gst_seek_pipeline_accurate(&session.pipeline, start_secs);
            }
            session.current_time = start_secs;
            session
                .at_eos
                .store(false, std::sync::atomic::Ordering::Release);
            session.last_frame_seeked = false;
            gst_set_video_session_state(session, gstreamer::State::Playing);
        } else {
            gst_set_video_session_state(session, gstreamer::State::Paused);
        }
        // 音频分支与会话管线同源同钟：上面的 seek/状态切换随管线一起作用于音频，
        // 不需要（也不能）单独驱动一条独立音频管线。
    }
    media_set_video_playing(store, playing);
    targets
}

/// 测试/诊断用的管线播放态快照。生产逐帧路径读取 `VideoSession::playing` 缓存。
/// 用零超时做**非阻塞**快照——`state(None)` 会阻塞到目标态达成（GST_CLOCK_TIME_NONE），
/// 到达 EOS 的管线 `current_state` 仍是 `Playing`，因此不能用它判断 EOS。
#[cfg(test)]
pub(crate) fn gst_pipeline_wants_playing(pipeline: &gstreamer::Pipeline) -> bool {
    let (_, current, pending) = pipeline.state(Some(gstreamer::ClockTime::from_mseconds(0)));
    if pending != gstreamer::State::VoidPending {
        pending == gstreamer::State::Playing
    } else {
        current == gstreamer::State::Playing
    }
}

/// 当前焦点视频是否应以实际播放时间驱动动画时间轴。多视频的全局 transport 标量
/// 不能回答这个问题：其它视频可以仍在播放，而焦点视频已经暂停。拖拽期间管线会因
/// flush-seek 短暂进入过渡态，因此优先使用该焦点目标在拖拽开始时记录的播放意图。
pub(crate) fn media_should_active_video_drive_timeline(store: &AppStore) -> bool {
    let Some(active) = store.state().active_video() else {
        return false;
    };
    if let Some(target) = store
        .state()
        .video_seek_session
        .as_ref()
        .and_then(|drag| drag.targets.iter().find(|target| target.index == active))
    {
        return target.was_playing;
    }
    store
        .state()
        .video_sessions
        .get(active)
        .is_some_and(|session| {
            session.playing && !session.at_eos.load(std::sync::atomic::Ordering::Acquire)
        })
}

/// 按当前 transport 目标的缓存状态校正全局 `video_playing`。播放/暂停命令即时
/// 设置该值；生产渲染路径仅在 Bus 报告 EOS 后调用一次，不逐帧查询 GStreamer。
/// 已到 EOS 的管线仍报 Playing，但画面已停，故按"未播放"处理，让按钮正确显示
/// 为"播放"且下一次按下能真正重新开始（见 `media_apply_video_transport` 中的 EOS 重启）。
///
/// 拖拽期间不参与实时校正：拖拽会对管线做大量 flush-seek，其过渡态会让
/// 管线状态会瞬时抖动；此时沿用拖拽目标开始时的播放意图。
pub(crate) fn media_refresh_video_playing_state(store: &mut AppStore) -> bool {
    let playing = if let Some(drag) = &store.state().video_seek_session {
        !drag.targets.is_empty() && drag.targets.iter().all(|target| target.was_playing)
    } else {
        let targets = media_control_targets(store);
        !targets.is_empty()
            && targets.iter().all(|&index| {
                let session = &store.state().video_sessions[index];
                session.playing && !session.at_eos.load(std::sync::atomic::Ordering::Acquire)
            })
    };
    if store.state().video_playing == playing {
        return false;
    }
    media_set_video_playing(store, playing);
    true
}

/// 跳转单个会话到指定秒（编辑器拖动播放头/时间条反向驱动视频）。视频画面只能
/// 落在自身有效范围内，因此会话时间和 GStreamer seek 都使用钳制值；动画时间轴
/// 是否保留原始 UI 请求值由 VideoTimeChanged.sync_timeline 决定。
pub(super) fn media_seek_video_session(session: &mut VideoSession, secs: f64) {
    let requested_secs = secs;
    let secs = media_clamp_video_seek_time(session, secs);
    let requested_past_end = session
        .duration
        .filter(|duration| duration.is_finite() && *duration > 0.0)
        .is_some_and(|duration| requested_secs.is_finite() && requested_secs >= duration);
    let repeated_last_frame = media_repeated_last_frame_seek(
        requested_past_end,
        session.last_frame_seeked,
        session.current_time,
        secs,
    );
    session.current_time = secs;
    // 暂停态 seek 产生的 preroll 会由 sink 回调保存并请求一次重绘。
    // 正向 ACCURATE seek 的目标若晚于最后一帧 PTS，即使仍小于容器 duration，
    // 解码器也会按 segment 起点裁掉最后一帧并直接 EOS。暂停态越过结尾时改为
    // 从媒体尾边界反向精确 seek，首个 preroll 就是最后一个可解码视频帧。
    // 不读取瞬时管线状态决定是否进入该分支：播放中的 flush-seek 会短暂显示为
    // Paused，曾导致同一交互随机残留负速率 segment。越界行为固定为先暂停。
    if requested_past_end {
        if let Some(duration) = session.duration {
            if repeated_last_frame {
                return;
            }
            gst_set_video_session_state(session, gstreamer::State::Paused);
            // 状态切换与反向 segment 都是异步请求；不能在 UI 回调中等待管线稳定。
            // 清掉在途旧帧后，末帧 preroll 回调会覆盖槽位并请求重绘。
            if let Ok(mut next_frame) = session.wgpu_sink.next_frame.lock() {
                next_frame.take();
            }
            session
                .at_eos
                .store(false, std::sync::atomic::Ordering::Release);
            session.last_frame_seeked = true;
            if !gst_seek_pipeline_last_frame(&session.pipeline, duration) {
                session.last_frame_seeked = false;
                gst_seek_pipeline_accurate(&session.pipeline, secs);
            }
            return;
        }
    }
    session
        .at_eos
        .store(false, std::sync::atomic::Ordering::Release);
    session.last_frame_seeked = false;
    gst_seek_pipeline_accurate(&session.pipeline, secs);
}

pub(super) fn media_repeated_last_frame_seek(
    requested_past_end: bool,
    last_frame_seeked: bool,
    current_time: f64,
    clamped_time: f64,
) -> bool {
    requested_past_end && last_frame_seeked && (current_time - clamped_time).abs() < 0.0005
}

/// 把视频画面时间限制在最近的有效时间点。优先使用会话缓存时长；缓存尚未建立时
/// 非阻塞查询管线并回填，查询仍不可用则只保证非负，后续帧同步会补齐时长。
fn media_clamp_video_seek_time(session: &mut VideoSession, secs: f64) -> f64 {
    if session.duration.is_none() {
        session.duration =
            crate::render::video_sink::gst_query_pipeline_duration_secs(&session.pipeline);
    }
    media_clamp_time_to_range(
        secs,
        session.duration,
        session.wgpu_sink.frame_duration_secs(),
    )
}

/// 纯时间钳制：无有效媒体时长时只钳制到零；超过结尾时落到最后一个可显示帧的
/// 起始时间，避免 seek 到 EOS 边界后拿不到 preroll。帧时长未知时退回结尾前 1ms。
pub(super) fn media_clamp_time_to_range(
    secs: f64,
    duration: Option<f64>,
    frame_duration: Option<f64>,
) -> f64 {
    let secs = if secs.is_finite() { secs.max(0.0) } else { 0.0 };
    let Some(duration) = duration.filter(|duration| duration.is_finite() && *duration > 0.0) else {
        return secs;
    };
    if secs < duration {
        return secs;
    }
    let tail_margin = frame_duration
        .filter(|frame_duration| frame_duration.is_finite() && *frame_duration > 0.0)
        .unwrap_or(0.001);
    (duration - tail_margin).max(0.0)
}

/// 会话是否停在用于显示的末帧位置。既用于拖拽结束时决定是否自动暂停，也用于
/// 再次按播放时从头建立正向 segment，避免继续沿末帧定位使用的负速率 segment。
pub(super) fn media_video_session_at_display_end(session: &VideoSession) -> bool {
    let Some(duration) = session
        .duration
        .filter(|duration| duration.is_finite() && *duration > 0.0)
    else {
        return false;
    };
    let display_end = media_clamp_time_to_range(
        duration,
        Some(duration),
        session.wgpu_sink.frame_duration_secs(),
    );
    session.current_time + 0.0005 >= display_end
}

/// 快速跳转（FLUSH + KEY_UNIT）：落到最近关键帧，解码代价小。只用于对精确帧
/// 无要求的场合（EOS 重播回 0 秒：流起点本身就是关键帧）。
fn gst_seek_pipeline_key_unit(pipeline: &gstreamer::Pipeline, secs: f64) -> bool {
    gst_seek_pipeline_with_flags(
        pipeline,
        secs,
        gstreamer::SeekFlags::FLUSH | gstreamer::SeekFlags::KEY_UNIT,
    )
}

/// 精确跳转（FLUSH + ACCURATE）：落到**确切**位置。用于暂停恢复与替换负速率
/// segment——KEY_UNIT 可能回退到目标之前的关键帧。与 key_unit 的唯一区别是
/// 定位标志，FLUSH 清流语义两者相同。
pub(super) fn gst_seek_pipeline_accurate(pipeline: &gstreamer::Pipeline, secs: f64) -> bool {
    gst_seek_pipeline_with_flags(
        pipeline,
        secs,
        gstreamer::SeekFlags::FLUSH | gstreamer::SeekFlags::ACCURATE,
    )
}

/// 暂停态定位最后一帧：以负速率从媒体尾边界向前建立精确 segment。负速率从
/// stop 位置开始，appsink 收到的首个 preroll 因而是尾边界之前的最后一帧。
fn gst_seek_pipeline_last_frame(pipeline: &gstreamer::Pipeline, duration: f64) -> bool {
    let duration =
        gstreamer::ClockTime::from_nseconds((duration.max(0.0) * 1_000_000_000.0).round() as u64);
    if let Err(error) = pipeline.seek(
        -1.0,
        gstreamer::SeekFlags::FLUSH | gstreamer::SeekFlags::ACCURATE,
        gstreamer::SeekType::Set,
        gstreamer::ClockTime::ZERO,
        gstreamer::SeekType::Set,
        duration,
    ) {
        log::warn!("视频末帧 seek 失败: {error}");
        return false;
    }
    true
}

/// 跳转管线到指定秒（带指定 seek 标志）。
fn gst_seek_pipeline_with_flags(
    pipeline: &gstreamer::Pipeline,
    secs: f64,
    flags: gstreamer::SeekFlags,
) -> bool {
    if let Err(e) = pipeline.seek_simple(
        flags,
        gstreamer::ClockTime::from_mseconds((secs.max(0.0) * 1000.0) as u64),
    ) {
        log::debug!("视频 seek 暂未成功（管线可能仍在状态转换）: {e}");
        return false;
    }
    true
}

/// 播放中会话的位置快照：`secs` 为管线当前位置（秒），`frame_duration_secs`
/// 为帧时长（秒；未知/可变帧率为 None，调用方回退为不量化）。
pub(crate) struct VideoPlayingPosition {
    pub(crate) secs: f64,
    pub(crate) frame_duration_secs: Option<f64>,
}

/// 查询单个会话播放位置：仅播放中返回（暂停时编辑器是时间权威，避免拖动
/// 播放头被陈旧位置回写）。无位置返回 None。
pub(crate) fn gst_session_playing_position(session: &VideoSession) -> Option<VideoPlayingPosition> {
    if !session.playing || session.at_eos.load(std::sync::atomic::Ordering::Acquire) {
        return None;
    }
    // 纳秒转秒：seconds() 会截断到整秒，导致播放头一秒一跳
    let secs = session
        .pipeline
        .query_position::<gstreamer::ClockTime>()?
        .nseconds() as f64
        / 1_000_000_000.0;
    let frame_duration_secs = session.wgpu_sink.frame_duration_secs();
    Some(VideoPlayingPosition {
        secs,
        frame_duration_secs,
    })
}

/// 将会话时间限制在目标管线的有效时长内。媒体刚建立时 duration 可能尚未可用，
/// 此时只做非负钳制，待后续帧/时间同步获得实际时长后再校正。
pub(super) fn gst_clamp_time_to_duration(pipeline: &gstreamer::Pipeline, secs: f64) -> f64 {
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
