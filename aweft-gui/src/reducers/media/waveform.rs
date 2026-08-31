use crate::store::AppStore;
use crate::store::state::AudioTrackInfo;
use gstreamer_pbutils::prelude::*;
use slint::ComponentHandle as _;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, OnceLock};

struct WaveformJob {
    window: slint::Weak<crate::bridge::types::MainWindow>,
    uri: String,
    video_index: usize,
    track_index: usize,
    stream_id: String,
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
        log::info!(
            "跳过已取消的波形任务: video={}, track={}, stream-id={}",
            job.video_index,
            job.track_index,
            job.stream_id,
        );
        return;
    }
    log::info!(
        "波形 worker 开始任务: video={}, track={}, stream-id={}, thread={:?}",
        job.video_index,
        job.track_index,
        job.stream_id,
        std::thread::current().name().unwrap_or("<unnamed>"),
    );
    let result = crate::render::video_sink::waveform::extract_cancelable(
        &job.uri,
        &job.stream_id,
        || job.cancellation.load(Ordering::Acquire),
        |start, duration, amplitudes| {
            if job.cancellation.load(Ordering::Acquire) {
                return;
            }
            let batch = amplitudes.to_vec();
            let cancellation = job.cancellation.clone();
            // 四秒级合批已将事件数压低到与媒体时长成线性的较小规模。这里不能
            // 同步等待 UI 回执：Area 一旦显示波形，模型通知成本会变化，若让解码
            // 线程等待可见组件完成求值，后台进度就会错误地依赖当前 Area 类型。
            let queued = job.window.upgrade_in_event_loop(move |instance| {
                if cancellation.load(Ordering::Acquire) {
                    return;
                }
                instance
                    .global::<crate::bridge::types::VideoCallbacks>()
                    .invoke_waveform_progress(
                        job.video_index as i32,
                        job.track_index as i32,
                        start as i32,
                        duration,
                        slint::ModelRc::from(std::rc::Rc::new(slint::VecModel::from(batch))),
                    );
            });
            if let Err(error) = queued {
                log::warn!("波形批次无法投递到 UI: {error}");
            }
        },
    );
    if job.cancellation.load(Ordering::Acquire) {
        log::info!(
            "波形 worker 任务已取消: video={}, track={}, stream-id={}",
            job.video_index,
            job.track_index,
            job.stream_id,
        );
        return;
    }
    match result {
        Ok((amplitudes, duration)) => log::info!(
            "音频波形提取完成: video={}, track={}, duration={duration:.3}s, buckets={}",
            job.video_index,
            job.track_index,
            amplitudes.len(),
        ),
        Err(error) => log::warn!(
            "音频波形提取失败 ({}, track={}, stream-id={}): {error:#}",
            job.uri,
            job.track_index,
            job.stream_id,
        ),
    }
}

/// 使用 Discoverer 读取容器内的真实音频流。没有 stream-id 的流无法稳定定向选择，
/// 因此跳过并记录警告，不用展示下标伪装成可选择的音轨。
pub(super) fn discover_audio_tracks(uri: &str) -> anyhow::Result<Vec<AudioTrackInfo>> {
    let discoverer = gstreamer_pbutils::Discoverer::new(gstreamer::ClockTime::from_seconds(10))?;
    let info = discoverer.discover_uri(uri)?;
    let mut tracks = Vec::new();
    for (index, audio) in info.audio_streams().into_iter().enumerate() {
        let Some(stream_id) = audio.stream_id().map(|id| id.to_string()) else {
            log::warn!("跳过没有 stream-id 的音轨: uri={uri}, track={index}");
            continue;
        };
        let language = audio
            .language()
            .map(|value| value.to_string())
            .unwrap_or_default();
        let tags = audio.tags();
        let title = tags.as_ref().and_then(|tags| {
            tags.get::<gstreamer::tags::Title>()
                .map(|value| value.get().to_owned())
        });
        let label = title.unwrap_or_else(|| format!("音轨 {}", index + 1));
        let caps = audio.caps();
        let codec = tags
            .as_ref()
            .and_then(|tags| {
                tags.get::<gstreamer::tags::AudioCodec>()
                    .map(|value| value.get().to_owned())
            })
            .or_else(|| {
                caps.as_ref().map(|caps| {
                    gstreamer_pbutils::pb_utils_get_codec_description(caps.as_ref()).to_string()
                })
            })
            .unwrap_or_default();
        let channels = audio.channels();
        let channel_mask = audio.channel_mask();
        let tag_text = |name: &str| {
            tags.as_ref()
                .and_then(|tags| tags.generic(name))
                .and_then(|value| value.get::<&str>().ok().map(str::to_owned))
                .unwrap_or_default()
        };
        tracks.push(AudioTrackInfo {
            stream_id,
            label,
            language,
            channels: channels as i32,
            channel_layout: audio_channel_layout(channels, channel_mask),
            channel_mask: (channel_mask != 0)
                .then(|| format!("0x{channel_mask:016X}"))
                .unwrap_or_default(),
            codec,
            caps: caps.map(|value| value.to_string()).unwrap_or_default(),
            sample_rate: audio.sample_rate() as i32,
            bit_depth: audio.depth() as i32,
            bitrate: audio.bitrate().min(i32::MAX as u32) as i32,
            max_bitrate: audio.max_bitrate().min(i32::MAX as u32) as i32,
            artist: tag_text(gstreamer::tags::Artist::TAG_NAME),
            album: tag_text(gstreamer::tags::Album::TAG_NAME),
            comment: tag_text(gstreamer::tags::Comment::TAG_NAME),
            description: tag_text(gstreamer::tags::Description::TAG_NAME),
            encoder: tag_text(gstreamer::tags::Encoder::TAG_NAME),
            encoded_by: tag_text(gstreamer::tags::EncodedBy::TAG_NAME),
        });
    }
    Ok(tracks)
}

fn audio_channel_layout(channels: u32, channel_mask: u64) -> String {
    match channels {
        0 => String::new(),
        1 => "单声道".to_owned(),
        2 => "立体声".to_owned(),
        6 if channel_mask & (1 << 3) != 0 => "5.1".to_owned(),
        8 if channel_mask & (1 << 3) != 0 => "7.1".to_owned(),
        count => format!("{count} 声道"),
    }
}

/// 将当前媒体的所有真实音轨送入单 worker。每条任务在解码前按 stream-id 选择
/// 容器内的压缩音轨；任务串行执行，避免多条离线管线同时争抢播放资源。
pub(super) fn start_waveform_extractions(
    store: &AppStore,
    uri: &str,
    video_index: usize,
    cancellation: Arc<AtomicBool>,
) {
    let Some(window) = store.main_window.clone() else {
        return;
    };
    let Some(tracks) = store.state().audio_tracks.get(video_index) else {
        return;
    };
    for (track_index, track) in tracks.iter().enumerate() {
        log::info!(
            "音轨波形任务入队: video={}, track={}, stream-id={}, uri={}",
            video_index,
            track_index,
            track.stream_id,
            uri,
        );
        if waveform_worker()
            .send(WaveformJob {
                window: window.clone(),
                uri: uri.to_owned(),
                video_index,
                track_index,
                stream_id: track.stream_id.clone(),
                cancellation: cancellation.clone(),
            })
            .is_err()
        {
            log::warn!("音频波形后台 worker 已退出");
            break;
        }
    }
}
