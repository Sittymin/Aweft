use crate::{bridge::types::MainWindow, store::AppStore};
use anyhow::{Context, Result};
use gstreamer::prelude::*;
use slint::ComponentHandle;
use std::{cell::RefCell, rc::Rc, sync::OnceLock};

static GSTREAMER_INIT_RESULT: OnceLock<Result<(), gstreamer::glib::Error>> = OnceLock::new();

fn initialize_gstreamer_once() -> Result<(), gstreamer::glib::Error> {
    // get_or_init 会执行一次闭包，并将闭包的返回值（这里是整个 Result 对象）
    // 存入 OnceLock，然后返回一个对该值的引用。
    let result: &Result<(), gstreamer::glib::Error> =
        GSTREAMER_INIT_RESULT.get_or_init(gstreamer::init);

    // 因为函数需要返回一个拥有的 Result，而不是引用，所以我们克隆它。
    result.clone()
}

pub fn initialize_media(store: &mut AppStore, video_uri: &str) -> Result<()> {
    initialize_gstreamer_once().context("GStreamer 初始化失败")?;

    let pipeline = gstreamer::ElementFactory::make("playbin")
        .property("uri", video_uri)
        .build()
        .context("创建 GStreamer playbin 失败")?
        .downcast::<gstreamer::Pipeline>()
        .expect("内部错误：Element 'playbin' 无法转换为 Pipeline");

    {
        let state = store.state_mut();
        state.video_pipeline = Some(pipeline);
    }

    Ok(())
}

// HACK: 后续需要修改为按需执行
pub fn connect_video_to_wgpu(
    store: &Rc<RefCell<AppStore>>,
    instance: &MainWindow,
    graphics_api: &slint::GraphicsAPI,
) -> Result<()> {
    use futures::channel::mpsc;

    let mut store_mut = store.borrow_mut();
    let state = store_mut.state_mut();

    let pipeline = state
        .video_pipeline
        .as_ref()
        .context("Pipeline 未初始化")?
        .clone();

    // 1. 创建 WGPU Sink
    let (sink_element, mut wgpu_sink) = crate::render::video_sink::create_sink(&pipeline);

    // 2. 获取消息总线
    let bus = pipeline.bus().context("无法获取 GStreamer Bus")?;

    // 3. 创建消息通道
    let (bus_sender, mut bus_receiver) = mpsc::unbounded::<gstreamer::Message>();

    // 4. ✅ 正确：使用 instance.as_weak()
    let weak_instance = instance.as_weak();
    let next_frame_notifier = Box::new(move || {
        // 从 GStreamer 线程安全地调用 Slint
        let _ = weak_instance.upgrade_in_event_loop(move |instance| {
            instance.window().request_redraw();
        });
    });

    // 5. 连接 WGPU
    wgpu_sink.connect(graphics_api, &bus, bus_sender, next_frame_notifier);

    // 6. 启动播放
    pipeline
        .set_state(gstreamer::State::Playing)
        .context("启动播放失败")?;

    // 7. 处理消息总线
    use futures::StreamExt;
    slint::spawn_local(async move {
        while let Some(msg) = bus_receiver.next().await {
            use gstreamer::MessageView;
            match msg.view() {
                MessageView::Eos(..) => {
                    log::info!("视频播放结束");
                    break;
                }
                MessageView::Error(err) => {
                    log::error!(
                        "GStreamer 错误: {} (来自 {:?})",
                        err.error(),
                        err.src().map(|s| s.path_string())
                    );
                    break;
                }
                MessageView::Warning(warning) => {
                    log::warn!(
                        "GStreamer 警告: {} (来自 {:?})",
                        warning.error(),
                        warning.src().map(|s| s.path_string())
                    );
                }
                _ => {}
            }
        }
    })
    .unwrap();

    // 8. 存储
    state.video_sink = Some(sink_element);
    state.wgpu_sink = Some(wgpu_sink);

    log::info!("✅ 视频系统连接成功，开始播放");
    Ok(())
}
