//! 离线音频波形提取：独立解码管线，绝不消费或干扰播放会话。

use gstreamer::prelude::*;
use gstreamer_app::AppSink;

/// 增量解析时每个峰值桶覆盖的固定时间宽度（1ms）。桶按时间追加，因此已经通过的
/// 时间段不会因媒体总时长稍后才可用而重新编号或移动。
pub const WAVEFORM_BUCKET_DURATION_SECS: f64 = 0.001;
/// 1ms 峰值桶使用 8kHz 单声道分析流，每桶聚合 8 个样本。相比直接扫描原始
/// 多声道/高采样率流，仍能减少 audioconvert/resample 和 CPU 扫描开销，同时保留
/// 毫秒级瞬态。
const WAVEFORM_ANALYSIS_RATE: i32 = 8_000;

/// 在调用方提供的线程中解码默认音频流并压缩成峰值桶。
/// `progress(start, duration, values)` 只提交从 `start` 开始的变更区间；调用方
/// 可以原地更新持久模型。`cancelled` 在解码循环内检查，替换媒体/退出时及时停工。
pub fn extract_cancelable<C, F>(
    uri: &str,
    cancelled: C,
    mut progress: F,
) -> anyhow::Result<(Vec<f32>, f32)>
where
    C: Fn() -> bool,
    F: FnMut(usize, f32, &[f32]),
{
    gstreamer::init().map_err(|error| anyhow::anyhow!("GStreamer 初始化失败: {error}"))?;
    let pipeline = gstreamer::Pipeline::new();
    // 波形只消费音频。uridecodebin 的默认 expose-all-streams=true 会在
    // caps 不匹配时仍暴露视频流；对 4K HEVC 容器会无谓启动视频解码器。
    let audio_caps = gstreamer::Caps::builder("audio/x-raw").build();
    let decoder = gstreamer::ElementFactory::make("uridecodebin")
        .property("uri", uri)
        .property("caps", audio_caps)
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

    pipeline.add_many([&decoder, &queue, &convert, &resample, &filter, &sink])?;
    gstreamer::Element::link_many([&queue, &convert, &resample, &filter, &sink])?;
    let entry = queue.clone();
    decoder.connect_pad_added(move |_decoder, pad| {
        let is_audio = pad
            .current_caps()
            .or_else(|| Some(pad.query_caps(None)))
            .and_then(|caps| caps.structure(0).map(|s| s.name().starts_with("audio/")))
            .unwrap_or(false);
        if is_audio {
            log::debug!("波形分析发现音频 pad: {}", pad.name());
            if let Some(sink_pad) = entry.static_pad("sink") {
                if !sink_pad.is_linked() {
                    let _ = pad.link(&sink_pad);
                }
            }
        }
    });

    pipeline.set_state(gstreamer::State::Playing)?;
    // Do not use the media duration for indexing. It is frequently unavailable until
    // decoding has started, and changing it would move already displayed waveform bars.
    let mut buckets = Vec::<f32>::new();
    // 仅记录自上次 progress 以来最早变化的桶；使用 usize::MAX 表示当前没有
    // 待提交区间，避免每批都从 0 重新提交整段历史波形。
    let mut dirty_start = usize::MAX;
    let bus = pipeline
        .bus()
        .ok_or_else(|| anyhow::anyhow!("无法获取音频分析 Bus"))?;
    loop {
        if cancelled() {
            let _ = pipeline.set_state(gstreamer::State::Null);
            return Ok((buckets, 0.0));
        }
        // 只在 appsink 上等待“数据或取消”最多 20ms；数据一到就立即处理。
        // 这不是等待 UI：progress 回调只通过 upgrade_in_event_loop 入队，
        // 解码线程不会等待主线程执行该闭包。短超时仅用于及时响应取消。
        let Some(sample) = appsink.try_pull_sample(gstreamer::ClockTime::from_mseconds(20))
        else {
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
        if dirty_start < buckets.len() {
            let duration = waveform_duration(buckets.len());
            progress(dirty_start, duration, &buckets[dirty_start..]);
            dirty_start = buckets.len();
        }
    }
    let _ = pipeline.set_state(gstreamer::State::Null);

    let duration = waveform_duration(buckets.len());
    if dirty_start < buckets.len() {
        progress(dirty_start, duration, &buckets[dirty_start..]);
    }
    Ok((buckets, duration as f32))
}

#[cfg(test)]
pub fn extract<F>(uri: &str, mut progress: F) -> anyhow::Result<(Vec<f32>, f32)>
where
    F: FnMut(f32, &[f32]),
{
    let mut all = Vec::new();
    extract_cancelable(
        uri,
        || false,
        |start, duration, batch| {
            if all.len() < start + batch.len() {
                all.resize(start + batch.len(), 0.0);
            }
            all[start..start + batch.len()].copy_from_slice(batch);
            progress(duration, &all);
        },
    )
}

fn accumulate_sample(sample: &gstreamer::Sample, buckets: &mut Vec<f32>) -> Option<usize> {
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
        buckets.resize(last_index + 1, 0.0);
    }

    // 若 PTS 跳过了尚未生成的时间段，必须从旧长度开始提交，才能让 UI
    // 逐行 push 中间的零桶；连续追加时该值正好等于 first_index。
    let mut changed_start = extended.then_some(previous_len.min(first_index));
    for (sample_index, bytes) in map.as_slice().chunks_exact(4).enumerate() {
        let bucket = first_index + sample_index / samples_per_bucket;
        let peak = f32::from_le_bytes(bytes.try_into().expect("4-byte PCM sample")).abs();
        if peak <= buckets[bucket] {
            continue;
        }
        buckets[bucket] = peak.min(1.0);
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
        let mut updates = Vec::new();
        let (amplitudes, duration) = extract_cancelable(
            &uri,
            || false,
            |start, _, buckets| {
                updates.push((start, buckets.len()));
                assert!(!buckets.is_empty());
            },
        )
        .expect("提取测试音频波形失败");
        assert!(duration > 0.0);
        assert!(duration > 7.0, "测试音频必须覆盖首秒之后的时间范围");
        assert!(amplitudes.len() > 7_000, "每 1ms 必须追加一个稳定时间桶");
        assert!(amplitudes.iter().any(|value| *value > 0.05));
        assert!(
            amplitudes
                .iter()
                .skip(amplitudes.len() / 4)
                .any(|value| *value > 0.05),
            "首秒后的音频必须进入后续波形桶",
        );
        assert!(!updates.is_empty(), "解码过程中必须产生增量波形更新");
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
    fn cancellation_stops_waveform_without_publishing_progress() {
        let _serial = serial_lock();
        let Some(path) = generated_test_audio() else {
            eprintln!("跳过：GStreamer 音频插件不可用");
            return;
        };
        let uri = url::Url::from_file_path(path)
            .expect("测试音频路径无法转换为 URI")
            .to_string();
        let mut published = false;
        let (amplitudes, duration) = extract_cancelable(&uri, || true, |_, _, _| published = true)
            .expect("取消波形任务不应失败");

        assert!(!published, "已取消任务不能再覆盖 UI 模型");
        assert!(amplitudes.is_empty());
        assert_eq!(duration, 0.0);
    }

    #[test]
    fn slint_headless_accepts_waveform_model() {
        i_slint_backend_testing::init_no_event_loop();
        let instance = crate::bridge::types::MainWindow::new().expect("创建无头 MainWindow 失败");
        let model = std::rc::Rc::new(slint::VecModel::from(vec![
            crate::bridge::types::SlintAudioWaveform {
                duration: 2.0,
                amplitudes: std::rc::Rc::new(slint::VecModel::from(vec![0.25, 0.75])).into(),
                levels: slint::ModelRc::default(),
            },
        ]));
        instance
            .global::<crate::bridge::types::VideoState>()
            .set_audio_waveforms(slint::ModelRc::from(model));
        let waveform = instance
            .global::<crate::bridge::types::VideoState>()
            .get_audio_waveforms()
            .row_data(0)
            .expect("无头 Slint 未收到波形模型");
        assert_eq!(waveform.duration, 2.0);
        assert_eq!(waveform.amplitudes.row_count(), 2);
    }
}
