//! 会话级实时音频分支，与视频在**同一条** GStreamer 管线内同源解析：
//!
//! ```text
//! urisourcebin → parsebin ─┬─ video elementary → 视频 decodebin → … → wgpu appsink
//!                          ├─ audio elementary → input-selector → 音频 decodebin
//!                          │                     （active-pad 运行中可写）
//!                          └─ 其它流 → fakesink
//! ```
//!
//! 音轨与视频共享同一个解析头与同一管线时钟，因此 A/V 天然同步，不存在
//! 两条独立音频管线的双播问题。切换音轨只改写 input-selector 的 `active-pad`
//! （元素状态 Playing 下可写），未选中的音轨在 selector 内被丢弃，不启动解码器。
//! 解码尾段 `queue → audioconvert → volume → autoaudiosink` 的 `volume` 命名
//! 固定为 [`AUDIO_VOLUME_NAME`]，是静音/增益的唯一软件入口。
//!
//! 本模块只面向任意 `Pipeline` 内的 parsebin 输出 pad；未来“单独添加音频”的纯
//! 音频会话（独立 parse 头、无视频分支）可复用同一套 selector/尾段装配逻辑。

use std::collections::HashMap;
use std::sync::{Arc, Mutex};

use anyhow::Context as _;
use gstreamer::{prelude::*, Element};

/// 音频尾段 volume 元素的固定名：静音写入、测试断言都以它定位。
pub(crate) const AUDIO_VOLUME_NAME: &str = "audio-volume";

/// 会话内实时音频的运行时句柄。句柄在管线构建期就存在（哪怕文件无音轨）；
/// 真正的 selector/decodebin/尾段在首个音频 pad 到达时才惰性装配，纯视频媒体
/// 不会创建 autoaudiosink（避免 headless 无音频设备时卡住整条视频管线）。
#[derive(Clone)]
pub(crate) struct SessionAudio {
    pub(crate) inner: Arc<Mutex<SessionAudioInner>>,
}

pub(crate) struct SessionAudioInner {
    pub(crate) pipeline: gstreamer::Pipeline,
    // selector 上按音轨出现顺序排列的 stream-id（默认取首个）。
    pub(crate) order: Vec<String>,
    // stream-id → input-selector 的 sink pad（parsebin audio pad 到达时建立）。
    pub(crate) selector_pads: HashMap<String, gstreamer::Pad>,
    pub(crate) selector: Option<Element>,
    pub(crate) decodebin: Option<Element>,
    pub(crate) tail_queue: Option<Element>,
    pub(crate) volume: Option<Element>,
    pub(crate) active_stream: Option<String>,
    // reducer 在 parsebin 尚未暴露该音轨前发起的选流请求，pad 到达或 no-more-pads 时落地。
    pub(crate) pending_stream: Option<String>,
    pub(crate) chain_ready: bool,
}

/// 为会话管线装配音频分支路由。parsebin 的音频 elementary pad 逐个到达时，
/// 首次到达会惰性创建 selector → decodebin → 尾段，随后把 pad 连进 selector；
/// `no-more-pads` 时把首个音轨（或 reducer 预先请求的 pending 音轨）设为 active。
pub(crate) fn build_session_audio(
    pipeline: &gstreamer::Pipeline,
    parsebin: &Element,
) -> SessionAudio {
    let inner = Arc::new(Mutex::new(SessionAudioInner {
        pipeline: pipeline.clone(),
        order: Vec::new(),
        selector_pads: HashMap::new(),
        selector: None,
        decodebin: None,
        tail_queue: None,
        volume: None,
        active_stream: None,
        pending_stream: None,
        chain_ready: false,
    }));

    let attach_owner = inner.clone();
    parsebin.connect_pad_added(move |_parsebin, pad| {
        let Some(stream_id) = pad.stream_id() else {
            return;
        };
        let stream_id = stream_id.to_string();
        let is_audio = pad
            .current_caps()
            .or_else(|| Some(pad.query_caps(None)))
            .and_then(|caps| caps.structure(0).map(|s| s.name().starts_with("audio/")))
            .unwrap_or(false);
        if !is_audio {
            return;
        }
        let mut guard = attach_owner.lock().unwrap_or_else(|e| e.into_inner());
        guard.attach_audio_pad(pad, &stream_id);
    });

    let activate_owner = inner.clone();
    parsebin.connect_no_more_pads(move |_parsebin| {
        let mut guard = activate_owner.lock().unwrap_or_else(|e| e.into_inner());
        guard.activate();
    });

    SessionAudio { inner }
}

impl SessionAudio {
    /// 请求把指定音轨设为当前播放音轨。parsebin 已暴露该音轨时立即切换并返回
    /// `true`；尚未暴露时记录为 pending，pad 到达后自动落地并返回 `false`。
    /// 切换后调用方需对整条管线做一次 flush-seek，让新音轨从当前时间重新对齐。
    pub(crate) fn select_stream(&self, stream_id: &str) -> bool {
        let mut guard = self.inner.lock().unwrap_or_else(|e| e.into_inner());
        if guard.active_stream.as_deref() == Some(stream_id) {
            return true;
        }
        if guard.selector_pads.contains_key(stream_id) {
            guard.pending_stream = Some(stream_id.to_owned());
            guard.activate();
            return true;
        }
        guard.pending_stream = Some(stream_id.to_owned());
        false
    }

    /// 通过命名 volume 元素实现会话内静音/增益。尾段尚未装配（无音轨）时为空操作。
    pub(crate) fn set_muted(&self, muted: bool) {
        let guard = self.inner.lock().unwrap_or_else(|e| e.into_inner());
        let Some(volume) = guard.volume.as_ref() else {
            return;
        };
        let gain = if muted { 0.0 } else { 1.0 };
        volume.set_property("mute", muted);
        volume.set_property("volume", gain);
    }

    /// 已接入 input-selector 的音轨数（parsebin 逐个暴露时递增）。测试用它断言
    /// 音频 pad 确实被本模块独占路由，而不是被视频侧抢走。
    pub(crate) fn attached_stream_count(&self) -> usize {
        let guard = self.inner.lock().unwrap_or_else(|e| e.into_inner());
        guard.order.len()
    }
}

impl SessionAudioInner {
    /// 处理 parsebin 的单个音频 pad：惰性装配尾段，把 pad 连进 selector 并登记。
    fn attach_audio_pad(&mut self, src_pad: &gstreamer::Pad, stream_id: &str) {
        if self.selector_pads.contains_key(stream_id) {
            return;
        }
        if self.ensure_chain().is_none() {
            log::warn!("音频尾段装配失败，静默跳过 stream-id {stream_id}");
            return;
        }
        let Some(selector) = self.selector.as_ref() else {
            return;
        };
        let Some(sel_sink) = selector.request_pad_simple("sink_%u") else {
            log::warn!("input-selector 请求 sink pad 失败，跳过 stream-id {stream_id}");
            return;
        };
        if let Err(error) = src_pad.link(&sel_sink) {
            log::warn!("音频流 {stream_id} 连接 input-selector 失败: {error}");
            return;
        }
        self.order.push(stream_id.to_owned());
        self.selector_pads
            .insert(stream_id.to_owned(), sel_sink.clone());
        // 尽可能早地把音轨设为 active：autoaudiosink 在无输入时会一直停在
        // preroll，拖住整条管线无法进入 Paused/Playing。首个音轨到达即激活
        // （默认首轨；若 reducer 已登记了后续目标，该目标 pad 到达时会在
        // attach 里再次切换）。
        if self.active_stream.is_none() || self.pending_stream.is_some() {
            self.activate();
        }
    }

    /// 惰性创建 `input-selector → 音频 decodebin → queue → audioconvert →
    /// volume → autoaudiosink` 并加入管线。只允许在首个音频 pad 到达时调用一次。
    fn ensure_chain(&mut self) -> Option<()> {
        if self.chain_ready {
            return Some(());
        }
        // input-selector 默认 sync-streams=true，会在非活动输入 pad 上按时钟/活动流时间
        // 同步等待而不立即丢弃数据。同一解复用器（如 matroskademux）串行推送各流时，
        // 非活动音轨 pad 会反压卡住解复用器线程，导致视频数据流无法送达视频解码器，
        // 整条管线在暂停预滚动（Preroll）期挂起（表现为多音轨视频无画面且无法播放）。
        // 置为 false 让非活动音轨的数据立即丢弃，解复用器畅通推进。
        let selector = gstreamer::ElementFactory::make("input-selector")
            .property("sync-streams", false)
            .build()
            .context("创建 input-selector 失败（coreelements 缺失？）")
            .ok()?;
        let decodebin = gstreamer::ElementFactory::make("decodebin").build().ok()?;
        let queue = gstreamer::ElementFactory::make("queue").build().ok()?;
        let convert = gstreamer::ElementFactory::make("audioconvert")
            .build()
            .ok()?;
        let volume = gstreamer::ElementFactory::make("volume")
            .name(AUDIO_VOLUME_NAME)
            .build()
            .ok()?;
        // 无真实音频设备的 headless 环境用 `AWEFT_AUDIO_SINK=fakesink` 跑真实
        // 管线回归测试；生产默认 autoaudiosink。
        let sink = if std::env::var("AWEFT_AUDIO_SINK").as_deref() == Ok("fakesink") {
            gstreamer::ElementFactory::make("fakesink")
                .property("sync", false)
                .build()
                .ok()?
        } else {
            gstreamer::ElementFactory::make("autoaudiosink").build().ok()?
        };
        self.pipeline
            .add_many([&selector, &decodebin, &queue, &convert, &volume, &sink])
            .ok()?;
        gstreamer::Element::link_many([&queue, &convert, &volume, &sink]).ok()?;

        // selector 的 src（Always）→ 音频 decodebin 的 sink（Always）。
        let selector_src = selector.static_pad("src")?;
        let decodebin_sink = decodebin.static_pad("sink")?;
        selector_src.link(&decodebin_sink).ok()?;

        // decodebin 解码后的 raw-audio pad 是动态的，出现时再连进尾段 queue。
        let tail_entry = queue.clone();
        decodebin.connect_pad_added(move |_decodebin, pad| {
            let Some(tail_sink) = tail_entry.static_pad("sink") else {
                return;
            };
            if !tail_sink.is_linked() {
                let _ = pad.link(&tail_sink);
            }
        });

        // 分支是在父管线可能已进入 Paused/Playing 后动态加入的：新元素默认停在
        // NULL，必须同步到父状态，否则上游很快被反压填满（沿用波形 fakesink 补丁的
        // 教训）。音频设备不可用时仅记日志，不把错误上抛到视频会话。
        for element in [&selector, &decodebin, &queue, &convert, &volume, &sink] {
            if let Err(error) = element.sync_state_with_parent() {
                log::warn!("同步音频分支状态失败（{}）: {error}", element.name());
            }
        }

        self.selector = Some(selector);
        self.decodebin = Some(decodebin);
        self.tail_queue = Some(queue);
        self.volume = Some(volume);
        self.chain_ready = true;
        Some(())
    }

    /// 选择当前播放音轨：优先 reducer 的 pending 请求，否则默认首个到达的音轨。
    fn activate(&mut self) {
        let target = self
            .pending_stream
            .clone()
            .filter(|id| self.selector_pads.contains_key(id))
            .or_else(|| self.order.first().cloned());
        let Some(stream_id) = target else {
            if let Some(pending) = &self.pending_stream {
                log::warn!("请求的音轨 stream-id {pending} 未在 parsebin 中暴露，本会话保持静音");
            }
            return;
        };
        let Some(selector_pad) = self.selector_pads.get(&stream_id).cloned() else {
            return;
        };
        if self.active_stream.as_deref() == Some(stream_id.as_str()) {
            self.pending_stream = None;
            return;
        }
        if let Some(selector) = self.selector.as_ref() {
            selector.set_property("active-pad", selector_pad);
            log::info!("会话音频切换至 stream-id {stream_id}");
        }
        self.active_stream = Some(stream_id);
        self.pending_stream = None;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// 静音经命名 volume 元素写入：置 mute 并把增益归零，解除时恢复 1.0。
    /// 只构造句柄与 volume 元素，不进入真实管线状态，因此无需音频设备。
    #[test]
    fn mute_writes_through_named_volume() {
        gstreamer::init().unwrap();
        let pipeline = gstreamer::Pipeline::new();
        let volume = gstreamer::ElementFactory::make("volume")
            .name(AUDIO_VOLUME_NAME)
            .build()
            .expect("需要 volume 插件");
        pipeline.add(&volume).unwrap();

        let handle = SessionAudio {
            inner: Arc::new(Mutex::new(SessionAudioInner {
                pipeline,
                order: Vec::new(),
                selector_pads: HashMap::new(),
                selector: None,
                decodebin: None,
                tail_queue: None,
                volume: Some(volume.clone()),
                active_stream: None,
                pending_stream: None,
                chain_ready: false,
            })),
        };

        handle.set_muted(true);
        assert!(volume.property::<bool>("mute"), "静音时应置 mute=true");
        assert_eq!(volume.property::<f64>("volume"), 0.0, "静音时应置 volume=0");

        handle.set_muted(false);
        assert!(!volume.property::<bool>("mute"), "解除静音应置 mute=false");
        assert_eq!(
            volume.property::<f64>("volume"),
            1.0,
            "解除静音应置 volume=1"
        );
    }
}
