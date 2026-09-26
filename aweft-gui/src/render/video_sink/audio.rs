//! 会话级实时音频与多音轨管理（基于 GStreamer 1.20+ 官方流选择机制）
//!
//! 在 `uridecodebin3` 统一管线拓扑下：
//! ```text
//! uridecodebin3 ─┬─ video_%u → queue → vapostproc → capsfilter → appsink
//!                └─ audio_%u → queue → audioconvert → volume → autoaudiosink
//! ```
//! 视频与音频天然在同一条管线内，共享统一时钟源（通常由声卡 `autoaudiosink` 提供）。
//!
//! ### 音轨切换与流选择
//! `uridecodebin3` 内部自带 `multiqueue`，所有音轨均有独立缓冲，不会反压阻塞解复用器。
//! 切换音轨时，把要保留的视频 stream-id 与目标音频 stream-id 收进一个 `Vec<&str>`，
//! 交给 `gstreamer::event::SelectStreams::builder(streams).build()` 构造事件，再用
//! `Pipeline::send_event` 发给管线（真实调用见 `SessionAudio::select_stream`）。
//! `decodebin3` 会在内部流线程进行原子无缝切换（Atomic Seamless Switch），无需对管线
//! 进行带 `FLUSH` 的 Seek，彻底杜绝管道冲刷卡顿、破音或 ASYNC 挂起。

use gstreamer::prelude::*;
use std::sync::{Arc, Mutex};

/// 音频尾段 volume 元素的固定名称（供定位或测试断言使用）。
pub(crate) const AUDIO_VOLUME_NAME: &str = "audio-volume";

/// 会话音频控制句柄。
#[derive(Clone)]
pub(crate) struct SessionAudio {
    inner: Arc<Mutex<SessionAudioInner>>,
}

struct SessionAudioInner {
    pipeline: gstreamer::Pipeline,
    /// 视频 stream-id（流选择时需保持视频与选中的音频同时被选）。
    video_stream_id: Option<String>,
    /// 当前选中的音频 stream-id。
    selected_audio_stream_id: Option<String>,
    /// 音频 volume 元素句柄，用于静音/增益。
    volume: Option<gstreamer::Element>,
    /// 是否已静音。
    muted: bool,
}

impl SessionAudio {
    pub(crate) fn new(pipeline: gstreamer::Pipeline) -> Self {
        Self {
            inner: Arc::new(Mutex::new(SessionAudioInner {
                pipeline,
                video_stream_id: None,
                selected_audio_stream_id: None,
                volume: None,
                muted: false,
            })),
        }
    }

    /// 设置已关联的 volume 元素。
    pub(crate) fn set_volume_element(&self, volume: gstreamer::Element) {
        let mut inner = self.inner.lock().unwrap_or_else(|e| e.into_inner());
        let muted = inner.muted;
        inner.volume = Some(volume.clone());
        let gain = if muted { 0.0 } else { 1.0 };
        volume.set_property("mute", muted);
        volume.set_property("volume", gain);
    }

    /// 设置视频流 ID。流选择时需要同时包含视频和音频。
    pub(crate) fn set_video_stream_id(&self, stream_id: String) {
        let mut inner = self.inner.lock().unwrap_or_else(|e| e.into_inner());
        inner.video_stream_id = Some(stream_id);
    }

    /// 静音控制。
    pub(crate) fn set_muted(&self, muted: bool) {
        let mut inner = self.inner.lock().unwrap_or_else(|e| e.into_inner());
        inner.muted = muted;
        if let Some(volume) = inner.volume.as_ref() {
            let gain = if muted { 0.0 } else { 1.0 };
            volume.set_property("mute", muted);
            volume.set_property("volume", gain);
        }
    }

    /// 选择要播放的音频 stream-id。
    /// 发送官方 `SelectStreams` 事件给管线，实现原子无缝切流。
    pub(crate) fn select_stream(&self, stream_id: &str) -> bool {
        let mut inner = self.inner.lock().unwrap_or_else(|e| e.into_inner());
        if inner.selected_audio_stream_id.as_deref() == Some(stream_id) {
            return true;
        }
        inner.selected_audio_stream_id = Some(stream_id.to_string());

        let mut streams = Vec::new();
        if let Some(ref vid) = inner.video_stream_id {
            streams.push(vid.as_str());
        }
        streams.push(stream_id);

        let event = gstreamer::event::SelectStreams::builder(streams).build();
        let sent = inner.pipeline.send_event(event);
        log::info!("通过 SelectStreams 切换音频轨至 {stream_id}, 结果: {sent}");
        sent
    }
}
