use crate::actions::media::{MediaAction, VideoAction};
use crate::depsgraph::Tag;
use crate::store::AppStore;
use crate::store::state::AppState;
use anyhow::{Context, Result};
use gstreamer::prelude::*;
use std::sync::OnceLock;

static GSTREAMER_INIT_RESULT: OnceLock<Result<(), gstreamer::glib::Error>> = OnceLock::new();

fn initialize_gstreamer_once() -> Result<(), gstreamer::glib::Error> {
    GSTREAMER_INIT_RESULT.get_or_init(gstreamer::init).clone()
}

/// 分发媒体域 Action：管线在此创建/跳转；帧推送与 GPU 连接属渲染层
/// （`render/video_sink`），播放位置经渲染帧同步直写动画标量（见
/// `bridge/callbacks.rs:handle_before_rendering`）。媒体域当前不产生 tag。
impl AppStore {
    pub(crate) fn dispatch_media(&mut self, action: MediaAction) -> Result<Vec<Tag>> {
        match action {
            MediaAction::Video(VideoAction::Initialize { uri }) => initialize_media(self, &uri)?,
            MediaAction::Video(VideoAction::Seek { timestamp }) => {
                seek_to_secs(&self.state, timestamp as f64 / 1000.0);
            }
            MediaAction::Video(VideoAction::Play) => {
                set_playback_state(&self.state, gstreamer::State::Playing)
            }
            MediaAction::Video(VideoAction::Pause) => {
                set_playback_state(&self.state, gstreamer::State::Paused)
            }
            // 帧由 appsink 回调和渲染通知驱动，Action 不需要重复拉取。
            MediaAction::Video(VideoAction::UpdateFrame) => {}
            MediaAction::Subtitle(action) => log::warn!("字幕 Action 尚未实现：{action:?}"),
        }
        Ok(Vec::new())
    }
}

fn set_playback_state(state: &AppState, target: gstreamer::State) {
    let Some(pipeline) = state.video_pipeline.as_ref() else {
        log::warn!("无法切换视频状态：视频管线未初始化");
        return;
    };
    if let Err(error) = pipeline.set_state(target) {
        log::warn!("切换视频状态到 {target:?} 失败: {error}");
    }
}

pub fn initialize_media(store: &mut AppStore, video_uri: &str) -> Result<()> {
    initialize_gstreamer_once().context("GStreamer 初始化失败")?;

    // 后端选择/搭建/嵌入/启动全部在渲染层编排（按会话与平台选择）
    crate::render::video_sink::initialize(store, video_uri)
}

/// 查询播放位置（秒）与视频帧时长（秒）：仅播放中返回（暂停时编辑器是
/// 时间权威，避免拖动播放头被陈旧位置回写）。无管线/无位置返回 None。
pub(crate) fn playing_position(state: &AppState) -> Option<(f64, Option<f64>)> {
    let pipeline = state.video_pipeline.as_ref()?;
    if pipeline.current_state() != gstreamer::State::Playing {
        return None;
    }
    // 纳秒转秒：seconds() 会截断到整秒，导致播放头一秒一跳
    let secs = pipeline
        .query_position::<gstreamer::ClockTime>()?
        .nseconds() as f64
        / 1_000_000_000.0;
    let frame_dur = state
        .wgpu_sink
        .as_ref()
        .and_then(|sink| sink.frame_duration_secs());
    Some((secs, frame_dur))
}

/// 跳转管线到指定秒（编辑器拖动播放头反向驱动视频）。无管线时静默忽略；
/// FLUSH 立即生效，KEY_UNIT 快速定位（高频拖动以精度换流畅）。
pub(crate) fn seek_to_secs(state: &AppState, secs: f64) {
    use gstreamer::prelude::*;
    let Some(pipeline) = state.video_pipeline.as_ref() else {
        return;
    };
    let ok = pipeline.seek_simple(
        gstreamer::SeekFlags::FLUSH | gstreamer::SeekFlags::KEY_UNIT,
        gstreamer::ClockTime::from_mseconds((secs.max(0.0) * 1000.0) as u64),
    );
    if let Err(e) = ok {
        log::warn!("视频 seek 失败: {e}");
    }
}
