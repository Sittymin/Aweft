use crate::store::AppStore;
use slint::ComponentHandle as _;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, OnceLock};

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
pub(super) fn start_waveform_extraction(
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
        log::warn!("音频波形后台 worker 已退出");
    }
}
