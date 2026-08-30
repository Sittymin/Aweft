//! 视频域投影：普通媒体 Action 的状态变更经 Video Tag 增量同步到 Slint。
//!
//! 帧图像、逐帧播放时钟和波形后台批次属于高频异步流，不走本投影；这些实时
//! 例外只允许行级更新，并在 `docs/architecture/video-playback.md` 中单独说明。

use std::rc::Rc;

use slint::{Model as _, VecModel};

use crate::bridge::types::{
    SlintAudioWaveform, SlintAudioWaveformLevel, SlintVideoPos, SlintVideoTimeBar,
};
use crate::depsgraph::Tag;
use crate::store::AppStore;
use crate::store::utils::rainbow::rainbow_color;

/// 会话结构变化：追加时只 push 新行；替换时保留帧/尺寸行并重置对应波形。
pub fn ui_project_video_structure(store: &mut AppStore, tags: &[Tag]) {
    for tag in tags {
        let Tag::VideoStructureChanged { index, inserted } = tag else {
            continue;
        };
        if *inserted {
            ui_append_video_models(store, *index);
        } else {
            ui_reset_video_waveform(store, *index);
        }
    }
}

/// 选择或主视频变化：刷新时间条的选择/主视频列，并同步 Slint 标量。
pub fn ui_project_video_selection(store: &mut AppStore, _tags: &[Tag]) {
    ui_sync_video_time_bars(store);
    ui_sync_video_scalars(store);
}

/// 视口位置变化：仅更新 Tag 指定的行。
pub fn ui_project_video_position(store: &mut AppStore, tags: &[Tag]) {
    for tag in tags {
        let Tag::VideoPositionChanged { indices } = tag else {
            continue;
        };
        for &index in indices {
            ui_update_video_position(store, index);
        }
    }
}

/// 静音变化：仅刷新对应时间条行。
pub fn ui_project_video_mute(store: &mut AppStore, tags: &[Tag]) {
    for tag in tags {
        let Tag::VideoMuteChanged { index } = tag else {
            continue;
        };
        ui_update_video_time_bar(store, *index);
    }
}

/// seek/活跃视频变化：局部刷新时间条，并把活跃视频时间写入动画编辑数据层。
/// GraphScalars operation 显式依赖本 operation，负责随后同步 Slint 动画标量。
pub fn ui_project_video_time(store: &mut AppStore, tags: &[Tag]) {
    for tag in tags {
        let Tag::VideoTimeChanged { indices } = tag else {
            continue;
        };
        for &index in indices {
            ui_update_video_time_bar(store, index);
        }
    }
    ui_project_active_video_time(store);
}

/// transport 状态变化：只同步 Slint 播放标量。
pub fn ui_project_video_playback(store: &mut AppStore, _tags: &[Tag]) {
    ui_sync_video_playback_scalar(store);
}

/// 将权威播放状态同步到 Slint 标量。渲染时钟校正播放状态时也复用此实时出口。
pub(crate) fn ui_sync_video_playback_scalar(store: &AppStore) {
    let Some(weak) = &store.video_state else {
        return;
    };
    let Some(state) = weak.upgrade() else {
        return;
    };
    state.set_playing(store.state().video_playing);
}

/// 实时播放时钟共用的行级出口。该函数不参与普通 Action 的 reducer 调用。
pub(crate) fn ui_update_video_time_bar_value(
    bars: &VecModel<SlintVideoTimeBar>,
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

/// 实时播放时钟例外：把活跃视频时间投影到动画编辑数据层并同步 Slint 标量。
/// 普通 seek 由 VideoTime Tag 触发同一逻辑；渲染回调按帧调用本出口，避免
/// 每帧创建 Action/Tag。
pub(crate) fn ui_sync_active_video_time_realtime(store: &mut AppStore) {
    let Some(active) = store.state().active_video() else {
        return;
    };
    let Some(secs) = store
        .state()
        .video_sessions
        .get(active)
        .map(|session| session.current_time)
    else {
        return;
    };
    if (store.ui_models.animation.graph.current_time - secs).abs() < 0.005 {
        return;
    }
    store.ui_models.animation.graph.current_time = secs;
    store.sync_animation_scalars();
}

fn ui_append_video_models(store: &mut AppStore, index: usize) {
    let Some(session) = store.state().video_sessions.get(index) else {
        log::warn!("视频投影追加失败：会话下标越界 {index}");
        return;
    };
    let position = SlintVideoPos {
        x: session.position_x as f32,
        y: session.position_y as f32,
    };
    let ui = &mut store.ui_models.video;
    if ui.frames.row_count() != index {
        log::error!("视频投影结构不同步：期望追加下标 {index}，现有 {} 行", ui.frames.row_count());
        return;
    }
    ui.frames.push(slint::Image::default());
    ui.widths.push(0);
    ui.heights.push(0);
    ui.positions.push(position);
    ui.time_bars.push(SlintVideoTimeBar::default());

    let amplitudes = Rc::new(VecModel::default());
    let level_amplitudes = (1..=12)
        .map(|_| Rc::new(VecModel::default()))
        .collect::<Vec<_>>();
    let levels = level_amplitudes
        .iter()
        .enumerate()
        .map(|(level, values)| SlintAudioWaveformLevel {
            bucket_duration: 0.001 * (1usize << (level + 1)) as f32,
            amplitudes: slint::ModelRc::from(values.clone()),
        })
        .collect::<Vec<_>>();
    ui.audio_waveforms.push(SlintAudioWaveform {
        duration: 0.0,
        amplitudes: slint::ModelRc::from(amplitudes.clone()),
        levels: slint::ModelRc::from(Rc::new(VecModel::from(levels))),
    });
    ui.waveform_amplitudes.push(amplitudes);
    ui.waveform_level_amplitudes.push(level_amplitudes);
}

fn ui_reset_video_waveform(store: &mut AppStore, index: usize) {
    let ui = &mut store.ui_models.video;
    if let Some(values) = ui.waveform_amplitudes.get(index) {
        values.clear();
    }
    if let Some(levels) = ui.waveform_level_amplitudes.get(index) {
        for level in levels {
            level.clear();
        }
    }
    let Some(mut waveform) = ui.audio_waveforms.row_data(index) else {
        return;
    };
    waveform.duration = 0.0;
    ui.audio_waveforms.set_row_data(index, waveform);
}

fn ui_update_video_position(store: &mut AppStore, index: usize) {
    let Some(session) = store.state().video_sessions.get(index) else {
        return;
    };
    store.ui_models.video.positions.set_row_data(
        index,
        SlintVideoPos {
            x: session.position_x as f32,
            y: session.position_y as f32,
        },
    );
}

fn ui_update_video_time_bar(store: &mut AppStore, index: usize) {
    let Some(session) = store.state().video_sessions.get(index) else {
        return;
    };
    let old = store
        .ui_models
        .video
        .time_bars
        .row_data(index)
        .unwrap_or_default();
    let bar = SlintVideoTimeBar {
        label: ui_video_label(&session.uri),
        time: session.current_time as f32,
        color: rainbow_color(index),
        selected: store.state().selected_videos.contains(&index),
        muted: session.muted,
        is_main: store.state().main_video == Some(index),
    };
    if old != bar {
        store.ui_models.video.time_bars.set_row_data(index, bar);
    }
}

fn ui_sync_video_time_bars(store: &mut AppStore) {
    for index in 0..store.state().video_sessions.len() {
        ui_update_video_time_bar(store, index);
    }
}

fn ui_sync_video_scalars(store: &AppStore) {
    let Some(weak) = &store.video_state else {
        return;
    };
    let Some(state) = weak.upgrade() else {
        return;
    };
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

fn ui_project_active_video_time(store: &mut AppStore) {
    let Some(active) = store.state().active_video() else {
        return;
    };
    let Some(secs) = store
        .state()
        .video_sessions
        .get(active)
        .map(|session| session.current_time)
    else {
        return;
    };
    store.ui_models.animation.graph.current_time = secs;
}

fn ui_video_label(uri: &str) -> slint::SharedString {
    url::Url::parse(uri)
        .ok()
        .and_then(|url| url.to_file_path().ok())
        .and_then(|path| path.file_name().map(|name| name.to_string_lossy().into_owned()))
        .unwrap_or_else(|| uri.to_owned())
        .into()
}
