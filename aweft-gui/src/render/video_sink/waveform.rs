//! 离线音频波形提取：独立解码管线，绝不消费或干扰播放会话。

use gstreamer::prelude::*;
use gstreamer_app::AppSink;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};

/// 增量解析时每个 min/max/rms 桶覆盖的固定时间宽度（1ms）。桶按时间追加，因此已经通过的
/// 时间段不会因媒体总时长稍后才可用而重新编号或移动。
pub const WAVEFORM_BUCKET_DURATION_SECS: f64 = 0.001;
/// 1ms 波形桶使用 8kHz 单声道分析流，每桶聚合 8 个样本。相比直接扫描原始
/// 多声道/高采样率流，仍能减少 audioconvert/resample 和 CPU 扫描开销，同时保留
/// 毫秒级瞬态。
const WAVEFORM_ANALYSIS_RATE: i32 = 8_000;
/// 累积约四秒的 1ms 波形桶后再通知 UI。解码仍持续进行，只降低跨线程闭包、
/// VecModel 通知和波形金字塔更新的频率。
const WAVEFORM_PROGRESS_BATCH_BUCKETS: usize = 4_096;

/// 单个 1ms 时间桶的 min/max/rms 包络（Audacity 式三值）。
/// 空桶/静音段为全 0，显示为中心零线；只含负样本的桶 `max` 保持 0.0，
/// 只含正样本的桶 `min` 保持 0.0。
/// `min` 只减小、`max` 只增大（单调极值）；`rms` 是桶内全部样本的均方根，
/// 非单调——金字塔用 `rms²×名义跨度` 加权聚合（见 `update_waveform_pyramid`）。
/// `sumsq`/`count` 是 rms 的累积状态，不跨模块边界（Slint 侧只有 min/max/rms）。
#[derive(Clone, Copy, Default, Debug)]
pub struct WaveformBucket {
    pub min: f32,
    pub max: f32,
    pub rms: f32,
    sumsq: f64,
    count: u32,
}

/// 在调用方提供的线程中解码指定音轨并压缩成 min/max 桶。显式低层管线在解码前按
/// stream-id 选择 elementary stream；decodebin 后的 raw-audio 链是未来滤镜插入点。
/// `progress(start, duration, values)` 只提交从 `start` 开始的变更区间；调用方
/// 可以原地更新持久模型。`cancelled` 在解码循环内检查，替换媒体/退出时及时停工。
pub fn extract_cancelable<C, F>(
    uri: &str,
    target_stream_id: &str,
    cancelled: C,
    mut progress: F,
) -> anyhow::Result<(Vec<WaveformBucket>, f32)>
where
    C: Fn() -> bool,
    F: FnMut(usize, f32, &[WaveformBucket]),
{
    gstreamer::init().map_err(|error| anyhow::anyhow!("GStreamer 初始化失败: {error}"))?;
    let pipeline = gstreamer::Pipeline::new();
    let source = gstreamer::ElementFactory::make("urisourcebin")
        .property("uri", uri)
        // Keep URI reading separate from parsing. The explicit parsebin below
        // exposes the same elementary-stream IDs as Discoverer, which lets us
        // select one audio stream without handing the container to decodebin.
        .property("parse-streams", false)
        .property("use-buffering", false)
        .build()?;
    let parser = gstreamer::ElementFactory::make("parsebin").build()?;
    let decoder = gstreamer::ElementFactory::make("decodebin")
        .property("expose-all-streams", false)
        .build()?;
    let queue = gstreamer::ElementFactory::make("queue").build()?;
    let convert = gstreamer::ElementFactory::make("audioconvert").build()?;
    let resample = gstreamer::ElementFactory::make("audioresample").build()?;
    let caps = gstreamer::Caps::builder("audio/x-raw")
        .field("format", "F32LE")
        .field("rate", WAVEFORM_ANALYSIS_RATE)
        .field("channels", 1i32)
        .build();
    let filter = gstreamer::ElementFactory::make("capsfilter")
        .property("caps", caps)
        .build()?;
    let sink = gstreamer::ElementFactory::make("appsink")
        .property("sync", false)
        // 离线分析不应继承 GstBaseSink 默认的 20ms processing deadline。
        .property("processing-deadline", 0u64)
        .property("max-buffers", 64u32)
        .build()?;
    let appsink = sink
        .clone()
        .dynamic_cast::<AppSink>()
        .map_err(|_| anyhow::anyhow!("创建的 appsink 元素类型异常"))?;

    pipeline.add_many([
        &source, &parser, &decoder, &queue, &convert, &resample, &filter, &sink,
    ])?;
    gstreamer::Element::link_many([&queue, &convert, &resample, &filter, &sink])?;

    let raw_entry = queue.clone();
    decoder.connect_pad_added(move |_decoder, pad| {
        let is_audio = pad
            .current_caps()
            .or_else(|| Some(pad.query_caps(None)))
            .and_then(|caps| caps.structure(0).map(|s| s.name().starts_with("audio/")))
            .unwrap_or(false);
        if !is_audio {
            return;
        }
        if let Some(sink_pad) = raw_entry.static_pad("sink")
            && !sink_pad.is_linked()
        {
            let _ = pad.link(&sink_pad);
        }
    });

    let target_linked = Arc::new(AtomicBool::new(false));
    let parser_exposed_all_pads = Arc::new(AtomicBool::new(false));
    let parser_exposed_all_pads_for_signal = parser_exposed_all_pads.clone();
    parser.connect_no_more_pads(move |_| {
        parser_exposed_all_pads_for_signal.store(true, Ordering::Release);
    });
    let target_linked_for_pad = target_linked.clone();
    let target_id = target_stream_id.to_owned();
    let decoder_entry = decoder.clone();
    let dynamic_pipeline = pipeline.clone();
    let parser_sink = parser
        .static_pad("sink")
        .ok_or_else(|| anyhow::anyhow!("parsebin 缺少 sink pad"))?;
    source.connect_pad_added(move |_source, pad| {
        if !parser_sink.is_linked() {
            if let Err(error) = pad.link(&parser_sink) {
                log::warn!("URI source 连接 parsebin 失败: {error}");
            }
        }
    });

    parser.connect_pad_added(move |_parser, pad| {
        let stream_id = pad.stream_id().map(|id| id.to_string()).unwrap_or_default();
        if stream_id == target_id && !target_linked_for_pad.swap(true, Ordering::AcqRel) {
            if let Some(sink_pad) = decoder_entry.static_pad("sink") {
                if let Err(error) = pad.link(&sink_pad) {
                    log::warn!("目标音轨连接 decodebin 失败 ({stream_id}): {error}");
                    target_linked_for_pad.store(false, Ordering::Release);
                }
            }
            return;
        }

        // 其它 elementary streams 必须被消费，容器才能持续解复用到 EOS；直接接
        // fakesink 不会为视频、字幕或其它音轨启动解码器。
        let Ok(discard) = gstreamer::ElementFactory::make("fakesink")
            .property("sync", false)
            .property("async", false)
            .build()
        else {
            return;
        };
        if dynamic_pipeline.add(&discard).is_err() || discard.sync_state_with_parent().is_err() {
            return;
        }
        if let Some(sink_pad) = discard.static_pad("sink") {
            let _ = pad.link(&sink_pad);
        }
    });

    let bus = pipeline
        .bus()
        .ok_or_else(|| anyhow::anyhow!("无法获取音频分析 Bus"))?;
    // Do not use the media duration for indexing. It is frequently unavailable until
    // decoding has started, and changing it would move already displayed waveform bars.
    let mut buckets = Vec::<WaveformBucket>::new();
    // 仅记录自上次 progress 以来最早变化的桶；使用 usize::MAX 表示当前没有
    // 待提交区间，避免每批都从 0 重新提交整段历史波形。
    let mut dirty_start = usize::MAX;
    pipeline.set_state(gstreamer::State::Playing)?;
    loop {
        if cancelled() {
            let _ = pipeline.set_state(gstreamer::State::Null);
            return Ok((buckets, 0.0));
        }
        // 只在 appsink 上等待“数据或取消”最多 20ms；数据一到就立即处理。
        // 这不是等待 UI：progress 回调只通过 upgrade_in_event_loop 入队，
        // 解码线程不会等待主线程执行该闭包。短超时仅用于及时响应取消。
        let Some(sample) = appsink.try_pull_sample(gstreamer::ClockTime::from_mseconds(20)) else {
            if parser_exposed_all_pads.load(Ordering::Acquire)
                && !target_linked.load(Ordering::Acquire)
            {
                let _ = pipeline.set_state(gstreamer::State::Null);
                anyhow::bail!("容器未暴露目标音轨 stream-id: {target_stream_id}");
            }
            if appsink.is_eos() {
                break;
            }
            use gstreamer::MessageView;
            let mut reached_eos = false;
            while let Some(message) = bus.timed_pop(gstreamer::ClockTime::ZERO) {
                match message.view() {
                    MessageView::Eos(..) => reached_eos = true,
                    MessageView::Error(error) => {
                        let _ = pipeline.set_state(gstreamer::State::Null);
                        anyhow::bail!("音频解码失败: {}", error.error());
                    }
                    _ => {}
                }
            }
            if reached_eos {
                break;
            }
            continue;
        };
        if let Some(changed) = accumulate_sample(&sample, &mut buckets) {
            dirty_start = dirty_start.min(changed);
        }
        if dirty_start < buckets.len()
            && buckets.len() - dirty_start >= WAVEFORM_PROGRESS_BATCH_BUCKETS
        {
            let duration = waveform_duration(buckets.len());
            progress(dirty_start, duration, &buckets[dirty_start..]);
            dirty_start = buckets.len();
        }
    }
    let _ = pipeline.set_state(gstreamer::State::Null);
    if !target_linked.load(Ordering::Acquire) {
        anyhow::bail!("容器未暴露目标音轨 stream-id: {target_stream_id}");
    }

    let duration = waveform_duration(buckets.len());
    if dirty_start < buckets.len() {
        progress(dirty_start, duration, &buckets[dirty_start..]);
    }
    Ok((buckets, duration as f32))
}

/// 构建独立的实时音频播放管线。它与视口视频管线完全分离：parsebin 暴露容器内
/// elementary streams，只有目标 stream-id 接入 decodebin；其它流接入异步 fakesink，
/// 因而不会被解码。返回的 volume 是统一的静音/增益入口，后续可在其前后插入滤镜。
pub fn build_audio_playback(
    uri: &str,
    target_stream_id: &str,
) -> anyhow::Result<(gstreamer::Pipeline, gstreamer::Element)> {
    let pipeline = gstreamer::Pipeline::new();
    let source = gstreamer::ElementFactory::make("urisourcebin")
        .property("uri", uri)
        .property("parse-streams", false)
        .property("use-buffering", false)
        .build()?;
    let parser = gstreamer::ElementFactory::make("parsebin").build()?;
    let decoder = gstreamer::ElementFactory::make("decodebin")
        .property("caps", gstreamer::Caps::builder("audio/x-raw").build())
        .property("expose-all-streams", false)
        .build()?;
    let queue = gstreamer::ElementFactory::make("queue").build()?;
    let convert = gstreamer::ElementFactory::make("audioconvert").build()?;
    let volume = gstreamer::ElementFactory::make("volume").build()?;
    let sink = gstreamer::ElementFactory::make("autoaudiosink").build()?;
    pipeline.add_many([&source, &parser, &decoder, &queue, &convert, &volume, &sink])?;
    gstreamer::Element::link_many([&queue, &convert, &volume, &sink])?;

    let parser_sink = parser
        .static_pad("sink")
        .ok_or_else(|| anyhow::anyhow!("parsebin 缺少 sink pad"))?;
    source.connect_pad_added(move |_source, pad| {
        if !parser_sink.is_linked() {
            let _ = pad.link(&parser_sink);
        }
    });

    let target_id = target_stream_id.to_owned();
    let decoder_entry = decoder.clone();
    let dynamic_pipeline = pipeline.clone();
    parser.connect_pad_added(move |_parser, pad| {
        let stream_id = pad.stream_id().map(|id| id.to_string()).unwrap_or_default();
        if stream_id == target_id {
            if let Some(sink_pad) = decoder_entry.static_pad("sink")
                && !sink_pad.is_linked()
            {
                let _ = pad.link(&sink_pad);
            }
            return;
        }
        let Ok(discard) = gstreamer::ElementFactory::make("fakesink")
            .property("sync", false)
            .property("async", false)
            .build()
        else {
            return;
        };
        if dynamic_pipeline.add(&discard).is_ok()
            && discard.sync_state_with_parent().is_ok()
            && let Some(sink_pad) = discard.static_pad("sink")
        {
            let _ = pad.link(&sink_pad);
        }
    });

    let raw_entry = queue.clone();
    decoder.connect_pad_added(move |_decoder, pad| {
        let is_audio = pad
            .current_caps()
            .or_else(|| Some(pad.query_caps(None)))
            .and_then(|caps| caps.structure(0).map(|s| s.name().starts_with("audio/")))
            .unwrap_or(false);
        if is_audio
            && let Some(sink_pad) = raw_entry.static_pad("sink")
            && !sink_pad.is_linked()
        {
            let _ = pad.link(&sink_pad);
        }
    });
    pipeline.set_state(gstreamer::State::Paused)?;
    Ok((pipeline, volume))
}

#[cfg(test)]
pub fn extract<F>(uri: &str, mut progress: F) -> anyhow::Result<(Vec<WaveformBucket>, f32)>
where
    F: FnMut(f32, &[WaveformBucket]),
{
    let mut all = Vec::new();
    extract_cancelable(
        uri,
        &first_audio_stream_id(uri)?,
        || false,
        |start, duration, batch| {
            if all.len() < start + batch.len() {
                all.resize(start + batch.len(), WaveformBucket::default());
            }
            all[start..start + batch.len()].copy_from_slice(batch);
            progress(duration, &all);
        },
    )
}

#[cfg(test)]
fn first_audio_stream_id(uri: &str) -> anyhow::Result<String> {
    audio_stream_ids(uri)?
        .into_iter()
        .next()
        .ok_or_else(|| anyhow::anyhow!("测试媒体没有带 stream-id 的音轨"))
}

#[cfg(test)]
fn audio_stream_ids(uri: &str) -> anyhow::Result<Vec<String>> {
    use gstreamer_pbutils::prelude::*;
    let discoverer = gstreamer_pbutils::Discoverer::new(gstreamer::ClockTime::from_seconds(5))?;
    Ok(discoverer
        .discover_uri(uri)?
        .audio_streams()
        .into_iter()
        .filter_map(|stream| stream.stream_id().map(|id| id.to_string()))
        .collect())
}

fn accumulate_sample(
    sample: &gstreamer::Sample,
    buckets: &mut Vec<WaveformBucket>,
) -> Option<usize> {
    let buffer = sample.buffer()?;
    let map = buffer.map_readable().ok()?;
    let timestamp = buffer
        .pts()
        .map(|value| value.nseconds() as f64 / 1_000_000_000.0)
        .unwrap_or(0.0);
    let samples_per_bucket = (WAVEFORM_ANALYSIS_RATE as usize / 1_000).max(1);
    let first_index = (timestamp.max(0.0) / WAVEFORM_BUCKET_DURATION_SECS).floor() as usize;
    let sample_count = map.as_slice().len() / 4;
    if sample_count == 0 {
        return None;
    }
    let last_index = first_index + (sample_count - 1) / samples_per_bucket;
    let previous_len = buckets.len();
    let extended = previous_len <= last_index;
    if extended {
        buckets.resize(last_index + 1, WaveformBucket::default());
    }

    // 若 PTS 跳过了尚未生成的时间段，必须从旧长度开始提交，才能让 UI
    // 逐行 push 中间的零桶；连续追加时该值正好等于 first_index。
    // 每个落入桶的样本都标记变更：rms 几乎每次都会变化（非单调），
    // 即使 min/max 未突破也必须上报，否则 UI 内层 rms 线条会滞后。
    let mut changed_start = extended.then_some(previous_len.min(first_index));
    for (sample_index, bytes) in map.as_slice().chunks_exact(4).enumerate() {
        let bucket = first_index + sample_index / samples_per_bucket;
        let value =
            f32::from_le_bytes(bytes.try_into().expect("4-byte PCM sample")).clamp(-1.0, 1.0);
        let entry = &mut buckets[bucket];
        if value < entry.min {
            entry.min = value;
        } else if value > entry.max {
            entry.max = value;
        }
        entry.sumsq += (value as f64) * (value as f64);
        entry.count += 1;
        entry.rms = (entry.sumsq / entry.count as f64).sqrt() as f32;
        changed_start = Some(changed_start.map_or(bucket, |start| start.min(bucket)));
    }
    changed_start
}

fn waveform_duration(bucket_count: usize) -> f32 {
    (bucket_count as f64 * WAVEFORM_BUCKET_DURATION_SECS).max(0.001) as f32
}

#[cfg(test)]
mod tests {
    use super::*;
    use slint::{ComponentHandle, Model};
    use std::path::{Path, PathBuf};
    use std::sync::{Mutex, Once, OnceLock};
    use std::time::{Duration, Instant};

    fn gst_init() {
        static INIT: Once = Once::new();
        INIT.call_once(|| gstreamer::init().expect("GStreamer 初始化失败"));
    }

    fn serial_lock() -> std::sync::MutexGuard<'static, ()> {
        static LOCK: Mutex<()> = Mutex::new(());
        LOCK.lock().unwrap_or_else(|error| error.into_inner())
    }

    fn generated_test_audio() -> Option<&'static Path> {
        static CACHE: OnceLock<Option<PathBuf>> = OnceLock::new();
        CACHE
            .get_or_init(|| {
                gst_init();
                let path = std::env::temp_dir().join("aweft_generated_waveform.wav");
                if path.is_file() {
                    return Some(path);
                }
                let pipeline = gstreamer::Pipeline::new();
                let source = gstreamer::ElementFactory::make("audiotestsrc")
                    .property("freq", 440.0f64)
                    // 64 x 1024 samples at 8kHz yields over eight seconds, so the
                    // test catches a provisional duration that incorrectly freezes
                    // at the first second of decoded audio.
                    .property("num-buffers", 64i32)
                    .build()
                    .ok()?;
                let caps = gstreamer::ElementFactory::make("capsfilter")
                    .property(
                        "caps",
                        gstreamer::Caps::builder("audio/x-raw")
                            .field("rate", 8_000i32)
                            .field("channels", 1i32)
                            .build(),
                    )
                    .build()
                    .ok()?;
                let convert = gstreamer::ElementFactory::make("audioconvert")
                    .build()
                    .ok()?;
                let wav = gstreamer::ElementFactory::make("wavenc").build().ok()?;
                let sink = gstreamer::ElementFactory::make("filesink")
                    .property("location", &path)
                    .build()
                    .ok()?;
                pipeline
                    .add_many([&source, &caps, &convert, &wav, &sink])
                    .ok()?;
                gstreamer::Element::link_many([&source, &caps, &convert, &wav, &sink]).ok()?;
                let bus = pipeline.bus()?;
                pipeline.set_state(gstreamer::State::Playing).ok()?;
                let deadline = Instant::now() + Duration::from_secs(10);
                loop {
                    let Some(message) = bus.timed_pop(gstreamer::ClockTime::from_mseconds(100))
                    else {
                        if Instant::now() >= deadline {
                            let _ = pipeline.set_state(gstreamer::State::Null);
                            return None;
                        }
                        continue;
                    };
                    use gstreamer::MessageView;
                    match message.view() {
                        MessageView::Eos(..) => break,
                        MessageView::Error(error) => {
                            eprintln!("跳过：生成测试音频失败: {}", error.error());
                            let _ = pipeline.set_state(gstreamer::State::Null);
                            return None;
                        }
                        _ => {}
                    }
                }
                let _ = pipeline.set_state(gstreamer::State::Null);
                path.is_file().then_some(path)
            })
            .as_deref()
    }

    fn generated_multitrack_audio() -> Option<&'static Path> {
        static CACHE: OnceLock<Option<PathBuf>> = OnceLock::new();
        CACHE
            .get_or_init(|| {
                gst_init();
                let path = std::env::temp_dir().join("aweft_generated_multitrack_waveform_v2.mka");
                if path.is_file() {
                    return Some(path);
                }
                let pipeline = gstreamer::Pipeline::new();
                let mux = gstreamer::ElementFactory::make("matroskamux")
                    .build()
                    .ok()?;
                let sink = gstreamer::ElementFactory::make("filesink")
                    .property("location", &path)
                    .build()
                    .ok()?;
                pipeline.add_many([&mux, &sink]).ok()?;
                mux.link(&sink).ok()?;

                for (index, wave) in ["sine", "silence"].into_iter().enumerate() {
                    let source = gstreamer::ElementFactory::make("audiotestsrc")
                        .property_from_str("wave", wave)
                        .property("freq", 440.0f64)
                        .property("num-buffers", 16i32)
                        .build()
                        .ok()?;
                    let caps = gstreamer::ElementFactory::make("capsfilter")
                        .property(
                            "caps",
                            gstreamer::Caps::builder("audio/x-raw")
                                .field("rate", 8_000i32)
                                .field("channels", 1i32)
                                .build(),
                        )
                        .build()
                        .ok()?;
                    let convert = gstreamer::ElementFactory::make("audioconvert")
                        .build()
                        .ok()?;
                    let encoder = gstreamer::ElementFactory::make("flacenc").build().ok()?;
                    let queue = gstreamer::ElementFactory::make("queue")
                        .name(format!("track-{index}"))
                        .build()
                        .ok()?;
                    pipeline
                        .add_many([&source, &caps, &convert, &encoder, &queue])
                        .ok()?;
                    gstreamer::Element::link_many([&source, &caps, &convert, &encoder, &queue])
                        .ok()?;
                    queue.link(&mux).ok()?;
                }

                let bus = pipeline.bus()?;
                pipeline.set_state(gstreamer::State::Playing).ok()?;
                let message = bus.timed_pop_filtered(
                    gstreamer::ClockTime::from_seconds(10),
                    &[gstreamer::MessageType::Eos, gstreamer::MessageType::Error],
                );
                let _ = pipeline.set_state(gstreamer::State::Null);
                match message.as_ref().map(|message| message.view()) {
                    Some(gstreamer::MessageView::Eos(..)) if path.is_file() => Some(path),
                    Some(gstreamer::MessageView::Error(error)) => {
                        eprintln!("跳过：生成双音轨测试媒体失败: {}", error.error());
                        None
                    }
                    _ => None,
                }
            })
            .as_deref()
    }

    #[test]
    fn extracts_generated_audio_and_reports_incremental_progress() {
        let _serial = serial_lock();
        let Some(path) = generated_test_audio() else {
            eprintln!("跳过：GStreamer 音频插件不可用");
            return;
        };
        let uri = url::Url::from_file_path(path)
            .expect("测试音频路径无法转换为 URI")
            .to_string();
        let stream_id = first_audio_stream_id(&uri).expect("测试媒体应包含音轨 stream-id");
        let mut updates = Vec::new();
        let (buckets, duration) = extract_cancelable(
            &uri,
            &stream_id,
            || false,
            |start, _, batch| {
                updates.push((start, batch.len()));
                assert!(!batch.is_empty());
            },
        )
        .expect("提取测试音频波形失败");
        assert!(duration > 0.0);
        assert!(duration > 7.0, "测试音频必须覆盖首秒之后的时间范围");
        assert!(buckets.len() > 7_000, "每 1ms 必须追加一个稳定时间桶");
        assert!(buckets.iter().any(|bucket| bucket.max > 0.05));
        assert!(buckets.iter().any(|bucket| bucket.rms > 0.05));
        assert!(
            buckets
                .iter()
                .all(|bucket| bucket.rms <= bucket.max.max(-bucket.min) + 1e-5),
            "rms 是桶内样本的均方根，不可能超出 min/max 包络",
        );
        assert!(
            buckets
                .iter()
                .skip(buckets.len() / 4)
                .any(|bucket| bucket.max > 0.05),
            "首秒后的音频必须进入后续波形桶",
        );
        assert!(!updates.is_empty(), "解码过程中必须产生增量波形更新");
        assert!(
            updates.len() <= 3,
            "八秒测试音频应以约四秒为一批，而不是随 appsink sample 高频更新: {updates:?}"
        );
        assert_eq!(
            updates.iter().map(|(_, rows)| *rows).sum::<usize>(),
            buckets.len(),
            "EOS 前不足一批的尾部波形桶也必须完整提交"
        );
        assert_eq!(updates[0].0, 0, "首次更新必须从模型开头开始");
        assert!(
            updates.windows(2).all(|pair| pair[1].0 >= pair[0].0),
            "后续变更区间不能倒退并反复发送完整前缀: {updates:?}"
        );
        if updates.len() > 1 {
            assert!(
                updates.iter().skip(1).any(|(start, _)| *start > 0),
                "至少一个后续批次应只提交新增后缀: {updates:?}"
            );
        }
    }

    #[test]
    fn selects_each_audio_track_by_stream_id() {
        let _serial = serial_lock();
        let Some(path) = generated_multitrack_audio() else {
            eprintln!("跳过：Matroska 或 FLAC GStreamer 插件不可用");
            return;
        };
        let uri = url::Url::from_file_path(path)
            .expect("双音轨测试媒体路径无法转换为 URI")
            .to_string();
        let stream_ids = audio_stream_ids(&uri).expect("Discoverer 读取双音轨失败");
        assert_eq!(stream_ids.len(), 2, "测试媒体必须暴露两条真实音轨");
        let (tone, _) = extract_cancelable(&uri, &stream_ids[0], || false, |_, _, _| {})
            .expect("提取第一条测试音轨失败");
        let (silent, _) = extract_cancelable(&uri, &stream_ids[1], || false, |_, _, _| {})
            .expect("提取第二条测试音轨失败");
        let tone_peak = tone.iter().map(|bucket| bucket.max).fold(0.0f32, f32::max);
        let silent_min = silent
            .iter()
            .map(|bucket| bucket.min)
            .fold(0.0f32, f32::min);
        let silent_peak = silent
            .iter()
            .map(|bucket| bucket.max)
            .fold(0.0f32, f32::max);
        let silent_rms = silent
            .iter()
            .map(|bucket| bucket.rms)
            .fold(0.0f32, f32::max);

        assert!(tone_peak > 0.1, "第一轨应包含正弦波: {tone_peak}");
        assert!(
            silent_peak < 0.01 && silent_min > -0.01 && silent_rms < 0.01,
            "第二轨应保持静音，不能残留第一轨 preroll: [{silent_min}, {silent_peak}] rms={silent_rms}"
        );
    }

    #[test]
    fn cancellation_stops_waveform_without_publishing_progress() {
        let _serial = serial_lock();
        let Some(path) = generated_test_audio() else {
            eprintln!("跳过：GStreamer 音频插件不可用");
            return;
        };
        let uri = url::Url::from_file_path(path)
            .expect("测试音频路径无法转换为 URI")
            .to_string();
        let stream_id = first_audio_stream_id(&uri).expect("测试媒体应包含音轨 stream-id");
        let mut published = false;
        let (buckets, duration) =
            extract_cancelable(&uri, &stream_id, || true, |_, _, _| published = true)
                .expect("取消波形任务不应失败");

        assert!(!published, "已取消任务不能再覆盖 UI 模型");
        assert!(buckets.is_empty());
        assert_eq!(duration, 0.0);
    }

    #[test]
    fn slint_headless_accepts_waveform_model() {
        i_slint_backend_testing::init_no_event_loop();
        let instance = crate::bridge::types::MainWindow::new().expect("创建无头 MainWindow 失败");
        let tracks = std::rc::Rc::new(slint::VecModel::from(vec![
            crate::bridge::types::SlintAudioTrack {
                waveform: crate::bridge::types::SlintAudioWaveform {
                    duration: 2.0,
                    buckets: std::rc::Rc::new(slint::VecModel::from(vec![
                        crate::bridge::types::SlintWaveformBucket {
                            min: -0.25,
                            max: 0.25,
                            rms: 0.15,
                        },
                        crate::bridge::types::SlintWaveformBucket {
                            min: -0.75,
                            max: 0.75,
                            rms: 0.45,
                        },
                    ]))
                    .into(),
                    levels: slint::ModelRc::default(),
                },
                ..Default::default()
            },
        ]));
        let model = std::rc::Rc::new(slint::VecModel::from(vec![
            crate::bridge::types::SlintVideoTimeBar {
                audio_tracks: tracks.into(),
                ..Default::default()
            },
        ]));
        instance
            .global::<crate::bridge::types::VideoState>()
            .set_time_bars(slint::ModelRc::from(model));
        let track = instance
            .global::<crate::bridge::types::VideoState>()
            .get_time_bars()
            .row_data(0)
            .expect("无头 Slint 未收到视频时间条模型")
            .audio_tracks
            .row_data(0)
            .expect("无头 Slint 未收到嵌套音轨模型");
        assert_eq!(track.waveform.duration, 2.0);
        assert_eq!(track.waveform.buckets.row_count(), 2);
        let bucket = track
            .waveform
            .buckets
            .row_data(0)
            .expect("无头 Slint 未收到波形桶模型");
        assert_eq!((bucket.min, bucket.max, bucket.rms), (-0.25, 0.25, 0.15));
    }
}
