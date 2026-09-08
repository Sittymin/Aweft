use crate::actions::media::{MediaAction, VideoAction};
use crate::depsgraph::Tag;
use crate::store::AppStore;
use crate::store::state::{VideoSeekSession, VideoSeekTarget};
use anyhow::{Context, Result};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, OnceLock};

static GSTREAMER_INIT_RESULT: OnceLock<Result<(), gstreamer::glib::Error>> = OnceLock::new();

fn gst_initialize_once() -> Result<(), gstreamer::glib::Error> {
    GSTREAMER_INIT_RESULT.get_or_init(gstreamer::init).clone()
}

/// 分发媒体域 Action：管线在此创建/跳转；帧推送与 GPU 连接属渲染层。
/// 普通状态变化返回 Video Tag，由 video projection 更新 UI；逐帧播放时钟经
/// tick tag 入图调度，帧图像仍由渲染层按实时例外直接增量同步。
impl AppStore {
    pub(crate) fn dispatch_media(&mut self, action: MediaAction) -> Result<Vec<Tag>> {
        match action {
            MediaAction::Video(VideoAction::Append { uri }) => {
                let index = media_append_video_session(self, &uri)?;
                let mut tags = vec![
                    Tag::VideoStructureChanged {
                        index,
                        inserted: true,
                    },
                    Tag::VideoTimeChanged {
                        indices: vec![index],
                        sync_timeline: true,
                    },
                ];
                if index == 0 {
                    tags.push(Tag::VideoMainChanged);
                }
                Ok(tags)
            }
            MediaAction::Video(VideoAction::Replace { index, uri }) => {
                media_replace_video_session(self, index, &uri)?;
                Ok(vec![
                    Tag::VideoStructureChanged {
                        index,
                        inserted: false,
                    },
                    Tag::VideoTimeChanged {
                        indices: vec![index],
                        sync_timeline: true,
                    },
                ])
            }
            MediaAction::Video(VideoAction::ToggleMute { index }) => {
                let Some(session) = self.state_mut().video_sessions.get_mut(index) else {
                    log::warn!("切换静音：下标越界 {index}");
                    return Ok(Vec::new());
                };
                session.muted = !session.muted;
                if let Some(audio) = session.audio.as_ref() {
                    audio.set_muted(session.muted);
                }
                Ok(vec![Tag::VideoMuteChanged { index }])
            }
            MediaAction::Video(VideoAction::SelectAudioTrack {
                video_index,
                track_index,
            }) => {
                media_select_audio_track(self, video_index, track_index)?;
                Ok(Vec::new())
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
                let playback_changed = media_refresh_video_playing_state(self);
                let mut tags = vec![
                    Tag::VideoSelectionChanged,
                    Tag::VideoTimeChanged {
                        indices: self.state().selected_videos.clone(),
                        sync_timeline: true,
                    },
                ];
                if playback_changed {
                    tags.push(Tag::VideoPlaybackChanged);
                }
                Ok(tags)
            }
            MediaAction::Video(VideoAction::SetMain { index }) => {
                if index >= self.state().video_sessions.len() {
                    log::warn!("设为主视频：下标越界 {index}");
                    return Ok(Vec::new());
                }
                let moved = media_set_main_video_session(self, index);
                let playback_changed = media_refresh_video_playing_state(self);
                let mut tags = vec![
                    Tag::VideoMainChanged,
                    Tag::VideoSelectionChanged,
                    Tag::VideoPositionChanged { indices: moved },
                    Tag::VideoTimeChanged {
                        indices: vec![index],
                        sync_timeline: true,
                    },
                ];
                if playback_changed {
                    tags.push(Tag::VideoPlaybackChanged);
                }
                Ok(tags)
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
                Ok(vec![Tag::VideoPositionChanged {
                    indices: vec![index],
                }])
            }
            MediaAction::Video(VideoAction::Seek { timestamp }) => {
                let requested_time = timestamp as f64 / 1000.0;
                let sync_timeline = media_should_active_video_drive_timeline(self);
                self.ui_models.animation.graph.current_time = requested_time;
                let indices = self.seek_drag_targets(requested_time);
                // 暂停态由编辑器时间轴保持用户请求值；播放态则由视频钳制后的实际
                // 时间立即接管，后续仍由逐帧 GStreamer 时钟持续驱动。
                let mut tags = vec![
                    Tag::VideoTimeChanged {
                        indices,
                        sync_timeline,
                    },
                    Tag::GraphScalarsChanged,
                ];
                // 越界钳制停靠无 EOS、逐帧渲染不会自动校正按钮；非拖拽的 seek
                // 停靠后在此立即刷新。拖拽中 media_refresh 沿用拖拽前意图态，按钮
                // 保持不动，释放时由 SeekEnd 决定"恢复播放 / 到末帧自动暂停"。
                if media_refresh_video_playing_state(self) {
                    tags.push(Tag::VideoPlaybackChanged);
                }
                Ok(tags)
            }
            MediaAction::Video(VideoAction::SeekVideo { index, timestamp }) => {
                if index >= self.state().video_sessions.len() {
                    log::warn!("指定视频 seek：下标越界 {index}");
                    return Ok(Vec::new());
                }
                let requested_time = timestamp as f64 / 1000.0;
                let is_active = self.state().active_video() == Some(index);
                let sync_timeline = is_active && media_should_active_video_drive_timeline(self);
                if is_active {
                    // 波形 Area 可查看任意音轨；只有它正在显示焦点视频的音轨时，
                    // 点击标尺才同时移动动画域的全局播放头。暂停态保留用户请求值，
                    // 播放态则由下方 VideoTimeChanged 投影回钳制后的实际视频位置。
                    self.ui_models.animation.graph.current_time = requested_time;
                }
                let session = &mut self.state_mut().video_sessions[index];
                media_seek_video_session(session, requested_time);
                // 音频分支与会话管线同源同钟：seek 已作用于整条管线，无需单独 seek。
                let mut tags = vec![Tag::VideoTimeChanged {
                    indices: vec![index],
                    // 非焦点音轨只刷新自身行和画面；焦点音轨播放中由实际位置
                    // 接管全局时间，暂停时则保留上方写入的原始请求值。
                    sync_timeline,
                }];
                if is_active {
                    tags.push(Tag::GraphScalarsChanged);
                }
                // 单击（共享标尺/波形标尺）把播放中的视频 seek 到范围外会被钳制并
                // 停靠为暂停；该路径无 EOS、逐帧渲染不会校正播放按钮，若不在此刷新
                // `video_playing` 并打 PlaybackChanged，按钮仍显示"播放中"，下一次
                // 按下会被 Toggle 当成"暂停"，表现为"停住后按播放再也无法续播"。
                if media_refresh_video_playing_state(self) {
                    tags.push(Tag::VideoPlaybackChanged);
                }
                Ok(tags)
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
                            was_playing: session.playing,
                        })
                    })
                    .collect();
                let anchor_time = self.state().video_sessions[index].current_time;
                self.state_mut().video_seek_session = Some(VideoSeekSession {
                    anchor_time,
                    targets,
                });
                // 拖拽期间对原本播放中的目标先静音：flush-seek 的停顿期间音频
                // 会发出切帧爆音/杂声，拖拽结束恢复播放时再还原用户静音设置。
                for target in self
                    .state()
                    .video_seek_session
                    .as_ref()
                    .into_iter()
                    .flat_map(|drag| &drag.targets)
                {
                    if target.was_playing
                        && let Some(session) = self.state().video_sessions.get(target.index)
                        && let Some(audio) = session.audio.as_ref()
                    {
                        audio.set_muted(true);
                    }
                }
                let playback_changed = media_refresh_video_playing_state(self);
                let mut tags = vec![
                    Tag::VideoSelectionChanged,
                    Tag::VideoTimeChanged {
                        indices: self.state().selected_videos.clone(),
                        sync_timeline: true,
                    },
                ];
                if playback_changed {
                    tags.push(Tag::VideoPlaybackChanged);
                }
                Ok(tags)
            }
            MediaAction::Video(VideoAction::SeekEnd) => {
                let Some(drag) = self.state_mut().video_seek_session.take() else {
                    return Ok(Vec::new());
                };
                let active = self.state().active_video();
                let sync_timeline = active.is_some_and(|active| {
                    drag.targets
                        .iter()
                        .any(|target| target.index == active && target.was_playing)
                });
                let indices = drag.targets.iter().map(|target| target.index).collect();
                for target in drag.targets {
                    let Some(session) = self.state_mut().video_sessions.get_mut(target.index) else {
                        continue;
                    };
                    if !target.was_playing {
                        continue;
                    }
                    let at_display_end = media_video_session_at_display_end(session);
                    let current_time = session.current_time;
                    let muted = session.muted;
                    if at_display_end {
                        // 播放中拖到范围外等价于播放到结尾：末帧反向 seek 已把
                        // 管线稳定留在 Paused，这里只恢复用户静音设置，不能再切回
                        // Playing，否则会沿负速率 segment 倒放。
                    } else {
                        // 先精确 seek 到拖拽点，再恢复视频播放。音频分支与视频同源
                        // 同钟，seek + Playing 已一并作用于它。
                        gst_seek_pipeline_accurate(&session.pipeline, current_time.max(0.0));
                        gst_set_video_session_state(session, gstreamer::State::Playing);
                    }
                    // 还原拖拽开始时的用户静音设置（拖拽期间由 SeekBegin 静音）。
                    if let Some(audio) = session.audio.as_ref() {
                        audio.set_muted(muted);
                    }
                }
                let playback_changed = media_refresh_video_playing_state(self);
                let mut tags = vec![Tag::VideoTimeChanged {
                    indices,
                    sync_timeline,
                }];
                if playback_changed {
                    tags.push(Tag::VideoPlaybackChanged);
                }
                Ok(tags)
            }
            MediaAction::Video(VideoAction::RefreshPlayingState) => {
                if media_refresh_video_playing_state(self) {
                    Ok(vec![Tag::VideoPlaybackChanged])
                } else {
                    Ok(vec![])
                }
            }
            MediaAction::Video(VideoAction::Play) => {
                let indices = media_apply_video_transport(self, true);
                Ok(vec![
                    Tag::VideoTimeChanged {
                        indices,
                        sync_timeline: true,
                    },
                    Tag::VideoPlaybackChanged,
                ])
            }
            MediaAction::Video(VideoAction::Pause) => {
                let indices = media_apply_video_transport(self, false);
                Ok(vec![
                    Tag::VideoTimeChanged {
                        indices,
                        sync_timeline: true,
                    },
                    Tag::VideoPlaybackChanged,
                ])
            }
            MediaAction::Video(VideoAction::TogglePlayback) => {
                let indices = media_apply_video_transport(self, !self.state().video_playing);
                Ok(vec![
                    Tag::VideoTimeChanged {
                        indices,
                        sync_timeline: true,
                    },
                    Tag::VideoPlaybackChanged,
                ])
            }
            // 帧由 appsink 回调和渲染通知驱动，Action 不需要重复拉取。
            MediaAction::Video(VideoAction::UpdateFrame) => Ok(Vec::new()),
            MediaAction::Subtitle(action) => {
                log::warn!("字幕 Action 尚未实现：{action:?}");
                Ok(Vec::new())
            }
        }
    }
}

mod playback;
mod waveform;

use playback::*;
pub(crate) use playback::{
    gst_session_playing_position, media_should_active_video_drive_timeline,
    media_refresh_video_playing_state,
};
use waveform::{discover_audio_tracks, start_waveform_extractions};

/// 切换会话的实时播放音轨。音轨在会话同一条管线内：input-selector 改写 active-pad
/// 后，对整条管线做一次精确 flush-seek 到当前时间，让新音轨与视频从同一时间点
/// 重新对齐（同一解码头/时钟下本来就能自动同步，flush-seek 补上的是新音轨
/// 启动时错过的 segment 状态，见 `gst_seek_pipeline_accurate` 的既有语义）。
fn media_select_audio_track(
    store: &mut AppStore,
    video_index: usize,
    track_index: usize,
) -> Result<()> {
    let Some(track) = store
        .state()
        .audio_tracks
        .get(video_index)
        .and_then(|tracks| tracks.get(track_index))
        .cloned()
    else {
        anyhow::bail!("选择音轨越界: video={video_index}, track={track_index}");
    };
    let Some(session) = store.state().video_sessions.get(video_index) else {
        anyhow::bail!("选择音轨的视频不存在: {video_index}");
    };
    let stream_id = track.stream_id.clone();
    let Some(audio) = session.audio.as_ref() else {
        anyhow::bail!("会话未装配音频分支，无法选轨: video={video_index}");
    };
    // parsebin 尚未暴露目标音轨时 select_stream 只登记 pending，pad 到达后自动生效；
    // 这里无法确知生效时机，仍按已生效处理（失败只记录日志，不打断视频会话）。
    let applied_now = audio.select_stream(&stream_id);

    store.state_mut().audio_track_indices[video_index] = Some(track_index);
    log::info!(
        "切换实时播放音轨: video={video_index}, track={track_index}, stream-id={stream_id}"
    );

    // 新音轨从当前时间对齐。媒体刚追加时管线尚在暂停预滚动，音轨 pad 可能尚未
    // 暴露；此时不做 flush-seek（首播由 transport 的播放路径负责），只对已建链
    // 的会话重新对齐。
    if applied_now {
        let session = &mut store.state_mut().video_sessions[video_index];
        media_seek_video_session(session, session.current_time);
        if let Some(audio) = session.audio.as_ref() {
            audio.set_muted(session.muted);
        }
    }
    Ok(())
}

/// 追加一个视频会话：搭建管线 + 连接 GPU + 自动播放，级联错开落点
/// （相对当前选中视频，无选中则相对主视频）。
fn media_append_video_session(store: &mut AppStore, video_uri: &str) -> Result<usize> {
    gst_initialize_once().context("GStreamer 初始化失败")?;
    let mut session = crate::render::video_sink::build_session(store, video_uri)?;
    let audio_tracks = discover_audio_tracks(video_uri).unwrap_or_else(|error| {
        log::warn!("发现媒体音轨失败 ({video_uri}): {error:#}");
        Vec::new()
    });
    let (x, y) = media_video_cascade_anchor(store);
    session.position_x = x;
    session.position_y = y;
    let is_first = store.state().video_sessions.is_empty();
    store.state_mut().video_sessions.push(session);
    store.state_mut().audio_tracks.push(audio_tracks);
    store.state_mut().audio_track_indices.push(None);
    if is_first {
        store.state_mut().main_video = Some(0);
    }
    let index = store.state().video_sessions.len() - 1;
    let cancellation = Arc::new(AtomicBool::new(false));
    store
        .state_mut()
        .waveform_cancellations
        .push(cancellation.clone());
    start_waveform_extractions(store, video_uri, index, cancellation);
    if !store.state().audio_tracks[index].is_empty()
        && let Err(error) = media_select_audio_track(store, index, 0)
    {
        log::warn!("初始化实时音轨失败: {error:#}");
    }
    Ok(index)
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
    let session_was_playing = old.playing;

    gst_initialize_once().context("GStreamer 初始化失败")?;
    let mut new_session = crate::render::video_sink::build_session(store, uri)?;
    let audio_tracks = discover_audio_tracks(uri).unwrap_or_else(|error| {
        log::warn!("发现媒体音轨失败 ({uri}): {error:#}");
        Vec::new()
    });
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
    crate::render::video_sink::stop_session_parts(&old.pipeline, old.wgpu_sink);
    if let Some(slot) = store.state_mut().audio_tracks.get_mut(index) {
        *slot = audio_tracks;
    }
    // 新会话自带的音频分支随会话置 Null 一起释放，无需单独处理。
    store.state_mut().audio_track_indices[index] = None;
    if let Some(old) = store.state().waveform_cancellations.get(index) {
        old.store(true, Ordering::Release);
    }
    let cancellation = Arc::new(AtomicBool::new(false));
    if let Some(slot) = store.state_mut().waveform_cancellations.get_mut(index) {
        *slot = cancellation.clone();
    }
    start_waveform_extractions(store, uri, index, cancellation);
    if !store.state().audio_tracks[index].is_empty()
        && let Err(error) = media_select_audio_track(store, index, 0)
    {
        log::warn!("替换媒体后初始化实时音轨失败: {error:#}");
    }
    {
        let session = &mut store.state_mut().video_sessions[index];
        // 即使旧会话原来是暂停的，也必须把新管线定位到保留/钳制后的位置；
        // 否则 UI 的 current_time 与新管线实际位置会分离（新管线仍在 0 秒）。
        gst_seek_pipeline_accurate(&session.pipeline, session.current_time);
        if session_was_playing {
            gst_set_video_session_state(session, gstreamer::State::Playing);
        }
    }
    Ok(())
}

/// 设置主视频：与旧主视频交换视口位置（主视频恒在原点，参考框/级联基准一致），
/// 并选中新主视频。位置用 set_row_data 局部更新，不触碰帧模型。
fn media_set_main_video_session(store: &mut AppStore, index: usize) -> Vec<usize> {
    let main = store.state().main_video;
    let mut moved = vec![index];
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
        moved.insert(0, main);
    }
    store.state_mut().main_video = Some(index);
    // 设为（并选中）主视频：交互对象顺带选中，视觉与参考框一致
    store.state_mut().selected_videos.clear();
    store.state_mut().selected_videos.push(index);
    moved
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

/// 当前 seek 目标视频：选中（无则主视频）。只修改会话与 GStreamer，返回受影响下标；
/// 时间条和动画播放头由 VideoTime projection 同步。
impl AppStore {
    pub(crate) fn seek_target(&mut self, secs: f64) -> Option<usize> {
        let Some(target) = self.state().active_video() else {
            return None;
        };
        let Some(session) = self.state_mut().video_sessions.get_mut(target) else {
            log::warn!("seek 目标视频不存在：{target}");
            return None;
        };
        media_seek_video_session(session, secs);
        Some(target)
    }

    /// 时间条拖拽目标：单拖时只有焦点，批量时按锚点增量移动全部选中视频。
    fn seek_drag_targets(&mut self, secs: f64) -> Vec<usize> {
        let Some(drag) = self.state().video_seek_session.clone() else {
            return self.seek_target(secs).into_iter().collect();
        };
        let delta = secs - drag.anchor_time;
        let mut indices = Vec::new();
        for target in drag.targets {
            let Some(session) = self.state_mut().video_sessions.get_mut(target.index) else {
                continue;
            };
            media_seek_video_session(session, target.origin_time + delta);
            indices.push(target.index);
        }
        indices
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

#[cfg(test)]
mod tests;
