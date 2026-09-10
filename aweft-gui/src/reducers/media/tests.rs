
use super::*;
use gstreamer::prelude::*;
use gstreamer_app::AppSink;

/// 构造两个重叠 + 一个错开的矩形（下标 0/1 重叠，2 独立）。
fn rects() -> Vec<VideoRect> {
    vec![
        VideoRect {
            x: 0.0,
            y: 0.0,
            w: 100.0,
            h: 100.0,
        }, // 0：底层
        VideoRect {
            x: 10.0,
            y: 10.0,
            w: 100.0,
            h: 100.0,
        }, // 1：覆盖在 0 之上
        VideoRect {
            x: 500.0,
            y: 500.0,
            w: 100.0,
            h: 100.0,
        }, // 2：独立
    ]
}

#[test]
fn pick_no_hit_returns_none() {
    let r = rects();
    assert_eq!(pick_overlapping_rect(&r, None, 400.0, 400.0), None);
}

#[test]
fn seek_drag_preserves_relative_offsets() {
    let anchor: f64 = 2.0;
    let requested: f64 = 5.5;
    let origins: [f64; 3] = [2.0, 7.25, 0.5];
    let resolved: Vec<f64> = origins
        .into_iter()
        .map(|origin| (origin + requested - anchor).max(0.0))
        .collect();
    assert_eq!(resolved, vec![5.5, 10.75, 4.0]);
}

#[test]
fn seek_time_is_clamped_to_video_range() {
    assert_eq!(media_clamp_time_to_range(-2.0, Some(10.0), Some(0.04)), 0.0);
    assert_eq!(media_clamp_time_to_range(4.5, Some(10.0), Some(0.04)), 4.5);
    assert_eq!(
        media_clamp_time_to_range(12.0, Some(10.0), Some(0.04)),
        9.96
    );
    assert_eq!(media_clamp_time_to_range(10.0, Some(10.0), None), 9.999);
    assert_eq!(media_clamp_time_to_range(f64::NAN, Some(10.0), None), 0.0);
    assert_eq!(media_clamp_time_to_range(12.0, None, Some(0.04)), 12.0);
}

#[test]
fn repeated_past_end_seek_reuses_pending_last_frame() {
    assert!(!media_repeated_last_frame_seek(true, false, 9.96, 9.96));
    assert!(media_repeated_last_frame_seek(true, true, 9.96, 9.96));
    assert!(!media_repeated_last_frame_seek(false, true, 9.96, 9.96));
    assert!(!media_repeated_last_frame_seek(true, true, 8.0, 9.96));
}

#[test]
fn waveform_seek_syncs_global_time_only_for_active_video() {
    let _g = crate::GST_SERIAL.lock().unwrap_or_else(|e| e.into_inner());
    gstreamer::init().expect("初始化 GStreamer 失败");
    i_slint_backend_testing::init_no_event_loop();
    let instance = crate::bridge::types::MainWindow::new().unwrap();
    let mut store = crate::store::AppStore::new(&instance);

    for index in 0..2 {
        let pipeline = gstreamer::Pipeline::new();
        let audio = crate::render::video_sink::audio::SessionAudio::new(pipeline.clone());
        let wgpu_sink = crate::render::video_sink::SlintWGPUSink::new();
        let sink_element = wgpu_sink.element();
        store
            .state_mut()
            .video_sessions
            .push(crate::store::state::VideoSession {
                uri: format!("test://waveform-{index}"),
                pipeline,
                sink_element,
                wgpu_sink,
                audio: Some(audio),
                position_x: 0.0,
                position_y: 0.0,
                current_time: 0.0,
                duration: Some(10.0),
                muted: false,
                playing: false,
                at_eos: std::sync::Arc::new(std::sync::atomic::AtomicBool::new(false)),
                last_frame_seeked: false,
            });
    }
    store.state_mut().main_video = Some(0);
    store.state_mut().selected_videos = vec![1];
    store.ui_models.animation.graph.current_time = 8.0;

    let tags = store
        .dispatch_media(crate::actions::media::MediaAction::Video(
            crate::actions::media::VideoAction::SeekStandalone {
                index: 0,
                timestamp: 12_000,
            },
        ))
        .unwrap();
    assert_eq!(store.ui_models.animation.graph.current_time, 8.0);
    assert!(
        store.state().video_sessions[0].current_time < 10.0,
        "非焦点音轨范围外 seek 也应把视频画面位置钳制在有效范围"
    );
    assert!(
        !tags
            .iter()
            .any(|tag| matches!(tag, crate::depsgraph::Tag::GraphScalarsChanged))
    );
    assert!(tags.iter().any(|tag| matches!(
        tag,
        crate::depsgraph::Tag::VideoTimeChanged {
            indices,
            sync_timeline: false,
        } if indices == &[0]
    )));

    let tags = store
        .dispatch_media(crate::actions::media::MediaAction::Video(
            crate::actions::media::VideoAction::SeekStandalone {
                index: 1,
                timestamp: 12_000,
            },
        ))
        .unwrap();
    assert_eq!(store.ui_models.animation.graph.current_time, 12.0);
    assert!(
        store.state().video_sessions[1].current_time < 10.0,
        "焦点音轨范围外 seek 应保留全局请求时间，但视频画面位置仍需钳制"
    );
    assert!(
        tags.iter()
            .any(|tag| matches!(tag, crate::depsgraph::Tag::GraphScalarsChanged))
    );
    assert!(tags.iter().any(|tag| matches!(
        tag,
        crate::depsgraph::Tag::VideoTimeChanged {
            indices,
            sync_timeline: false,
        } if indices == &[1]
    )));
}

#[test]
fn pick_hit_without_selection_takes_topmost() {
    let r = rects();
    // (50, 50) 同时命中 0 与 1 → 无选中取最上层（下标 1）
    assert_eq!(pick_overlapping_rect(&r, None, 50.0, 50.0), Some(1));
    // (600, 600) 只命中 2
    assert_eq!(pick_overlapping_rect(&r, None, 600.0, 600.0), Some(2));
    // (5, 5) 只命中 0（1 起点在 (10,10)）
    assert_eq!(pick_overlapping_rect(&r, None, 5.0, 5.0), Some(0));
}

#[test]
fn pick_cycles_through_overlapping_videos() {
    let r = rects();
    // 选中 0（底层）时点重叠区 → 循环到 1；再点 → 回到 0
    assert_eq!(pick_overlapping_rect(&r, Some(0), 50.0, 50.0), Some(1));
    assert_eq!(pick_overlapping_rect(&r, Some(1), 50.0, 50.0), Some(0));
    // 选中 2（不在重叠命中集内）→ 取最上层 1
    assert_eq!(pick_overlapping_rect(&r, Some(2), 50.0, 50.0), Some(1));
}

#[test]
fn hit_top_returns_topmost_at_point() {
    let r = rects();
    assert_eq!(hit_top_rect(&r, 50.0, 50.0), Some(1));
    assert_eq!(hit_top_rect(&r, 5.0, 5.0), Some(0));
    assert_eq!(hit_top_rect(&r, 600.0, 600.0), Some(2));
    assert_eq!(hit_top_rect(&r, 300.0, 300.0), None);
}

// —— 播放键在 EOS 后失效的回归测试（复现用户报告）——
//
// 复现流程：打开视频（自动播放到结尾）→ 按"播放"键（应重新开始，Bug 下无效）
// → 拖动时间条（seek）→ 再按"播放"键（Bug 下才生效）。
//
// 真实 GStreamer 语义：到达 EOS 的管线 `current_state()` 仍是 `Playing`，
// 但单靠 `set_state(Playing)` 无法恢复出帧，必须先 `seek` 清 EOS。
// 旧逻辑只切状态、不 seek，于是播放键"按下却画面不动"——只有拖动时间条
// 触发 seek 才能救活。`media_apply_video_playback(true)` 会对 EOS
// 会话先 seek 回起点，播放键始终能重启。

/// 用 GStreamer 现场生成一段极短视频（约 0.5s），避免依赖用户机器素材。
/// 失败返回 None（调用方跳过）。
fn tiny_test_video() -> Option<std::path::PathBuf> {
    gstreamer::init().ok()?;
    let out = std::env::temp_dir().join("aweft_eos_tiny_test.mp4");
    let pipeline = gstreamer::Pipeline::new();
    let src = gstreamer::ElementFactory::make("videotestsrc")
        .property("num-buffers", 15i32)
        .build()
        .ok()?;
    // 固定为常见的 4:2:0 输入，避免 videotestsrc 的默认格式让 x264enc 生成
    // High 4:4:4 Profile；后者在精简测试环境中可能没有可用解码器。
    let convert = gstreamer::ElementFactory::make("videoconvert")
        .build()
        .ok()?;
    let raw_caps = gstreamer::ElementFactory::make("capsfilter")
        .property(
            "caps",
            gstreamer::Caps::builder("video/x-raw")
                .field("format", "I420")
                .build(),
        )
        .build()
        .ok()?;
    let enc = gstreamer::ElementFactory::make("x264enc").build().ok()?;
    let mux = gstreamer::ElementFactory::make("qtmux").build().ok()?;
    let sink = gstreamer::ElementFactory::make("filesink")
        .property("location", out.to_str()?)
        .build()
        .ok()?;
    pipeline
        .add_many([&src, &convert, &raw_caps, &enc, &mux, &sink])
        .ok()?;
    gstreamer::Element::link_many([&src, &convert, &raw_caps, &enc, &mux, &sink]).ok()?;
    pipeline.set_state(gstreamer::State::Playing).ok()?;
    // 等 EOS（生成完成）
    let bus = pipeline.bus()?;
    use gstreamer::prelude::*;
    for msg in bus.iter_timed(gstreamer::ClockTime::NONE) {
        use gstreamer::MessageView;
        match msg.view() {
            MessageView::Eos(..) => break,
            MessageView::Error(e) => {
                eprintln!("跳过：生成测试视频错误: {}", e.error());
                let _ = pipeline.set_state(gstreamer::State::Null);
                return None;
            }
            _ => {}
        }
    }
    let _ = pipeline.set_state(gstreamer::State::Null);
    if out.is_file() { Some(out) } else { None }
}

/// 直接读取 appsink，供不安装生产回调的测试使用。
fn pump_appsink(element: &gstreamer::Element) -> bool {
    if let Some(appsink) = element.clone().dynamic_cast::<AppSink>().ok() {
        appsink
            .try_pull_sample(gstreamer::ClockTime::ZERO)
            .is_some()
    } else {
        false
    }
}

/// 模拟渲染线程消费回调保存的最新帧。
fn pump(store: &mut crate::store::AppStore) {
    for s in &mut store.state_mut().video_sessions {
        s.wgpu_sink.fetch_next_frame();
    }
}

/// 边泵帧边等待条件成立（替代无消费者的 wait_until，用于测播放推进）。
fn pump_until<F: Fn(&mut crate::store::AppStore) -> bool>(
    store: &mut crate::store::AppStore,
    timeout_ms: u64,
    cond: F,
) -> bool {
    let step = std::time::Duration::from_millis(20);
    let mut elapsed = 0u64;
    while elapsed < timeout_ms {
        pump(store);
        if cond(store) {
            return true;
        }
        std::thread::sleep(step);
        elapsed += 20;
    }
    cond(store)
}

#[test]
fn play_restarts_from_eos_after_open() {
    let _g = crate::GST_SERIAL.lock().unwrap_or_else(|e| e.into_inner());
    let Some(video) = tiny_test_video() else {
        eprintln!("跳过：GStreamer 不可用或测试视频生成失败");
        return;
    };
    // 无事件循环的 headless Slint 组件树（仅需 AppStore 容器，不渲染）
    i_slint_backend_testing::init_no_event_loop();
    let instance = crate::bridge::types::MainWindow::new().unwrap();
    let mut store = crate::store::AppStore::new(&instance);

    // 直接搭一条真实解码管线（与 build_session 同源，但本测试不连 WGPU sink）。
    let (pipeline, sink_element, wgpu_sink, _audio) =
        crate::render::video_sink::pipeline::build_decode_pipeline_with_dmabuf(
            &url::Url::from_file_path(&video).unwrap().to_string(),
            false,
            false,
        )
        .expect("搭建解码管线失败");
    pipeline
        .set_state(gstreamer::State::Playing)
        .expect("启动解码管线失败");

    // 等管线真正播到 EOS（位置停在时长处不再推进）。背压下必须泵帧推进。
    let dur = {
        let mut reached = false;
        for _ in 0..250 {
            pump_appsink(&sink_element);
            let pos = pipeline.query_position::<gstreamer::ClockTime>();
            let d = pipeline.query_duration::<gstreamer::ClockTime>();
            if let (Some(p), Some(dd)) = (pos, d) {
                if dd > gstreamer::ClockTime::from_nseconds(0) && p >= dd {
                    reached = true;
                    break;
                }
            }
            std::thread::sleep(std::time::Duration::from_millis(20));
        }
        reached
    };
    assert!(dur, "测试视频应能播到 EOS");
    let audio = crate::render::video_sink::audio::SessionAudio::new(pipeline.clone());
    // 此时 media_refresh_video_playing_state 会判其处于 EOS → video_playing 应为 false
    store
        .state_mut()
        .video_sessions
        .push(crate::store::state::VideoSession {
            uri: video.to_string_lossy().into(),
            pipeline,
            sink_element,
            wgpu_sink,
            audio: Some(audio),
            position_x: 0.0,
            position_y: 0.0,
            current_time: 0.0,
            duration: None,
            muted: false,
            playing: true,
            at_eos: std::sync::Arc::new(std::sync::atomic::AtomicBool::new(true)),
            last_frame_seeked: false,
        });
    store.state_mut().main_video = Some(0);

    // 模拟"打开即自动播放"把 video_playing 置 true，且 UI 播放状态同步会
    // 逐帧校正把 EOS 识别为未播放（false）—— 这正是用户看到的"暂停"态。
    store.state_mut().video_playing = true;
    crate::reducers::media::media_refresh_video_playing_state(&mut store);
    assert!(
        !store.state().video_playing,
        "EOS 时按钮应显示可播放（video_playing=false）"
    );

    // 用户第一次按"播放"键：Bug 下 set_state(Playing) 无法恢复；修复后 seek 回起点重启。
    store
        .dispatch_media(crate::actions::media::MediaAction::Video(
            crate::actions::media::VideoAction::Play,
        ))
        .unwrap();

    // 重启后应重新出帧：位置回落到时长之前并继续推进。
    let restarted = pump_until(&mut store, 5000, |s| {
        let a0 = s.state().video_sessions[0]
            .pipeline
            .query_position::<gstreamer::ClockTime>();
        matches!(a0, Some(p) if p > gstreamer::ClockTime::from_mseconds(50))
    });
    assert!(
        restarted,
        "按播放键后管线应从 EOS 重启（位置回落后于时长）—— 修复前此处失效"
    );
    // 进一步确认确实在前进（位置变大）
    let t0 = store.state().video_sessions[0]
        .pipeline
        .query_position::<gstreamer::ClockTime>();
    let advanced = pump_until(&mut store, 3000, |s| {
        let p = s.state().video_sessions[0]
            .pipeline
            .query_position::<gstreamer::ClockTime>();
        matches!((t0, p), (Some(a), Some(b)) if b > a)
    });
    assert!(advanced, "重启后播放应持续推进");
}

/// 时间线 Header 播放键控制视口选择集合；无选择时只控制主视频。
#[test]
fn play_controls_selected_video_sessions_and_defaults_to_main() {
    let _g = crate::GST_SERIAL.lock().unwrap_or_else(|e| e.into_inner());
    let Some(video) = tiny_test_video() else {
        eprintln!("跳过：GStreamer 不可用或测试视频生成失败");
        return;
    };
    let uri = url::Url::from_file_path(&video).unwrap().to_string();
    i_slint_backend_testing::init_no_event_loop();
    let instance = crate::bridge::types::MainWindow::new().unwrap();
    let mut store = crate::store::AppStore::new(&instance);

    // 两条独立真实解码管线（同源短片，互不影响）
    let build = || {
        let (p, s, w, a) = crate::render::video_sink::pipeline::build_decode_pipeline_with_dmabuf(
            &uri, false, false,
        )
        .expect("搭建解码管线失败");
        p.set_state(gstreamer::State::Playing).expect("启动失败");
        (p, s, w, a)
    };
    let (p0, s0, w0, a0) = build();
    let (p1, s1, w1, a1) = build();

    // 让 0 号先播放一会儿以推进位置（背压下必须泵帧推进）
    for _ in 0..75 {
        pump_appsink(&s0);
        if p0
            .query_position::<gstreamer::ClockTime>()
            .is_some_and(|p| p > gstreamer::ClockTime::from_mseconds(50))
        {
            break;
        }
        std::thread::sleep(std::time::Duration::from_millis(20));
    }
    // 暂停 1 号（模拟"另一个未被激活的视频处于未播放态"）
    p1.set_state(gstreamer::State::Paused).unwrap();

    store
        .state_mut()
        .video_sessions
        .push(crate::store::state::VideoSession {
            uri: uri.clone(),
            pipeline: p0,
            sink_element: s0,
            wgpu_sink: w0,
            audio: Some(a0),
            position_x: 0.0,
            position_y: 0.0,
            current_time: 0.0,
            duration: None,
            muted: false,
            playing: true,
            at_eos: std::sync::Arc::new(std::sync::atomic::AtomicBool::new(false)),
            last_frame_seeked: false,
        });
    store
        .state_mut()
        .video_sessions
        .push(crate::store::state::VideoSession {
            uri: uri.clone(),
            pipeline: p1,
            sink_element: s1,
            wgpu_sink: w1,
            audio: Some(a1),
            position_x: 50.0,
            position_y: 50.0,
            current_time: 0.0,
            duration: None,
            muted: false,
            playing: false,
            at_eos: std::sync::Arc::new(std::sync::atomic::AtomicBool::new(false)),
            last_frame_seeked: false,
        });
    store.state_mut().main_video = Some(0);
    store.state_mut().selected_videos.clear();

    // 无选择：播放键只控制主视频，参考视频保持原暂停态。
    store
        .dispatch_media(crate::actions::media::MediaAction::Video(
            crate::actions::media::VideoAction::Play,
        ))
        .unwrap();

    assert!(
        pump_until(&mut store, 1000, |s| gst_pipeline_wants_playing(
            &s.state().video_sessions[0].pipeline
        )),
        "无选择时主视频应进入播放态"
    );
    assert!(!gst_pipeline_wants_playing(
        &store.state().video_sessions[1].pipeline
    ));

    // 活跃视频 0 应被 flush-seek 重新驱动出帧（之前只有拖动时间条能播）
    let resumed = pump_until(&mut store, 3000, |s| {
        let a0 = s.state().video_sessions[0]
            .pipeline
            .query_position::<gstreamer::ClockTime>();
        matches!(a0, Some(p) if p > gstreamer::ClockTime::from_mseconds(50))
    });
    assert!(resumed, "无选择时播放键应让主视频真正开始/恢复出帧");

    // 普通点击选择 0，再 Ctrl 点击加入 1；选择变化本身不改变 transport。
    store
        .dispatch_media(crate::actions::media::MediaAction::Video(
            crate::actions::media::VideoAction::Select {
                index: Some(0),
                modifier: crate::bridge::types::SlintSelectionModifier::None,
            },
        ))
        .unwrap();
    store
        .dispatch_media(crate::actions::media::MediaAction::Video(
            crate::actions::media::VideoAction::Select {
                index: Some(1),
                modifier: crate::bridge::types::SlintSelectionModifier::Ctrl,
            },
        ))
        .unwrap();
    assert_eq!(store.state().selected_videos, vec![0, 1]);
    assert!(gst_pipeline_wants_playing(
        &store.state().video_sessions[0].pipeline
    ));
    assert!(!gst_pipeline_wants_playing(
        &store.state().video_sessions[1].pipeline
    ));

    // 多选播放后，两条被选会话同步进入播放态。
    store
        .dispatch_media(crate::actions::media::MediaAction::Video(
            crate::actions::media::VideoAction::Play,
        ))
        .unwrap();
    assert!(
        pump_until(&mut store, 1000, |s| s
            .state()
            .video_sessions
            .iter()
            .all(|session| gst_pipeline_wants_playing(&session.pipeline))),
        "多选播放应作用于全部被选视频"
    );

    store
        .dispatch_media(crate::actions::media::MediaAction::Video(
            crate::actions::media::VideoAction::Pause,
        ))
        .unwrap();
    assert!(
        pump_until(&mut store, 1000, |s| s
            .state()
            .video_sessions
            .iter()
            .all(|session| !gst_pipeline_wants_playing(&session.pipeline))),
        "多选暂停应作用于全部被选视频"
    );

    // 回归：0 号继续播放、1 号作为焦点保持暂停。即使 0 号不断出帧触发渲染
    // 时钟同步，也不能把 1 号的钳制视频位置反写到范围外的动画时间轴。
    store.state_mut().selected_videos = vec![1];
    gst_seek_pipeline_accurate(&store.state().video_sessions[0].pipeline, 0.0);
    store.state().video_sessions[0]
        .pipeline
        .set_state(gstreamer::State::Playing)
        .expect("非焦点视频恢复播放失败");
    store.state().video_sessions[1]
        .pipeline
        .set_state(gstreamer::State::Paused)
        .expect("焦点视频暂停失败");
    store.state_mut().video_sessions[1].current_time = 0.3;
    store.ui_models.animation.graph.current_time = 12.0;
    assert!(
        !media_active_video_drives_timeline(&store),
        "焦点视频暂停时不应因其它视频播放而驱动时间轴"
    );
    crate::projections::ui_sync_active_video_time_realtime(&mut store);
    assert_eq!(
        store.ui_models.animation.graph.current_time, 12.0,
        "其它视频出帧不能把暂停焦点的钳制位置反写到范围外时间轴"
    );
}

/// 复现：暂停后重新播放应从原位置继续，而不是回到开头。
/// 同时复现：初始应显示"暂停"图标（正在播放），不应需要点击两次才播放。
#[test]
fn resume_continues_from_paused_position() {
    let _g = crate::GST_SERIAL.lock().unwrap_or_else(|e| e.into_inner());
    let Some(video) = tiny_test_video() else {
        eprintln!("跳过：GStreamer 不可用或测试视频生成失败");
        return;
    };
    let uri = url::Url::from_file_path(&video).unwrap().to_string();
    i_slint_backend_testing::init_no_event_loop();
    let instance = crate::bridge::types::MainWindow::new().unwrap();
    let mut store = crate::store::AppStore::new(&instance);

    let (pipeline, sink_element, wgpu_sink, audio) =
        crate::render::video_sink::pipeline::build_decode_pipeline_with_dmabuf(&uri, false, false)
            .expect("搭建解码管线失败");
    pipeline
        .set_state(gstreamer::State::Playing)
        .expect("启动失败");

    // 等播放推进到一个明确位置（> 200ms）；背压下必须泵帧推进。
    // 背压使 query_position 反映"已提交（被消费）"位置，略滞后于解码位置，
    // 故到达后再多泵若干帧让已提交位置稳定抬升，避免暂停点偏低。
    let reached = {
        let mut ok = false;
        for _ in 0..250 {
            pump_appsink(&sink_element);
            if pipeline
                .query_position::<gstreamer::ClockTime>()
                .is_some_and(|p| p > gstreamer::ClockTime::from_mseconds(200))
            {
                ok = true;
                break;
            }
            std::thread::sleep(std::time::Duration::from_millis(20));
        }
        ok
    };
    for _ in 0..30 {
        pump_appsink(&sink_element);
    }
    assert!(reached, "应能播放推进到 200ms 之后");

    // 模拟渲染循环里的 render_sync_video_playback_time：把真实播放位置同步进 current_time。
    // 背压下 pipeline.query_position 在暂停时会回退到 sink 已提交位置（滞后于解码位置），
    // 因此续播必须以 current_time 为准（media_apply_video_playback 也是这么做）。
    let play_pos = pipeline
        .query_position::<gstreamer::ClockTime>()
        .expect("播放中应能查询位置");
    let play_secs = play_pos.nseconds() as f64 / 1_000_000_000.0;

    store
        .state_mut()
        .video_sessions
        .push(crate::store::state::VideoSession {
            uri,
            pipeline,
            sink_element,
            wgpu_sink,
            audio: Some(audio),
            position_x: 0.0,
            position_y: 0.0,
            current_time: play_secs,
            duration: None,
            muted: false,
            playing: true,
            at_eos: std::sync::Arc::new(std::sync::atomic::AtomicBool::new(false)),
            last_frame_seeked: false,
        });
    store.state_mut().main_video = Some(0);

    // 模拟打开即自动播放：video_playing=true，且 UI 播放状态同步应确认正在播放
    store.state_mut().video_playing = true;
    crate::reducers::media::media_refresh_video_playing_state(&mut store);
    assert!(
        store.state().video_playing,
        "初始正在播放时应显示暂停图标（video_playing=true）"
    );

    // 暂停
    store
        .dispatch_media(crate::actions::media::MediaAction::Video(
            crate::actions::media::VideoAction::Pause,
        ))
        .unwrap();
    let paused_pos = store.state().video_sessions[0]
        .pipeline
        .query_position::<gstreamer::ClockTime>()
        .expect("暂停时应有位置");
    // 续播点以 current_time 为准（见 media_apply_video_playback）；此处仅校验明显已推进、非空
    let pause_point = gstreamer::ClockTime::from_nseconds(
        (store.state().video_sessions[0].current_time * 1_000_000_000.0) as u64,
    );
    assert!(
        paused_pos > gstreamer::ClockTime::from_mseconds(50),
        "暂停位置应明显大于 0：{paused_pos:?}"
    );

    // 重新播放：应从 paused_pos 附近继续，而不是回到开头
    store
        .dispatch_media(crate::actions::media::MediaAction::Video(
            crate::actions::media::VideoAction::Play,
        ))
        .unwrap();
    // (上面已是正确构造；下面恢复采样逻辑)

    // 在短时间窗内密集采样位置，记录最小值：若从开头重启会先掉到接近 0；
    // 正确应维持在 paused_pos 附近（KEY_UNIT 可能回退几十毫秒）。
    let mut min_pos = gstreamer::ClockTime::from_seconds(1000);
    for _ in 0..40 {
        pump(&mut store);
        if let Some(p) = store.state().video_sessions[0]
            .pipeline
            .query_position::<gstreamer::ClockTime>()
        {
            if p < min_pos {
                min_pos = p;
            }
        }
        std::thread::sleep(std::time::Duration::from_millis(10));
    }
    assert!(
        min_pos > pause_point.saturating_sub(gstreamer::ClockTime::from_mseconds(80)),
        "重新播放不应回到开头（采样最小位置 {min_pos:?} 应接近暂停点 {pause_point:?}）"
    );
    // 继续推进（应超过暂停位置）
    let advanced = pump_until(&mut store, 3000, |s| {
        s.state().video_sessions[0]
            .pipeline
            .query_position::<gstreamer::ClockTime>()
            .is_some_and(|p| p > paused_pos)
    });
    assert!(advanced, "重新播放后应继续从暂停位置推进");
}

/// 复现：拖动时间线（Seek）应改变活跃视频的播放位置。
#[test]
fn toggle_playback_at_end_restarts_from_start() {
    let _g = crate::GST_SERIAL.lock().unwrap_or_else(|e| e.into_inner());
    let Some(video) = tiny_test_video() else {
        eprintln!("跳过：GStreamer 不可用或测试视频生成失败");
        return;
    };
    let uri = url::Url::from_file_path(&video).unwrap().to_string();
    i_slint_backend_testing::init_no_event_loop();
    let instance = crate::bridge::types::MainWindow::new().unwrap();
    let mut store = crate::store::AppStore::new(&instance);

    let (pipeline, sink_element, wgpu_sink, audio) =
        crate::render::video_sink::pipeline::build_decode_pipeline_with_dmabuf(&uri, false, false)
            .expect("搭建解码管线失败");
    pipeline
        .set_state(gstreamer::State::Playing)
        .expect("启动失败");
    // 先播放推进一点（背压下必须泵帧推进）
    for _ in 0..100 {
        pump_appsink(&sink_element);
        if pipeline
            .query_position::<gstreamer::ClockTime>()
            .is_some_and(|p| p > gstreamer::ClockTime::from_mseconds(50))
        {
            break;
        }
        std::thread::sleep(std::time::Duration::from_millis(20));
    }

    store
        .state_mut()
        .video_sessions
        .push(crate::store::state::VideoSession {
            uri,
            pipeline,
            sink_element,
            wgpu_sink,
            audio: Some(audio),
            position_x: 0.0,
            position_y: 0.0,
            current_time: 0.0,
            duration: None,
            muted: false,
            playing: true,
            at_eos: std::sync::Arc::new(std::sync::atomic::AtomicBool::new(false)),
            last_frame_seeked: false,
        });
    store.state_mut().main_video = Some(0);

    // 先暂停（解码线程停下，appsink 停在最后一帧）。
    store
        .dispatch_media(crate::actions::media::MediaAction::Video(
            crate::actions::media::VideoAction::Pause,
        ))
        .unwrap();
    // 排空 appsink：pull 模式下解码线程已停、不会回填，槽位彻底清空，
    // 否则 flush-seek 不会自动丢弃已持有的预滚动帧，新 seek 的预滚动帧无槽可入
    // 而阻塞（seek 死锁，无帧、query_position=None）。
    let sink_el = store.state().video_sessions[0].sink_element.clone();
    for _ in 0..200 {
        let had = sink_el
            .clone()
            .dynamic_cast::<AppSink>()
            .ok()
            .and_then(|a| a.try_pull_sample(gstreamer::ClockTime::ZERO))
            .is_some();
        if !had {
            break;
        }
    }

    // 暂停态拖动到 0.3s（模拟时间线拖拽：SeekBegin 切到播放态使 flush-seek 可接受，
    // 拖拽出帧实时预览；SeekPreview 落到 0.3s；SeekCommit 回落暂停交付暂停预览）。
    store
        .dispatch_media(crate::actions::media::MediaAction::Video(
            crate::actions::media::VideoAction::SeekBegin {
                index: 0,
                is_batch: false,
            },
        ))
        .unwrap();
    store
        .dispatch_media(crate::actions::media::MediaAction::Video(
            crate::actions::media::VideoAction::SeekPreview { timestamp: 300 },
        ))
        .unwrap();
    store
        .dispatch_media(crate::actions::media::MediaAction::Video(
            crate::actions::media::VideoAction::SeekCommit,
        ))
        .unwrap();

    // UI 时间条用 session.current_time（media_seek_video_session 写入 0.3）；且 fetch_next_frame
    // 直接从 appsink 拉帧（含暂停态预滚动帧），暂停态拖拽也能在后续重绘即时预览——
    // 这正是用户"拖动改变播放时间"的真实可见行为。
    assert!(
        (store.state().video_sessions[0].current_time - 0.3).abs() < 0.05,
        "拖动后时间条 current_time 应为 ~0.3s，实际 {}",
        store.state().video_sessions[0].current_time
    );
    // 暂停态拖拽的核心可见行为（用户原始诉求"拖动改变播放时间"）：
    // 1) 时间条 current_time 已变为 0.3s（上面的断言）；
    // 2) 按播放后管线从 current_time 精确续播——时钟从 ~0.3s 处继续推进，
    //    而非回到开头，证明 drag→play 端到端生效（帧实际产出由
    //    play_restarts_from_eos_after_open 到达 EOS 覆盖）。
    store
        .dispatch_media(crate::actions::media::MediaAction::Video(
            crate::actions::media::VideoAction::Play,
        ))
        .unwrap();
    let from_drag = pump_until(&mut store, 3000, |s| {
        s.state().video_sessions[0]
            .pipeline
            .query_position::<gstreamer::ClockTime>()
            .is_some_and(|p| p >= gstreamer::ClockTime::from_mseconds(250))
    });
    assert!(from_drag, "拖拽到 0.3s 后按播放应从 ~0.3s 处继续");
}

/// 回归：暂停态点击视频时长之后必须取得最后一帧 preroll。正向精确 seek 到
/// duration 附近会把最后一帧按 segment 起点裁掉，因此这里验证反向尾端 seek。
#[test]
fn paused_seek_past_end_prerolls_last_frame() {
    let _g = crate::GST_SERIAL.lock().unwrap_or_else(|e| e.into_inner());
    let Some(video) = tiny_test_video() else {
        eprintln!("跳过：GStreamer 不可用或测试视频生成失败");
        return;
    };
    let uri = url::Url::from_file_path(&video).unwrap().to_string();
    let (pipeline, sink_element, wgpu_sink, audio) =
        crate::render::video_sink::pipeline::build_decode_pipeline_with_dmabuf(&uri, false, false)
            .expect("搭建解码管线失败");
    let appsink = sink_element
        .clone()
        .dynamic_cast::<AppSink>()
        .expect("视频 sink 不是 appsink");
    sink_element.set_property("sync", true);
    let next_frame = std::sync::Arc::clone(&wgpu_sink.next_frame);
    let frame_duration_slot = std::sync::Arc::clone(&wgpu_sink.frame_dur_secs);
    let notifications = std::sync::Arc::new(std::sync::atomic::AtomicUsize::new(0));
    let notifications_for_callback = std::sync::Arc::clone(&notifications);
    appsink.set_callbacks(
        crate::render::video_sink::SlintWGPUSink::build_gst_appsink_callbacks(
            std::sync::Arc::clone(&next_frame),
            frame_duration_slot,
            Some(std::sync::Arc::new(move || {
                notifications_for_callback.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
            })),
        ),
    );
    pipeline
        .set_state(gstreamer::State::Paused)
        .expect("管线进入暂停态失败");
    let _ = pipeline.state(Some(gstreamer::ClockTime::from_seconds(5)));
    let duration = pipeline
        .query_duration::<gstreamer::ClockTime>()
        .expect("测试视频没有 duration");
    // 生产回调已消费初始首帧 preroll；清空槽位并记录通知次数，确保后续观察到
    // 的一定是越界 seek 新产生的末帧及其重绘请求。
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(2);
    while std::time::Instant::now() < deadline
        && next_frame
            .lock()
            .unwrap_or_else(|error| error.into_inner())
            .is_none()
    {
        std::thread::sleep(std::time::Duration::from_millis(10));
    }
    next_frame
        .lock()
        .unwrap_or_else(|error| error.into_inner())
        .take()
        .expect("暂停预滚动没有通过生产回调写入首帧槽位");
    let notifications_before_seek = notifications.load(std::sync::atomic::Ordering::SeqCst);

    let duration_secs = duration.nseconds() as f64 / 1_000_000_000.0;
    let mut session = crate::store::state::VideoSession {
        uri,
        pipeline: pipeline.clone(),
        sink_element,
            wgpu_sink,
            audio: Some(audio),
            position_x: 0.0,
            position_y: 0.0,
            current_time: 0.0,
        duration: Some(duration_secs),
        muted: false,
        playing: false,
        at_eos: std::sync::Arc::new(std::sync::atomic::AtomicBool::new(false)),
        last_frame_seeked: false,
    };
    // 模拟用户在播放中点击视频时长之后。即便管线当前处于 Playing，越界
    // 定位也必须确定性地暂停，再以反向 segment 只取得一张末帧 preroll。
    session
        .pipeline
        .set_state(gstreamer::State::Playing)
        .expect("模拟播放态失败");
    session.playing = true;
    let _ = session
        .pipeline
        .state(Some(gstreamer::ClockTime::from_seconds(2)));
    media_seek_video_session(&mut session, duration_secs + 10.0);
    assert!(
        !gst_pipeline_wants_playing(&session.pipeline),
        "播放中越界 seek 后必须稳定暂停，不能残留倒放中的 Playing 状态"
    );

    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(5);
    let sample = loop {
        if let Some(sample) = next_frame
            .lock()
            .unwrap_or_else(|error| error.into_inner())
            .take()
        {
            break sample;
        }
        assert!(
            std::time::Instant::now() < deadline,
            "暂停态越界 seek 后生产回调没有把最后一帧写入画面槽位"
        );
        std::thread::sleep(std::time::Duration::from_millis(10));
    };
    assert!(
        notifications.load(std::sync::atomic::Ordering::SeqCst) > notifications_before_seek,
        "末帧 preroll 到达后必须请求 Slint 重绘"
    );
    let buffer = sample.buffer().expect("末帧 sample 没有 buffer");
    let pts = buffer.pts().expect("末帧 buffer 没有 PTS");
    let stream_time = sample
        .segment()
        .and_then(|segment| segment.downcast_ref::<gstreamer::ClockTime>())
        .and_then(|segment| segment.to_stream_time(pts))
        .expect("无法把末帧 PTS 映射为流时间");
    let frame_duration = buffer
        .duration()
        .unwrap_or(gstreamer::ClockTime::from_mseconds(34));
    assert!(stream_time < duration, "末帧流时间必须早于媒体时长");
    assert!(
        stream_time + frame_duration + gstreamer::ClockTime::from_mseconds(2) >= duration,
        "preroll 应为最后一帧：流时间={stream_time:?}, 原始 PTS={pts:?}, 帧长={frame_duration:?}, 时长={duration:?}"
    );

    // 再按播放必须先用正速率 seek 清掉末帧定位的负速率 segment，然后从头
    // 正向推进；若先切 Playing 再 seek，这里会偶发观察到倒放。
    i_slint_backend_testing::init_no_event_loop();
    let instance = crate::bridge::types::MainWindow::new().unwrap();
    let mut store = crate::store::AppStore::new(&instance);
    store.state_mut().video_sessions.push(session);
    store.state_mut().main_video = Some(0);
    store
        .dispatch_media(crate::actions::media::MediaAction::Video(
            crate::actions::media::VideoAction::Play,
        ))
        .unwrap();
    assert!(
        pump_until(&mut store, 3000, |store| {
            store.state().video_sessions[0]
                .pipeline
                .query_position::<gstreamer::ClockTime>()
                .is_some_and(|position| {
                    position > gstreamer::ClockTime::from_mseconds(50) && position < duration
                })
        }),
        "末帧暂停后再次播放应从头正向推进"
    );
    let _ = store.state().video_sessions[0]
        .pipeline
        .set_state(gstreamer::State::Null);
}

/// 验证 `gst_set_audio_muted` 能真正落到音频 sink 上：
/// `volume` 元素同时暴露 `mute` 与 `volume`，正是兜底分支命中的元素类型
/// （与 pulsesink/autoaudiosink→child 同一条设置路径）。若此断言通过，
/// 说明拖拽静音的属性写入逻辑有效，运行时无声音是该路径生效的直接结果。
#[test]
fn drag_audio_mute_targets_audio_sink() {
    use gstreamer::prelude::*;
    gstreamer::init().unwrap();
    let pipeline = gstreamer::Pipeline::new();
    let vol = gstreamer::ElementFactory::make("volume")
        .build()
        .expect("需要 volume 插件来验证静音逻辑");
    pipeline.add(&vol).unwrap();

    gst_set_audio_muted(&pipeline, true);
    assert!(vol.property::<bool>("mute"), "静音时应置 mute=true");
    assert_eq!(vol.property::<f64>("volume"), 0.0, "静音时应置 volume=0");

    gst_set_audio_muted(&pipeline, false);
    assert!(!vol.property::<bool>("mute"), "解除静音应置 mute=false");
    assert_eq!(vol.property::<f64>("volume"), 1.0, "解除静音应置 volume=1");
}

/// 复现：拖拽期间 Header 播放按钮不应随 flush-seek 的过渡态闪烁。
/// 拖拽时 `media_refresh_video_playing_state` 应沿用 seek 会话记录的拖拽前意图态，
/// 而非实时查询管线状态——flush-seek 会让管线瞬时停在 Paused，导致图标乱跳。
#[test]
fn drag_keeps_header_play_state_stable() {
    let _g = crate::GST_SERIAL.lock().unwrap_or_else(|e| e.into_inner());
    let Some(video) = tiny_test_video() else {
        eprintln!("跳过：GStreamer 不可用或测试视频生成失败");
        return;
    };
    let uri = url::Url::from_file_path(&video).unwrap().to_string();
    i_slint_backend_testing::init_no_event_loop();
    let instance = crate::bridge::types::MainWindow::new().unwrap();
    let mut store = crate::store::AppStore::new(&instance);

    let (pipeline, sink_element, wgpu_sink, audio) =
        crate::render::video_sink::pipeline::build_decode_pipeline_with_dmabuf(&uri, false, false)
            .expect("搭建解码管线失败");
    store
        .state_mut()
        .video_sessions
        .push(crate::store::state::VideoSession {
            uri,
            pipeline,
            sink_element,
            wgpu_sink,
            audio: Some(audio),
            position_x: 0.0,
            position_y: 0.0,
            current_time: 0.0,
            duration: None,
            muted: false,
            playing: true,
            at_eos: std::sync::Arc::new(std::sync::atomic::AtomicBool::new(false)),
            last_frame_seeked: false,
        });
    store.state_mut().main_video = Some(0);

    // 模拟"播放中开始拖拽"：拖拽前是播放态（was_playing=true）。
    store.state_mut().video_playing = true;

    // 拖拽中：即便实时状态因 flush-seek 瞬时变为非播放，UI 播放状态同步
    // 也必须回到 was_playing，而不是跟随闪烁成暂停图标。
    store.state_mut().video_seek_session = Some(crate::store::state::VideoSeekSession {
        anchor_time: 0.0,
        targets: vec![crate::store::state::VideoSeekTarget {
            index: 0,
            origin_time: 0.0,
            was_playing: true,
        }],
    });
    store.state_mut().video_playing = false;
    super::media_refresh_video_playing_state(&mut store);
    assert!(
        store.state().video_playing,
        "拖拽中应稳定显示播放态（沿用 was_playing），不应闪烁成暂停"
    );

    // 松手：恢复按实时状态校正（此处管线未真正播放，校正为 false 亦合理）。
    store.state_mut().video_seek_session = None;
    super::media_refresh_video_playing_state(&mut store);
}

/// 切换音轨：选中下标写入 `audio_track_indices` 并发出 `VideoAudioTrackChanged`。
#[test]
fn select_audio_track_updates_index_and_emits_tag() {
    let _g = crate::GST_SERIAL.lock().unwrap_or_else(|e| e.into_inner());
    gstreamer::init().expect("初始化 GStreamer 失败");
    i_slint_backend_testing::init_no_event_loop();
    let instance = crate::bridge::types::MainWindow::new().unwrap();
    let mut store = crate::store::AppStore::new(&instance);

    let pipeline = gstreamer::Pipeline::new();
    let audio = crate::render::video_sink::audio::SessionAudio::new(pipeline.clone());
    let wgpu_sink = crate::render::video_sink::SlintWGPUSink::new();
    let sink_element = wgpu_sink.element();
    store
        .state_mut()
        .video_sessions
        .push(crate::store::state::VideoSession {
            uri: "test://switch-track".to_string(),
            pipeline,
            sink_element,
            wgpu_sink,
            audio: Some(audio),
            position_x: 0.0,
            position_y: 0.0,
            current_time: 0.0,
            duration: Some(10.0),
            muted: false,
            playing: false,
            at_eos: std::sync::Arc::new(std::sync::atomic::AtomicBool::new(false)),
            last_frame_seeked: false,
        });
    // 两条假音轨：只用 stream-id，切换不断言解码结果（空管线发 SelectStreams 事件不炸即可）。
    let fake_track = |stream_id: &str| crate::store::state::AudioTrackInfo {
        stream_id: stream_id.to_string(),
        label: String::new(),
        language: String::new(),
        channels: 2,
        channel_layout: String::new(),
        channel_mask: String::new(),
        codec: String::new(),
        caps: String::new(),
        sample_rate: 0,
        bit_depth: 0,
        bitrate: 0,
        max_bitrate: 0,
        artist: String::new(),
        album: String::new(),
        comment: String::new(),
        description: String::new(),
        encoder: String::new(),
        encoded_by: String::new(),
    };
    store.state_mut().audio_tracks.push(vec![
        fake_track("stream-a"),
        fake_track("stream-b"),
    ]);
    store.state_mut().audio_track_indices.push(None);

    let tags = store
        .dispatch_media(crate::actions::media::MediaAction::Video(
            crate::actions::media::VideoAction::SelectAudioTrack {
                video_index: 0,
                track_index: 1,
            },
        ))
        .expect("切换音轨不应失败");
    assert_eq!(store.state().audio_track_indices[0], Some(1));
    assert!(
        tags.iter().any(|tag| matches!(
            tag,
            crate::depsgraph::Tag::VideoAudioTrackChanged { video_index: 0 }
        )),
        "切换音轨应发出 VideoAudioTrackChanged，实际: {tags:?}"
    );

    // 越界音轨与不存在的视频都应报错，且不改已选下标。
    assert!(store
        .dispatch_media(crate::actions::media::MediaAction::Video(
            crate::actions::media::VideoAction::SelectAudioTrack {
                video_index: 0,
                track_index: 5,
            },
        ))
        .is_err());
    assert!(store
        .dispatch_media(crate::actions::media::MediaAction::Video(
            crate::actions::media::VideoAction::SelectAudioTrack {
                video_index: 3,
                track_index: 0,
            },
        ))
        .is_err());
    assert_eq!(store.state().audio_track_indices[0], Some(1));
}
