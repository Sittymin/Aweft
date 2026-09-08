//! 音频域投影：视频会话结构变更时同步 Slint `AudioState` 的音轨行模型。
//!
//! 音频模型与视频时间条模型由同一个 `VideoStructureChanged` tag 驱动（depsgraph
//! 拓扑不变）：`projections/video` 的结构操作先调本模块完成音频行的追加/重置，
//! 再刷新时间条行。帧图像与波形后台批次属于高频异步流，不走本投影，只允许行级
//! 更新（见 `docs/architecture/video-playback.md` 中的实时例外）。

use std::rc::Rc;

use slint::{Model as _, VecModel};

use crate::bridge::types::{SlintAudioTrack, SlintAudioWaveform};
use crate::depsgraph::Tag;
use crate::store::AppStore;

/// 波形视图上报：某 Area 的可视窗口变化，同步其持久列模型（数量局部增删、
/// 行值原地更新，模型标识恒定，Slint 侧不重建槽位）。
pub fn ui_project_video_waveform_view(store: &mut AppStore, tags: &[Tag]) {
    use crate::store::audio::AreaWaveformView;
    for tag in tags {
        let Tag::VideoWaveformViewChanged {
            area,
            video_index,
            track_index,
            bucket_ms,
            first,
            count,
        } = tag
        else {
            continue;
        };
        let view = AreaWaveformView {
            video_index: *video_index,
            track_index: *track_index,
            bucket_ms: *bucket_ms,
            first: *first,
            count: *count,
        };
        crate::store::audio::ui_sync_waveform_columns(&mut store.ui_models.audio, *area, view);
    }
}

/// 会话结构变化：追加时只 push 新行；替换时保留桶模型槽位并重建对应音轨行。
pub fn ui_project_audio_structure(store: &mut AppStore, tags: &[Tag]) {
    for tag in tags {
        let Tag::VideoStructureChanged { index, inserted } = tag else {
            continue;
        };
        if *inserted {
            ui_append_audio_models(store, *index);
        } else {
            ui_reset_audio_waveform(store, *index);
        }
    }
}

/// 播放音轨变化：更新对应视频的播放音轨下标。
pub fn ui_project_video_audio_track(store: &mut AppStore, tags: &[Tag]) {
    for tag in tags {
        let Tag::VideoAudioTrackChanged {
            video_index,
            track_index,
        } = tag
        else {
            continue;
        };
        let audio = &mut store.ui_models.audio;
        if *video_index < audio.playback_tracks.row_count() {
            audio
                .playback_tracks
                .set_row_data(*video_index, *track_index as i32);
        }
    }
}

fn ui_append_audio_models(store: &mut AppStore, index: usize) {
    let tracks = store
        .state()
        .audio_tracks
        .get(index)
        .cloned()
        .unwrap_or_default();
    let playback_track = store
        .state()
        .audio_track_indices
        .get(index)
        .copied()
        .flatten()
        .map(|idx| idx as i32)
        .unwrap_or(-1);
    let audio = &mut store.ui_models.audio;
    if audio.tracks.row_count() != index {
        log::error!(
            "音频投影结构不同步：期望追加下标 {index}，现有 {} 行",
            audio.tracks.row_count()
        );
        return;
    }
    let (track_rows, buckets) = ui_build_audio_track_models(&tracks);
    let model = Rc::new(VecModel::from(track_rows));
    audio.track_models.push(Rc::clone(&model));
    audio.tracks.push(slint::ModelRc::from(model));
    audio.playback_tracks.push(playback_track);
    audio.buckets.push(buckets);
}

fn ui_reset_audio_waveform(store: &mut AppStore, index: usize) {
    let tracks = store
        .state()
        .audio_tracks
        .get(index)
        .cloned()
        .unwrap_or_default();
    let playback_track = store
        .state()
        .audio_track_indices
        .get(index)
        .copied()
        .flatten()
        .map(|idx| idx as i32)
        .unwrap_or(-1);
    let (track_rows, buckets) = ui_build_audio_track_models(&tracks);
    let audio = &mut store.ui_models.audio;
    let model = Rc::new(VecModel::from(track_rows));
    if let Some(slot) = audio.track_models.get_mut(index) {
        *slot = Rc::clone(&model);
    }
    if index < audio.tracks.row_count() {
        audio.tracks.set_row_data(index, slint::ModelRc::from(model));
    }
    if index < audio.playback_tracks.row_count() {
        audio.playback_tracks.set_row_data(index, playback_track);
    }
    if let Some(slot) = audio.buckets.get_mut(index) {
        *slot = buckets;
    }
}

fn ui_build_audio_track_models(
    tracks: &[crate::store::state::AudioTrackInfo],
) -> (
    Vec<SlintAudioTrack>,
    Vec<Rc<VecModel<crate::bridge::types::SlintWaveformBucket>>>,
) {
    let mut rows = Vec::with_capacity(tracks.len());
    let mut bases = Vec::with_capacity(tracks.len());
    for track in tracks {
        let base = Rc::new(VecModel::default());
        rows.push(SlintAudioTrack {
            stream_id: track.stream_id.clone().into(),
            label: track.label.clone().into(),
            language: track.language.clone().into(),
            channels: track.channels,
            channel_layout: track.channel_layout.clone().into(),
            channel_mask: track.channel_mask.clone().into(),
            codec: track.codec.clone().into(),
            caps: track.caps.clone().into(),
            sample_rate: track.sample_rate,
            bit_depth: track.bit_depth,
            bitrate: track.bitrate,
            max_bitrate: track.max_bitrate,
            artist: track.artist.clone().into(),
            album: track.album.clone().into(),
            comment: track.comment.clone().into(),
            description: track.description.clone().into(),
            encoder: track.encoder.clone().into(),
            encoded_by: track.encoded_by.clone().into(),
            waveform: SlintAudioWaveform { duration: 0.0 },
            waveform_status: crate::bridge::types::AudioTrackWaveformStatus::Pending,
        });
        bases.push(base);
    }
    (rows, bases)
}
