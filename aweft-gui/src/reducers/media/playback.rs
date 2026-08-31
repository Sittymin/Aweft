use crate::store::AppStore;
use crate::store::state::VideoSession;
use gstreamer::prelude::*;

pub(super) fn gst_set_session_state(session: &VideoSession, target: gstreamer::State) {
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
pub(super) fn gst_set_audio_muted(pipeline: &gstreamer::Pipeline, muted: bool) {
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
/// current_time，因此参考视频仍可独立 seek；按下播放后从各自位置同步启动。
///
/// 发起播放时不仅要 `set_state(Playing)`，还要对该管线做一次 flush-seek：
/// 1. 处于 EOS（播完）的管线 `current_state` 仍是 `Playing`，但单靠
///    `set_state(Playing)` 不会重新出帧——必须先 `seek` 清 EOS（EOS 时回到起点）；
/// 2. 暂停/卡顿的管线同样只有 seek（flush）才能真正恢复出帧，这正是"拖动时间条
///    能播、按播放键不动"的原因。这里直接在播放时 seek 到当前位置，等价于用户
///    拖动时间条的效果，保证播放键始终能开始/恢复播放。
pub(super) fn media_apply_video_playback(store: &mut AppStore, playing: bool) -> Vec<usize> {
    if store.state().video_sessions.is_empty() {
        media_set_video_playing(store, false);
        return Vec::new();
    }
    let targets = media_control_targets(store);
    for &index in &targets {
        let session = &mut store.state_mut().video_sessions[index];
        if playing {
            let at_end = media_video_session_at_display_end(session)
                || gst_session_at_eos(&session.pipeline);
            let target = if at_end {
                0.0
            } else {
                session.current_time.max(0.0)
            };
            // 必须先用正速率 seek 替换可能残留的负速率末帧 segment，再进入
            // Playing；反过来会让管线在两次调用之间短暂倒放。
            if at_end {
                gst_seek_pipeline_to_secs(&session.pipeline, target);
            } else {
                gst_seek_pipeline_accurate(&session.pipeline, target);
            }
            session.current_time = target;
            gst_set_session_state(session, gstreamer::State::Playing);
        } else {
            gst_set_session_state(session, gstreamer::State::Paused);
        }
    }
    media_set_video_playing(store, playing);
    targets
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

/// 当前焦点视频是否应以实际播放时间驱动动画时间轴。多视频的全局 transport 标量
/// 不能回答这个问题：其它视频可以仍在播放，而焦点视频已经暂停。拖拽期间管线会因
/// flush-seek 短暂进入过渡态，因此优先使用该焦点目标在拖拽开始时记录的播放意图。
pub(crate) fn media_active_video_drives_timeline(store: &AppStore) -> bool {
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
            gst_pipeline_wants_playing(&session.pipeline)
                && !gst_session_at_eos(&session.pipeline)
        })
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
pub(crate) fn media_refresh_video_playing_state(store: &mut AppStore) -> bool {
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
    session.current_time = secs;
    // 暂停态 seek 产生的 preroll 会由 sink 回调保存并请求一次重绘。
    // 正向 ACCURATE seek 的目标若晚于最后一帧 PTS，即使仍小于容器 duration，
    // 解码器也会按 segment 起点裁掉最后一帧并直接 EOS。暂停态越过结尾时改为
    // 从媒体尾边界反向精确 seek，首个 preroll 就是最后一个可解码视频帧。
    // 不读取瞬时管线状态决定是否进入该分支：播放中的 flush-seek 会短暂显示为
    // Paused，曾导致同一交互随机残留负速率 segment。越界行为固定为先暂停。
    if requested_past_end {
        if let Some(duration) = session.duration {
            gst_set_session_state(session, gstreamer::State::Paused);
            // set_state(Paused) 可能异步返回；若立即发反向 seek，切换前尚在途的
            // 正向播放帧会先覆盖画面槽，看起来像末帧没有刷新。越界点击是低频
            // 操作，允许有限等待管线稳定，再丢弃那张旧帧后建立末帧 segment。
            let _ = session
                .pipeline
                .state(Some(gstreamer::ClockTime::from_mseconds(500)));
            if let Ok(mut next_frame) = session.wgpu_sink.next_frame.lock() {
                next_frame.take();
            }
            if !gst_seek_pipeline_last_frame(&session.pipeline, duration) {
                gst_seek_pipeline_accurate(&session.pipeline, secs);
            }
            return;
        }
    }
    gst_seek_pipeline_accurate(&session.pipeline, secs);
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
    let Some(duration) = duration.filter(|duration| duration.is_finite() && *duration > 0.0)
    else {
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
pub(super) fn gst_seek_pipeline_accurate(pipeline: &gstreamer::Pipeline, secs: f64) {
    gst_seek_pipeline_with_flags(
        pipeline,
        secs,
        gstreamer::SeekFlags::FLUSH | gstreamer::SeekFlags::ACCURATE,
    );
}

/// 暂停态定位最后一帧：以负速率从媒体尾边界向前建立精确 segment。负速率从
/// stop 位置开始，appsink 收到的首个 preroll 因而是尾边界之前的最后一帧。
fn gst_seek_pipeline_last_frame(pipeline: &gstreamer::Pipeline, duration: f64) -> bool {
    let duration = gstreamer::ClockTime::from_nseconds(
        (duration.max(0.0) * 1_000_000_000.0).round() as u64,
    );
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
