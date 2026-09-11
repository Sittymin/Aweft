//! GStreamer 管线搭建与格式协商。
//! 显式搭建解码管线（不依赖 playbin）：`uridecodebin` 自动择优解码器，
//! 视频分支经 `queue` → 后处理器（vapostproc 或 autovideoconvert）→ capsfilter
//! → appsink，音频分支可选。协商候选 caps 由当前渲染器能力生成。

use anyhow::Context as _;
use gstreamer::{Element, prelude::*};

use super::dmabuf;
use super::sink::SlintWGPUSink;

/// 创建视频 Sink（appsink 元素 + 帧缓冲）。由 `reducers/media` 在搭建显式
/// 管线时挂到转换链末端（不再依赖 playbin 的 video-sink 属性）。
/// caps 约束不设在 appsink 上（vapostproc 静态模板不含 dmabuf 特性，
/// 会在 pad link 期直接失败），而由管线中的 capsfilter 在协商期施加。
pub(crate) fn create_sink() -> (Element, SlintWGPUSink) {
    let slint_sink = SlintWGPUSink::new();
    let sink_element = slint_sink.element();
    (sink_element, slint_sink)
}

/// 生成 appsink caps。
/// - sysmem：提供 sink 能按实际格式处理的 Y410/Y444/Y42B/I420/YV12/
///   NV12/P010，交给 vapostproc 按输入格式损失评分选择，避免无条件降为 4:2:0。
/// - `dmabuf`：在 sysmem caps 前追加当前零拷贝导入器支持的 Y410 和 4:2:0 格式。
///   对导入器或驱动不支持的布局，协商仍可回退到后面的 sysmem 结构。
/// - `ten_bit`：wgpu 设备是否启用 TEXTURE_FORMAT_16BIT_NORM（P010 的
///   R16Unorm/Rg16Unorm 平面纹理必需，未启用时必须从 caps 排除，
///   否则纹理创建会 panic）。Y410 使用 Rgba8Unorm 保存 packed 字节，
///   不依赖该 feature。
/// - 故意不约束 pixel-aspect-ratio：PAR 跟随实际输入。显示尺寸由渲染端
///   按每帧协商出的 PAR 换算（见 `FramePlanes::display_width`），不让
///   GStreamer 为凑方形像素多一次重采样。
pub(crate) fn video_caps_with(dmabuf: bool, ten_bit: bool) -> gstreamer::Caps {
    // 注意字段值必须是 GstValueList（gstreamer::List），&[&str] 会成为 GStrv，
    // 与上游 pad 模板的 GstValueList 在链接期无法求交集
    let sysmem_formats = gstreamer::List::new(
        dmabuf::sysmem_capabilities(ten_bit)
            .map(|capability| capability.name)
            .collect::<Vec<_>>(),
    );
    let sysmem_caps = gstreamer::Caps::builder("video/x-raw")
        .field("format", &sysmem_formats)
        .build();
    if !dmabuf {
        return sysmem_caps;
    }

    // 不在这里填写裸 fourcc：裸 `NV12` 只代表线性 modifier，而 VA 驱动
    // 常常只导出 `NV12:0x...` 这样的 tiled modifier。READY 后从 vapostproc
    // 查询到的实际 drm-format 会填入最终 caps，并由 filter_runtime_caps
    // 去掉当前渲染器不支持的 fourcc。
    let mut caps = gstreamer::Caps::builder("video/x-raw")
        .features(["memory:DMABuf"])
        .field("format", "DMA_DRM")
        .build();
    // 同一个 capsfilter 同时给出零拷贝和系统内存候选，让 vapostproc 根据
    // 输入的色度采样/位深选择损失最小的结构，而不是由应用预先强制 4:2:0。
    caps.make_mut().append(sysmem_caps);
    caps
}

/// 保留 vapostproc 查询到的真实 DMABuf modifier，同时过滤掉没有对应
/// WGPU 平面布局的 fourcc。不能用裸 fourcc 重建 caps，否则会丢失 tiled
/// modifier，GStreamer 可能退回系统内存。
pub(crate) fn filter_runtime_caps(mut caps: gstreamer::Caps) -> gstreamer::Caps {
    caps.make_mut().filter_map_in_place(|features, structure| {
        if !features.contains("memory:DMABuf") {
            return gstreamer::CapsFilterMapAction::Keep;
        }

        if let Ok(format) = structure.get::<String>("drm-format") {
            return if dmabuf::plane_layout_from_drm_format(&format).is_some() {
                gstreamer::CapsFilterMapAction::Keep
            } else {
                gstreamer::CapsFilterMapAction::Remove
            };
        }

        let Ok(formats) = structure.get::<gstreamer::List>("drm-format") else {
            return gstreamer::CapsFilterMapAction::Remove;
        };
        let supported = formats
            .as_ref()
            .iter()
            .filter_map(|value| {
                let format = value.get::<String>().ok()?;
                dmabuf::plane_layout_from_drm_format(&format).map(|_| value.clone())
            })
            .collect::<Vec<_>>();
        if supported.is_empty() {
            return gstreamer::CapsFilterMapAction::Remove;
        }
        structure.set("drm-format", gstreamer::List::from_values(supported));
        gstreamer::CapsFilterMapAction::Keep
    });
    caps
}

/// 创建并实际初始化 VA 后处理器，同时查询当前驱动实际可用的输出 caps。
/// factory 存在只说明 GStreamer 注册了该元素；进入 READY 才会打开
/// DRM/VA display 和 VPP context。
fn try_vapostproc(requested_caps: &gstreamer::Caps) -> Option<(Element, gstreamer::Caps)> {
    let element = match gstreamer::ElementFactory::make("vapostproc").build() {
        Ok(element) => element,
        Err(error) => {
            log::debug!("创建 vapostproc 失败，使用 CPU 转换: {error}");
            return None;
        }
    };
    // 即使输入输出格式相同也强制经过 VPP：VA 解码器输出是 VAMemory，
    // 只有真正执行一次后处理，才能按下游候选导出 sysmem/DMABuf。
    element.set_property("disable-passthrough", true);

    if let Err(error) = element.set_state(gstreamer::State::Ready) {
        log::debug!("vapostproc 初始化失败，使用 CPU 转换: {error}");
        let _ = element.set_state(gstreamer::State::Null);
        return None;
    }

    // 交给 GStreamer 等待状态转换完成，避免用固定超时把慢速驱动误判为不可用。
    let (state_result, state, pending) = element.state(None);
    if state_result.is_err() || state != gstreamer::State::Ready {
        log::debug!("vapostproc 未进入 READY（当前 {state:?}，待定 {pending:?}），使用 CPU 转换");
        let _ = element.set_state(gstreamer::State::Null);
        return None;
    }

    let Some(src_pad) = element.static_pad("src") else {
        log::debug!("vapostproc 缺少 src pad，使用 CPU 转换");
        let _ = element.set_state(gstreamer::State::Null);
        return None;
    };

    // `query_caps()` 对 GstBaseTransform 会返回去掉 format/drm-format 的
    // transform-caps，不能用来枚举实际输出格式。vapostproc 的 pad template
    // 是插件注册时根据当前 VA display 探测生成的，保留真实 fourcc/modifier；
    // READY 探测保证该 template 对应的 VA display 已成功初始化。
    let supported_caps = filter_runtime_caps(src_pad.pad_template_caps());
    let mut selected_caps = requested_caps.intersect(&supported_caps);
    // vapostproc 的 pad template 通常自己声明 PAR=1/1（VPP 归一为方形像素）。
    // 落地"PAR 跟随输入"时不能让它随模板混进 capsfilter，否则协商仍会把
    // 变形片源强制重采样成 1:1，显示端永远拿不到原始 PAR。
    for structure in selected_caps.make_mut().iter_mut() {
        structure.remove_field("pixel-aspect-ratio");
    }
    if selected_caps.is_empty() {
        log::debug!(
            "vapostproc READY 但没有与渲染器候选相交的输出 caps（驱动 caps: {supported_caps:?}），使用 CPU 转换"
        );
        let _ = element.set_state(gstreamer::State::Null);
        return None;
    }
    log::debug!("vapostproc 动态输出 caps: {selected_caps}");

    let _ = element.set_state(gstreamer::State::Null);
    Some((element, selected_caps))
}

// —— 通用回退后端的管线搭建与启动 ——

/// 搭建解码管线（解码器按 rank 自动择优，VA-API 驱动存在时硬解优先）：
/// ```text
/// uridecodebin ─ video → queue ! vapostproc ! capsfilter ! appsink
///              │            （GPU 444p10→P010/NV12；dmabuf 零拷贝或 sysmem 下载；
///              │             无 VA 退 autovideoconvert CPU 转换）
///              └ audio → queue ! audioconvert ! volume ! AUDIO_SINK_FACTORY
///                                   （桌面 autoaudiosink，Android openslessink；
///                                    探测不到输出时改接 fakesink）
/// ```
/// vapostproc rank 为 0 不会被 autoplug，必须显式添加：它在 GPU 上完成
/// 444p10→YU12/P010 转换与 CSC（CPU 端 videoconvert 在 4K 下极昂贵）。
/// 测试可在任意支持 dmabuf 的管线按需协商零拷贝导入；不支持时走 sysmem 回退。
pub(crate) fn build_decode_pipeline_with_dmabuf(
    uri: &str,
    ten_bit: bool,
    dmabuf: bool,
) -> anyhow::Result<(
    gstreamer::Pipeline,
    Element,
    SlintWGPUSink,
    super::audio::SessionAudio,
)> {
    use gstreamer::prelude::*;

    let pipeline = gstreamer::Pipeline::new();
    let decoder = gstreamer::ElementFactory::make("uridecodebin3")
        .name("video-decoder")
        .property("uri", uri)
        .build()
        .context("创建 uridecodebin3 失败（gst-plugins-base 缺失？）")?;
    pipeline.add(&decoder)?;

    // 创建会话音频控制句柄
    let session_audio = super::audio::SessionAudio::new(pipeline.clone());

    // vapostproc rank 为 0 不会被 autoplug，必须显式添加：它在 GPU 上完成
    // 444p10→P010/NV12 转换与 CSC（CPU 端 videoconvert 在 4K 下极昂贵）。
    // 候选 caps 先按当前 WGPU 导入能力生成，再由 vapostproc READY 后的
    // 运行时 src caps 进一步收窄；最终格式由 GStreamer 的 fixate 规则选择。
    let (sink_element, wgpu_sink) = create_sink();
    let video_queue = gstreamer::ElementFactory::make("queue").build()?;
    let requested_caps = video_caps_with(dmabuf, ten_bit);
    let (video_convert, output_caps): (Element, gstreamer::Caps) =
        if let Some((vapostproc, caps)) = try_vapostproc(&requested_caps) {
            (vapostproc, caps)
        } else {
            (
                gstreamer::ElementFactory::make("autovideoconvert")
                    .build()
                    .context("创建 autovideoconvert 失败")?,
                video_caps_with(false, ten_bit),
            )
        };
    let capsfilter = gstreamer::ElementFactory::make("capsfilter")
        .property("caps", output_caps)
        .build()?;
    pipeline.add_many([&video_queue, &video_convert, &capsfilter, &sink_element])?;
    gstreamer::Element::link_many([&video_queue, &video_convert, &capsfilter, &sink_element])?;

    // 音频分支惰性创建：源确实带音频（pad_added 出现 audio pad）时才把
    // queue ! audioconvert ! volume ! autoaudiosink 加入管线。
    let video_entry = video_queue.clone();
    let pipeline_for_audio = pipeline.clone();
    let audio_state = std::sync::Arc::new(std::sync::Mutex::new(None));
    let session_audio_for_pad = session_audio.clone();

    decoder.connect_pad_added(move |_decoder, pad| {
        let stream_id = pad.stream_id().map(|s| s.to_string());
        let kind = classify_pad(pad);

        match kind {
            PadMediaKind::Video => {
                if let Some(sid) = stream_id {
                    session_audio_for_pad.set_video_stream_id(sid);
                }
                link_pad_by_media_type(pad, &video_entry);
            }
            PadMediaKind::Audio => {
                // 与「音频分支已建」配成一对：只有前者说明 uridecodebin 没把音频流
                // 解出来，只有后者说明解出来了但分支元素缺失。
                log::debug!("音频 pad 到达：{}", pad.name());
                let mut guard = audio_state.lock().unwrap_or_else(|e| e.into_inner());
                if guard.is_none() {
                    let (entry, vol) = build_audio_branch(&pipeline_for_audio, pad);
                    if let Some(vol) = vol {
                        session_audio_for_pad.set_volume_element(vol);
                    }
                    *guard = entry;
                }
                if let Some(ref entry) = *guard {
                    link_pad_by_media_type(pad, entry);
                }
            }
            PadMediaKind::Other => {}
        }
    });

    Ok((pipeline, sink_element, wgpu_sink, session_audio))
}

/// 系统当前是否存在可出声的音频输出（设备类 "Audio/Sink"）。
///
/// 判定只能提前做：`autoaudiosink` 在 READY→PAUSED 才真正打开设备，届时无声卡
/// 或无 PipeWire/Pulse 会话会让整条管线状态切换失败。这里逐个实例化设备提供方
/// （pipewire/pulse/alsa/oss…）并检查其枚举出的设备类；对应提供方插件缺失时
/// 同样判为无设备。
///
/// 注意不要在提供方层预过滤：`DeviceProviderFactory::has_classesv` 比对的是插件
/// klass（形如 `Sink/Source/Audio`），传 "Audio/Sink" 恒为 false。提供方实例化
/// 本身很快，且 `any` 在第一个枚举到输出设备的提供方处短路（本机实测 ~10ms）；
/// 只有判为无设备时才需要遍历全部提供方（本机 alsadeviceprovider 一家约 230ms，
/// 合计 ~0.25s）。该耗时发生在 `pad-added` 的流线程上，不影响 UI 线程。
#[cfg(not(target_os = "android"))]
fn has_audio_output() -> bool {
    gstreamer::DeviceProviderFactory::factories(gstreamer::Rank::NONE)
        .into_iter()
        .filter_map(|factory| factory.get())
        .any(|provider| {
            provider
                .devices()
                .into_iter()
                .any(|device| device.has_classesv(&["Audio/Sink"]))
        })
}

/// 有音频输出时，分支末端用的 sink 元素名。
///
/// Android 上不能用 `autoaudiosink`：它属于 `autodetect` 插件，靠设备提供方枚举
/// 设备来挑 sink，而我们链进来的插件没有任何 `DeviceProviderFactory`，它永远选不出
/// 设备。`openslessink` 直接开 OpenSL ES 的默认输出混音，不经过设备枚举这一层。
#[cfg(target_os = "android")]
const AUDIO_SINK_FACTORY: &str = "openslessink";

#[cfg(not(target_os = "android"))]
const AUDIO_SINK_FACTORY: &str = "autoaudiosink";

/// Android 上能否出声。
///
/// 上面那套设备提供方探测在这里用不了：链进来的插件一个都不注册
/// `DeviceProviderFactory`，恒为 false。所以这里探测的是 [`AUDIO_SINK_FACTORY`]
/// 这个元素在不在注册表里——它的归档没链上时确实无声，不能无条件当作有。
#[cfg(target_os = "android")]
fn has_audio_output() -> bool {
    gstreamer::ElementFactory::find(AUDIO_SINK_FACTORY).is_some()
}

/// 惰性创建音频分支 `queue ! audioconvert ! volume ! (AUDIO_SINK_FACTORY|fakesink)`
/// 并接入 `src_pad`，返回 (分支入口 queue, volume 元素)。
///
/// 顺序：分支内部互链 → 把 `src_pad` 链到分支入口 → 最后才同步状态。这是
/// `pad-added` 里动态加分支的惯例顺序（先接上再放行）。装机实测换序前后现象一致，
/// 它不是那次无声的成因，别把它当成修复。
///
/// 每个失败落点都要在日志里点名原因：这条分支建不起来的全部表现只是"画面正常但
/// 完全无声"，静默返回过一次让整轮排查无从下手。
fn build_audio_branch(
    pipeline: &gstreamer::Pipeline,
    src_pad: &gstreamer::Pad,
) -> (Option<Element>, Option<Element>) {
    use gstreamer::prelude::*;
    let Ok(queue) = gstreamer::ElementFactory::make("queue").build() else {
        log::warn!("音频分支未建立：注册表里没有 queue 元素");
        return (None, None);
    };
    let Ok(convert) = gstreamer::ElementFactory::make("audioconvert").build() else {
        log::warn!("音频分支未建立：注册表里没有 audioconvert 元素");
        return (None, None);
    };
    let Ok(volume) = gstreamer::ElementFactory::make("volume")
        .name(super::audio::AUDIO_VOLUME_NAME)
        .build()
    else {
        log::warn!("音频分支未建立：注册表里没有 volume 元素");
        return (None, None);
    };
    let has_output = has_audio_output();
    let sink_name = if has_output {
        AUDIO_SINK_FACTORY
    } else {
        log::warn!("没有可用的音频输出，音频分支改用 fakesink（不发声，仅保持时钟同步）");
        "fakesink"
    };
    let sink_builder = gstreamer::ElementFactory::make(sink_name);
    let Ok(sink) = (if has_output {
        sink_builder.build()
    } else {
        sink_builder.property("sync", true).build()
    }) else {
        log::warn!("音频分支未建立：注册表里没有 {sink_name} 元素");
        return (None, None);
    };

    if let Err(error) = pipeline.add_many([&queue, &convert, &volume, &sink]) {
        log::warn!("音频分支未建立：加入管线失败 {error}");
        return (None, None);
    }
    if let Err(error) = gstreamer::Element::link_many([&queue, &convert, &volume, &sink]) {
        log::warn!("音频分支未建立：元素互链失败 {error}");
        return (None, None);
    }
    link_pad_by_media_type(src_pad, &queue);

    for element in [&queue, &convert, &volume, &sink] {
        if let Err(error) = element.sync_state_with_parent() {
            log::warn!("同步动态音频分支状态失败（{}）: {error}", element.name());
        }
    }
    log::debug!("音频分支已建：sink = {sink_name}");
    (Some(queue), Some(volume))
}

/// uridecodebin 源 pad 按当前 caps 的 media type 分类（视频/音频/其他）。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum PadMediaKind {
    Video,
    Audio,
    Other,
}

/// 读取 pad 的当前 caps，用第一个 structure 的 media type 字符串分类。
fn classify_pad(pad: &gstreamer::Pad) -> PadMediaKind {
    use gstreamer::prelude::*;
    let name = pad.name();
    if name.starts_with("video") {
        return PadMediaKind::Video;
    }
    if name.starts_with("audio") {
        return PadMediaKind::Audio;
    }
    let caps = pad.current_caps().or_else(|| Some(pad.query_caps(None)));
    let Some(media) = caps.and_then(|caps| caps.structure(0).map(|s| s.name().to_string())) else {
        return PadMediaKind::Other;
    };
    if media.starts_with("video/") {
        PadMediaKind::Video
    } else if media.starts_with("audio/") {
        PadMediaKind::Audio
    } else {
        PadMediaKind::Other
    }
}

/// 把 uridecodebin 新增的源 pad 链到目标分支入口（视频 queue 或音频 queue）。
fn link_pad_by_media_type(pad: &gstreamer::Pad, target: &Element) {
    use gstreamer::prelude::*;

    let Some(sink_pad) = target.static_pad("sink") else {
        log::warn!("分支入口 {} 没有 sink pad，无法接入", target.name());
        return;
    };
    if sink_pad.is_linked() {
        return;
    }
    if let Err(error) = pad.link(&sink_pad) {
        log::warn!("{} 链接 {}.sink 失败: {error}", pad.name(), target.name());
    }
}
