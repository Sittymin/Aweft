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

// —— 视频会话管线的搭建 ——

/// 搭建视频会话管线（解码器按 rank 自动择优，VA-API 驱动存在时硬解优先）：
/// ```text
/// urisourcebin → parsebin ─┬─ video elementary → 视频 decodebin
///                          │     → queue → vapostproc → capsfilter → appsink
///                          │        （GPU 444p10→P010/NV12；dmabuf 零拷贝或
///                          │          sysmem 下载；无 VA 退 autovideoconvert）
///                          └─ audio elementary → 见 `super::audio`（input-selector
///                             选流，解码链在本管线内，与视频共享时钟）
/// ```
/// 头部显式分层为 urisourcebin + parsebin：parsebin 暴露与 Discoverer 相同的
/// elementary stream-id，音轨选择按该 id 进行（视频分支只取首个视频流，其余
/// 视频流与字幕等接到 fakesink 丢弃，保证解复用推进到 EOS 且不启动多余解码器）。
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
    let source = gstreamer::ElementFactory::make("urisourcebin")
        .property("uri", uri)
        // 关闭 parse-streams：让显式 parsebin 暴露与 Discoverer 一致的
        // elementary stream-id，供音频分支按 stream-id 选流。
        .property("parse-streams", false)
        .property("use-buffering", false)
        .build()
        .context("创建 urisourcebin 失败（gst-plugins-base 缺失？）")?;
    let parser = gstreamer::ElementFactory::make("parsebin")
        .build()
        .context("创建 parsebin 失败")?;
    pipeline.add_many([&source, &parser])?;

    // urisourcebin 的 src pad 是 sometimes（typefind 后才出现）：出现即连进
    // parsebin 的静态 sink，与波形提取管线同构。
    let parser_sink = parser.static_pad("sink").context("parsebin 缺少 sink pad")?;
    source.connect_pad_added(move |_source, pad| {
        if !parser_sink.is_linked()
            && let Err(error) = pad.link(&parser_sink)
        {
            log::warn!("URISource 连接 parsebin 失败: {error}");
        }
    });

    // 视频解码：parsebin 的 video elementary pad → 视频 decodebin（显式，VA 硬解
    // 优先）→ 视频 tail。decodebin 的 raw-video src pad 是动态的，出现后再链
    // 到 video_queue。
    let decoder = gstreamer::ElementFactory::make("decodebin")
        .name("video-decoder")
        .build()
        .context("创建视频 decodebin 失败")?;
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
    pipeline.add_many([
        &decoder,
        &video_queue,
        &video_convert,
        &capsfilter,
        &sink_element,
    ])?;
    gstreamer::Element::link_many([&video_queue, &video_convert, &capsfilter, &sink_element])?;

    let video_tail = video_queue.clone();
    decoder.connect_pad_added(move |_decoder, pad| {
        let Some(tail_sink) = video_tail.static_pad("sink") else {
            return;
        };
        if !tail_sink.is_linked() {
            let _ = pad.link(&tail_sink);
        }
    });

    // parsebin 的 elementary pad 路由：视频交给视频 decodebin，音频交给
    // `super::audio` 的处理器（input-selector 选流，独占这些 pad——本处理器
    // 若抢先给音频 pad 接 fakesink，audio.rs 就再也链不上 selector，autoaudiosink
    // 无数据会把整条管线拖在 preroll），其它流接到 fakesink。
    let dynamic_pipeline = pipeline.clone();
    let decoder_entry = decoder.clone();
    parser.connect_pad_added(move |_parser, pad| {
        let Some(media) = pad
            .current_caps()
            .or_else(|| Some(pad.query_caps(None)))
            .and_then(|caps| caps.structure(0).map(|s| s.name().to_string()))
        else {
            return;
        };
        if media.starts_with("audio/") {
            // 音频 pad 由 audio.rs 统一接入 input-selector，这里不得触碰。
            return;
        }
        let decode_target = if media.starts_with("video/") {
            decoder_entry.static_pad("sink").filter(|sink| !sink.is_linked())
        } else {
            None
        };
        if let Some(sink_pad) = decode_target {
            if let Err(error) = pad.link(&sink_pad) {
                log::warn!("视频流连接 decodebin 失败: {error}");
            }
            return;
        }
        // 非目标流必须被消费，容器才能持续解复用到 EOS；异步 fakesink 直接丢弃
        // 不会为视频/字幕/其它音轨启动解码器（与波形提取管线的口径一致）。
        let Ok(discard) = gstreamer::ElementFactory::make("fakesink")
            .property("sync", false)
            .property("async", false)
            .build()
        else {
            log::warn!("fakesink 创建失败，跳过非目标流");
            return;
        };
        if dynamic_pipeline.add(&discard).is_err() {
            log::warn!("fakesink 加入管线失败，跳过非目标流");
            return;
        }
        if discard.sync_state_with_parent().is_err() {
            log::warn!("fakesink 状态同步失败，跳过非目标流");
            return;
        }
        if let Some(sink_pad) = discard.static_pad("sink")
            && !sink_pad.is_linked()
        {
            let _ = pad.link(&sink_pad);
        }
    });

    // 音频分支装配：parsebin 的 audio pad 到达时由 audio.rs 惰性创建
    // input-selector → 音频 decodebin → 尾段，并登记到会话句柄。
    let session_audio = super::audio::build_session_audio(&pipeline, &parser);

    Ok((pipeline, sink_element, wgpu_sink, session_audio))
}
