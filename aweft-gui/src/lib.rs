pub mod screen;
pub mod slint_video_sink;
pub mod types;
mod wgpu_subtitle_renderer;

use std::collections::{HashMap, HashSet};
use std::{cell::RefCell, rc::Rc};

use anyhow::{Context, Result};
use gstreamer::prelude::*;
use screen::area::ScrAreaKey;
use screen::map::ScrAreaMap;
use screen::primitives::{Adjacency, Rcti};
use screen::vertex::ScrVertKey;
use slint::wgpu_24::{WGPUConfiguration, WGPUSettings};
use slint::{
    Model, PhysicalSize, RenderingState,
    winit_030::{WinitWindowAccessor, WinitWindowEventResult, winit},
};
use types::*;

/// 内部边数据结构，包含计算出的所有信息
struct EdgeSegmentData {
    x: f32,
    y: f32,
    width: f32,
    height: f32,
    vert_keys: Vec<SlintKey>,
}

/// 拖拽操作的状态信息
#[derive(Debug, Clone)]
pub enum DragOperation {
    /// 无拖拽操作
    None,
    /// 分割操作 - 存储分割的源区域和预览数据
    Split {
        source_area: ScrAreaKey,
        preview_data: SplitPreviewOverlayData,
    },
    /// 合并操作 - 存储源区域、目标区域和邻接关系
    Join {
        source_area_key: ScrAreaKey,
        target_area_key: ScrAreaKey,
        adjacency: Adjacency,
    },
    /// 边移动操作 - 存储移动状态信息
    MoveEdge {
        /// 轴方向：true=水平轴(改变Y坐标)，false=垂直轴(改变X坐标)
        dir_axis_horizontal: bool,
        /// 原始轴坐标值
        origval: u32,
        /// 向正方向的最大位移距离
        bigger: u32,
        /// 向负方向的最大位移距离
        smaller: u32,
        /// 参与移动的顶点键集合
        moving_vert_keys: Vec<ScrVertKey>,
    },
}

/// 应用状态和逻辑的唯一管理者
pub struct AppState {
    pub screen_map: ScrAreaMap,
    pub ui_settings: UISettings,
    pub window_size: PhysicalSize,
    pub initialized: bool,
    /// 当前拖拽操作状态
    pub current_drag_operation: DragOperation,
    /// 保持 GStreamer 管道生命周期，防止被提前释放
    pub video_pipeline: Option<gstreamer::Pipeline>,
    /// 保持 Sink 生命周期
    pub video_sink: Option<gstreamer::Element>,
    /// WGPU 视频 Sink 实例
    pub wgpu_sink: Option<slint_video_sink::SlintWGPUSink>,
    /// 字幕渲染器
    pub subtitle_renderer: Option<wgpu_subtitle_renderer::SubtitleRenderer>,
    /// libass 字幕渲染器
    pub libass_renderer: Option<aweft_libass::SubtitleRenderer>,
}
impl AppState {
    /// 构造函数，从 Slint 窗口初始化
    pub fn new(window: &slint::Window) -> Self {
        Self {
            screen_map: ScrAreaMap::new(),
            ui_settings: UISettings::from_slint_window(window),
            window_size: window.size(),
            initialized: false,
            current_drag_operation: DragOperation::None,
            video_pipeline: None,
            video_sink: None,
            wgpu_sink: None,
            subtitle_renderer: None,
            libass_renderer: None,
        }
    }

    /// 将内部区域数据转换为 Slint 需要的格式
    /// 这个函数现在是 AppState 的一部分，可以直接访问其字段，无需加锁
    fn convert_areas_to_slint_data(&self) -> Vec<ScrAreaData> {
        let mut areas_data = Vec::new();
        let scale_factor = self.ui_settings.get_scale_factor();
        let actionzone_side_length = self.ui_settings.get_actionzone_side_length() as f32;

        for (area_key, area) in &self.screen_map.areas {
            let (geometry_rect, winx, winy) = area.geometry();

            // 物理像素 -> 逻辑像素转换
            let logical_x = geometry_rect.xmin as f64 / scale_factor;
            let logical_y = geometry_rect.ymin as f64 / scale_factor;
            let logical_width = winx as f64 / scale_factor;
            let logical_height = winy as f64 / scale_factor;

            areas_data.push(ScrAreaData {
                area_key: key_to_slint_area_key(area_key),
                x: logical_x as f32,
                y: logical_y as f32,
                width: logical_width as f32,
                height: logical_height as f32,
                actionzone_side_length,
            });
        }
        areas_data
    }

    fn convert_edges_to_slint_display_data(&self) -> Vec<ScrEdgeDisplayData> {
        self.compute_edges_data()
            .into_iter()
            .map(|edge| ScrEdgeDisplayData {
                x: edge.x,
                y: edge.y,
                width: edge.width,
                height: edge.height,
                active: false, // 默认为非激活状态，后续可通过回调更新
            })
            .collect()
    }

    /// 统一的边数据计算函数，避免重复计算
    fn compute_edges_data(&self) -> Vec<EdgeSegmentData> {
        // 1) 先将所有非边框的、轴对齐的边转换为"线段"并去重
        // 线段键: (is_horizontal, constant_coord, start, end)
        let mut seen_segments: HashSet<(bool, u32, u32, u32)> = HashSet::new();
        // 分组为: 方向/常量坐标 -> [(start, end, start_vert_key, end_vert_key)]
        let mut grouped_segments: HashMap<(bool, u32), Vec<(u32, u32, ScrVertKey, ScrVertKey)>> =
            HashMap::new();

        for (edge_key, edge) in &self.screen_map.edges {
            if edge.border {
                continue;
            }
            let v1 =
                self.screen_map.verts.get(edge.v1).unwrap_or_else(|| {
                    panic!("Edge: {:#?} 存在无效Vert: {:#?}", edge_key, edge.v1)
                });
            let v2 =
                self.screen_map.verts.get(edge.v2).unwrap_or_else(|| {
                    panic!("Edge: {:#?} 存在无效Vert: {:#?}", edge_key, edge.v2)
                });

            let is_horizontal = v1.vec.y == v2.vec.y;
            let is_vertical = v1.vec.x == v2.vec.x;

            match (is_horizontal, is_vertical) {
                (true, false) => {
                    let y = v1.vec.y; // 恒定y
                    let (x1, x2, ks, ke) = if v1.vec.x <= v2.vec.x {
                        (v1.vec.x, v2.vec.x, edge.v1, edge.v2)
                    } else {
                        (v2.vec.x, v1.vec.x, edge.v2, edge.v1)
                    };
                    // 去重：完全一致的段只保留一次
                    if seen_segments.insert((true, y, x1, x2)) {
                        grouped_segments
                            .entry((true, y))
                            .or_default()
                            .push((x1, x2, ks, ke));
                    }
                }
                (false, true) => {
                    let x = v1.vec.x; // 恒定x
                    let (y1, y2, ks, ke) = if v1.vec.y <= v2.vec.y {
                        (v1.vec.y, v2.vec.y, edge.v1, edge.v2)
                    } else {
                        (v2.vec.y, v1.vec.y, edge.v2, edge.v1)
                    };
                    if seen_segments.insert((false, x, y1, y2)) {
                        grouped_segments
                            .entry((false, x))
                            .or_default()
                            .push((y1, y2, ks, ke));
                    }
                }
                (false, false) => {
                    println!("调试信息 - 非轴对齐边:");
                    println!("  Edge key: {:?}", edge_key);
                    println!(
                        "  v1 key: {:?}, 位置: ({}, {})",
                        edge.v1, v1.vec.x, v1.vec.y
                    );
                    println!(
                        "  v2 key: {:?}, 位置: ({}, {})",
                        edge.v2, v2.vec.x, v2.vec.y
                    );
                    println!("  边是否为边框: {}", edge.border);
                    panic!(
                        "存在非水平或垂直 Edge 数据: edge_key={:?}, v1=({}, {}), v2=({}, {})",
                        edge_key, v1.vec.x, v1.vec.y, v2.vec.x, v2.vec.y
                    );
                }
                (true, true) => {
                    panic!("Edge 的两个顶点重合，长度为零: edge_key={:?}", edge_key);
                }
            }
        }

        // 2) 在每个组内按范围合并相邻/重叠的段，得到最大连续段
        let half_edge_width = self.ui_settings.get_half_edge_width();
        let scale_factor = self.ui_settings.get_scale_factor();

        let mut result: Vec<EdgeSegmentData> = Vec::new();

        for ((is_horizontal, constant), mut spans) in grouped_segments {
            // 按起点排序，次序用终点保证稳定
            spans.sort_by(|a, b| match a.0.cmp(&b.0) {
                std::cmp::Ordering::Equal => a.1.cmp(&b.1),
                other => other,
            });

            let mut merged: Vec<(u32, u32, Vec<ScrVertKey>)> = Vec::new();
            let mut current: Option<(u32, u32, Vec<ScrVertKey>)> = None;

            for (start, end, ks, ke) in spans {
                if let Some((cur_s, mut cur_e, mut keys)) = current {
                    use std::cmp::Ordering;
                    match start.cmp(&cur_e) {
                        Ordering::Greater => {
                            // 完全不相邻且无重叠：分开
                            merged.push((cur_s, cur_e, keys));
                            current = Some((start, end, vec![ks, ke]));
                        }
                        Ordering::Equal | Ordering::Less => {
                            // 相邻（start == cur_e）或重叠/包含（start < cur_e）：合并
                            if end > cur_e {
                                cur_e = end;
                            }
                            keys.push(ks);
                            keys.push(ke);
                            current = Some((cur_s, cur_e, keys));
                        }
                    }
                } else {
                    current = Some((start, end, vec![ks, ke]));
                }
            }
            if let Some(seg) = current {
                merged.push(seg);
            }

            // 3) 生成UI矩形数据（逻辑像素坐标）
            for (start, end, mut keys) in merged {
                // 去重（按键），再根据几何顺序排序
                keys.sort_by_key(|k| {
                    let v = self.screen_map.verts.get(*k).unwrap();
                    if is_horizontal { v.vec.x } else { v.vec.y }
                });
                keys.dedup();
                let vert_keys_vec: Vec<SlintKey> =
                    keys.into_iter().map(|k| key_to_slint_area_key(k)).collect();

                if is_horizontal {
                    let width = ((end - start) as f64 / scale_factor) as f32;
                    let height = (((half_edge_width * 2.0) / scale_factor) + 1.0) as f32;
                    let x = (start as f64 / scale_factor) as f32;
                    let y = ((constant as f64 - half_edge_width) / scale_factor) as f32;
                    result.push(EdgeSegmentData {
                        x,
                        y,
                        width,
                        height,
                        vert_keys: vert_keys_vec,
                    });
                } else {
                    let height = ((end - start) as f64 / scale_factor) as f32;
                    let width = (((half_edge_width * 2.0) / scale_factor) + 1.0) as f32;
                    let x = ((constant as f64 - half_edge_width) / scale_factor) as f32;
                    let y = (start as f64 / scale_factor) as f32;
                    result.push(EdgeSegmentData {
                        x,
                        y,
                        width,
                        height,
                        vert_keys: vert_keys_vec,
                    });
                }
            }
        }

        result
    }

    fn convert_edges_to_slint_touch_data(&self) -> Vec<ScrEdgeTouchAreaData> {
        self.compute_edges_data()
            .into_iter()
            .map(|edge| ScrEdgeTouchAreaData {
                x: edge.x,
                y: edge.y,
                width: edge.width,
                height: edge.height,
                vert_keys: edge.vert_keys.as_slice().into(),
            })
            .collect()
    }

    /// 统一获取边数据的方法，返回显示和触摸数据
    fn convert_edges_to_slint_both_data(
        &self,
    ) -> (Vec<ScrEdgeDisplayData>, Vec<ScrEdgeTouchAreaData>) {
        let edges_data = self.compute_edges_data();

        let display_data = edges_data
            .iter()
            .map(|edge| ScrEdgeDisplayData {
                x: edge.x,
                y: edge.y,
                width: edge.width,
                height: edge.height,
                active: false,
            })
            .collect();

        let touch_data = edges_data
            .into_iter()
            .map(|edge| ScrEdgeTouchAreaData {
                x: edge.x,
                y: edge.y,
                width: edge.width,
                height: edge.height,
                vert_keys: edge.vert_keys.as_slice().into(),
            })
            .collect();

        (display_data, touch_data)
    }

    /// 以包含端坐标返回当前窗口的物理像素矩形 (0,0)-(w-1,h-1)
    pub fn current_window_rect(&self) -> Rcti {
        Rcti {
            xmin: 0,
            ymin: 0,
            xmax: self.window_size.width.saturating_sub(1),
            ymax: self.window_size.height.saturating_sub(1),
        }
    }
}
/// 简化的应用窗口管理器
pub struct AppWindowManager {
    instance: MainWindow,
    state: Rc<RefCell<AppState>>,
}

impl AppWindowManager {
    /// 创建新的应用窗口管理器
    pub fn new() -> Result<Self> {
        use slint::BackendSelector;
        // 强制选择winit后端
        {
            let sel = BackendSelector::new();
            let res = sel
                .backend_name("winit".into())
                // 使用 WGPU 渲染以支持视频显示
                .require_wgpu_24(WGPUConfiguration::Automatic(WGPUSettings::default()))
                .select();
            if let Err(e) = res {
                eprintln!("Slint 后端选择失败: {:?}", e);
                eprintln!("提示: 需要可用的 WGPU 环境与驱动，且请在 nix develop 中运行");
                return Err(e.into());
            }
        }
        let instance = MainWindow::new().context("创建 Slint 主窗口失败")?;
        // FIXME: 这里获得的是未初始化的数据
        let state = Rc::new(RefCell::new(AppState::new(&instance.window())));
        Ok(Self { instance, state })
    }

    /// 获取 MainWindow 的引用
    pub fn get_instance(&self) -> &MainWindow {
        &self.instance
    }

    /// 运行应用
    pub fn run(&mut self) -> Result<()> {
        self.setup_callbacks();

        // 直接启动 UI 事件循环
        self.instance.run().context("启动 UI 事件循环失败")
    }

    fn setup_callbacks(&self) {
        let weak_instance = self.instance.as_weak();
        let window = self.instance.window();

        // 初始化 GStreamer 与播放管线（WGPU 路径）
        gstreamer::init().unwrap();
        let pipeline = gstreamer::ElementFactory::make("playbin")
            .property("uri", "file:///home/Sittymin/Downloads/IMG_0161.MP4")
            .build()
            .unwrap()
            .downcast::<gstreamer::Pipeline>()
            .unwrap();

        {
            let mut state = self.state.borrow_mut();
            state.video_pipeline = Some(pipeline.clone());
        }

        let weak_instance_for_init = weak_instance.clone();
        let state_for_init = Rc::clone(&self.state);
        if let Err(e) = window.set_rendering_notifier(move |rendering_state, graphics_api| {
            if let Some(instance) = weak_instance_for_init.upgrade() {
                match rendering_state {
                    RenderingState::RenderingSetup => {
                        println!(
                            "窗口初始化完成: {:?}, {:?}",
                            instance.window().size(),
                            graphics_api
                        );
                        if let Err(e) =
                            AppWindowManager::initialize_layout(&state_for_init, &instance)
                        {
                            panic!("布局初始化失败: {e}")
                        }

                        // 连接 WGPU Sink 并启动播放
                        let mut state = state_for_init.borrow_mut();
                        if let Some(pipeline) = state.video_pipeline.clone() {
                            // 创建消息通道处理 GStreamer 总线消息
                            let (bus_sender, mut bus_receiver) =
                                futures::channel::mpsc::unbounded::<gstreamer::Message>();

                            // 启动异步任务处理总线消息
                            std::thread::spawn(move || {
                                let rt = tokio::runtime::Runtime::new().unwrap();
                                rt.block_on(async {
                                    while let Some(msg) =
                                        futures::StreamExt::next(&mut bus_receiver).await
                                    {
                                        match msg.view() {
                                            gstreamer::MessageView::Error(err) => {
                                                eprintln!("GStreamer 错误: {}", err.error());
                                            }
                                            gstreamer::MessageView::Eos(_) => {
                                                println!("视频播放结束");
                                            }
                                            _ => {}
                                        }
                                    }
                                });
                            });

                            // 创建 WGPU Sink
                            let (sink_element, mut wgpu_sink) =
                                crate::slint_video_sink::create_sink(&pipeline);

                            // 连接 WGPU 上下文
                            wgpu_sink.connect(
                                graphics_api,
                                &pipeline.bus().unwrap(),
                                bus_sender,
                                Box::new({
                                    let weak_instance = instance.as_weak();
                                    move || {
                                        weak_instance
                                            .upgrade_in_event_loop(|ui| {
                                                ui.window().request_redraw()
                                            })
                                            .ok();
                                    }
                                }),
                            );

                            // 启动播放
                            if let Err(err) = pipeline.set_state(gstreamer::State::Playing) {
                                eprintln!("设置播放状态失败: {err:?}");
                            }

                            state.video_sink = Some(sink_element);
                            state.wgpu_sink = Some(wgpu_sink);

                            // 初始化字幕渲染器
                            let subtitle_renderer =
                                crate::wgpu_subtitle_renderer::SubtitleRenderer::new(graphics_api);
                            state.subtitle_renderer = Some(subtitle_renderer);

                            // 初始化 libass 字幕渲染器 (使用实际窗口大小)
                            let window_size = instance.window().size();
                            match aweft_libass::SubtitleRenderer::new(
                                window_size.width as i32,
                                window_size.height as i32,
                            ) {
                                Ok(libass_renderer) => {
                                    state.libass_renderer = Some(libass_renderer);
                                    println!("字幕渲染器初始化成功");
                                }
                                Err(e) => {
                                    eprintln!("字幕渲染器初始化失败: {}", e);
                                }
                            }

                            // FIXME: 需要处理此函数执行时机
                            // 生成并显示字幕（只在第一次渲染时执行）
                            // HACK: 可能是原子操作，之后需要移动位置或改变类型
                            let subtitle_image_opt = Self::render_hello_world_subtitle(
                                &mut state,
                                &instance,
                                window_size.width as i32,
                                window_size.height as i32,
                            );
                            if let Some((subtitle_image, x, y)) = subtitle_image_opt {
                                instance
                                    .global::<SubtitleState>()
                                    .set_current_subtitle_frame(subtitle_image);
                                instance.set_subtitle_x(x as f32);
                                instance.set_subtitle_y(y as f32);
                            }
                        }
                    }
                    RenderingState::BeforeRendering => {
                        // 获取新的视频帧并设置到 UI
                        let mut image_opt = None;
                        {
                            let mut state = state_for_init.borrow_mut();
                            if let Some(wgpu_sink) = &state.wgpu_sink {
                                image_opt = wgpu_sink.fetch_next_frame();
                            }
                        }
                        if let Some(image) = image_opt {
                            // println!("收到新视频帧，设置到 UI");
                            instance
                                .global::<crate::types::VideoState>()
                                .set_current_video_frame(image);
                        }
                    }
                    RenderingState::RenderingTeardown => {
                        // 清理 WGPU sink
                        let state = state_for_init.borrow();
                        if let Some(wgpu_sink) = &state.wgpu_sink {
                            wgpu_sink.deactivate_and_pause();
                        }
                    }
                    _ => (),
                }
            }
        }) {
            eprintln!("注册渲染初始化失败: {:?}", e);
        };

        let weak_instance_for_drag = weak_instance.clone();
        let state_for_drag = Rc::clone(&self.state);
        self.instance
            .global::<CallbackManager>()
            .on_actionzone_drag(move |area_key, corner, offset_x, offset_y| {
                let mut state = state_for_drag.borrow_mut();
                let scale_factor = state.ui_settings.get_scale_factor();
                let actionzone_side_length = state.ui_settings.get_actionzone_side_length();

                let physical_offset_x = offset_x as f64 * scale_factor;
                let physical_offset_y = offset_y as f64 * scale_factor;

                // 将 SlintKey 转回 SlotMap Key
                let source_area_key: ScrAreaKey = slint_area_key_to_key(&area_key);

                // 防护检查：确保源区域仍然存在
                if !state.screen_map.areas.contains_key(source_area_key) {
                    println!("警告：源区域 {:?} 已不存在，忽略拖拽事件", source_area_key);
                    return;
                }

                if let Some(instance) = weak_instance_for_drag.upgrade() {
                    if let Some(target_area_key) = state.screen_map.find_area_with_actionzone(
                        source_area_key,
                        corner,
                        physical_offset_x,
                        physical_offset_y,
                        actionzone_side_length,
                    ) {
                        if target_area_key == source_area_key {
                            // println!("在相同区域内 - 执行分割操作");
                            let (relative_offset_x, relative_offset_y) =
                                state.screen_map.convert_corner_offset_to_relative_offset(
                                    target_area_key,
                                    corner,
                                    physical_offset_x,
                                    physical_offset_y,
                                    actionzone_side_length,
                                );
                            let split_preview_overlay_data =
                                state.screen_map.calculate_split_areas(
                                    target_area_key,
                                    physical_offset_x,
                                    physical_offset_y,
                                    relative_offset_x,
                                    relative_offset_y,
                                );

                            // 更新拖拽状态（保持物理像素数据）
                            state.current_drag_operation = DragOperation::Split {
                                source_area: source_area_key,
                                preview_data: split_preview_overlay_data.clone(),
                            };

                            // 显示分割预览，隐藏合并预览
                            instance.set_show_join_highlight_overlay(false);
                            instance.set_show_close_area_overlays(false);
                            instance.set_show_split_preview_overlay(true);
                            // 将数据转换为逻辑像素再发送到 Slint
                            let split_preview_overlay_data_logical =
                                scale_split_overlay(split_preview_overlay_data, scale_factor);
                            instance
                                .set_split_preview_overlay_data(split_preview_overlay_data_logical);
                        } else {
                            // println!("移动到不同区域 {:?} - 执行合并操作", target_area_key);
                            // 计算邻接关系
                            if let Some(adjacency) = state
                                .screen_map
                                .detect_adjacency_by_keys(source_area_key, target_area_key)
                            {
                                // 更新拖拽状态
                                state.current_drag_operation = DragOperation::Join {
                                    source_area_key,
                                    target_area_key,
                                    adjacency,
                                };

                                // 生成合并高亮与即将关闭区域预览
                                match state.screen_map.preview_join_adjacent_areas(
                                    source_area_key,
                                    adjacency,
                                    &state.ui_settings,
                                ) {
                                    Ok((join_data, close_datas)) => {
                                        // 显示合并预览，隐藏分割预览
                                        instance.set_show_split_preview_overlay(false);
                                        // 缩放到逻辑像素
                                        let join_data_logical =
                                            scale_join_overlay(join_data, scale_factor);
                                        let close_datas_logical =
                                            scale_close_overlays(close_datas, scale_factor);
                                        instance.set_join_highlight_overlay_data(join_data_logical);
                                        instance.set_show_join_highlight_overlay(true);
                                        instance.set_close_area_overlay_datas(
                                            close_datas_logical.as_slice().into(),
                                        );
                                        instance.set_show_close_area_overlays(
                                            !close_datas_logical.is_empty(),
                                        );
                                    }
                                    Err(e) => {
                                        eprintln!("生成合并预览失败: {}", e);
                                        instance.set_show_join_highlight_overlay(false);
                                        instance.set_show_close_area_overlays(false);
                                    }
                                }
                            } else {
                                // 非相邻：清除拖拽状态，隐藏合并预览
                                state.current_drag_operation = DragOperation::None;
                                instance.set_show_join_highlight_overlay(false);
                                instance.set_show_close_area_overlays(false);
                            }
                        }
                    } else {
                        // println!("移动到区域外 - 可能执行窗口操作");
                        // 离开任何区域，清除拖拽状态，隐藏所有预览
                        state.current_drag_operation = DragOperation::None;
                        instance.set_show_split_preview_overlay(false);
                        instance.set_show_join_highlight_overlay(false);
                        instance.set_show_close_area_overlays(false);
                    }
                }
            });

        let weak_instance_for_drag_end = weak_instance.clone();
        let state_for_drag_end = Rc::clone(&self.state);
        self.instance
            .global::<CallbackManager>()
            .on_actionzone_drag_end(move |area_key, is_cancel| {
                println!("鼠标拖动结束, 是否是右键取消: {is_cancel}");

                if !is_cancel {
                    if let Some(instance) = weak_instance_for_drag_end.upgrade() {
                        let mut state = state_for_drag_end.borrow_mut();

                        // 先提取拖拽状态，避免借用冲突
                        let drag_operation = state.current_drag_operation.clone();

                        // 根据存储的拖拽状态执行相应操作
                        match drag_operation {
                            DragOperation::Split {
                                source_area,
                                preview_data,
                            } => {
                                println!(
                                    "执行拆分操作，源区域: {:?}, 预览数据: {:#?}",
                                    source_area, preview_data
                                );

                                // 验证分割数据是否有效（避免0尺寸的区域）
                                if preview_data.width2 <= 0.0 || preview_data.height2 <= 0.0 {
                                    println!("分割数据无效（第二个区域尺寸为0），取消分割操作");
                                } else {
                                    let ui_settings = &state.ui_settings.clone();
                                    if let Err(e) = state.screen_map.split_area(
                                        source_area,
                                        preview_data,
                                        ui_settings,
                                    ) {
                                        eprintln!("执行区域拆分失败: {}", e);
                                    } else {
                                        // 更新UI显示与边缘边标记
                                        let current_rect = state.current_window_rect();
                                        let slint_area_data = state.convert_areas_to_slint_data();
                                        instance.set_scrareas(slint_area_data.as_slice().into());
                                        // 更新 Edge 位置
                                        state
                                            .screen_map
                                            .mark_window_border_edges(&current_rect, ui_settings);
                                        let (slint_edges_display_data, slint_edges_touch_data) =
                                            state.convert_edges_to_slint_both_data();
                                        instance.set_scredgedisplays(
                                            slint_edges_display_data.as_slice().into(),
                                        );
                                        instance.set_scredgetouchareas(
                                            slint_edges_touch_data.as_slice().into(),
                                        );
                                    }
                                }
                            }
                            DragOperation::Join {
                                source_area_key,
                                target_area_key,
                                adjacency,
                            } => {
                                println!(
                                    "执行合并操作，源区域: {:?} -> 目标区域: {:?}",
                                    source_area_key, target_area_key
                                );

                                // 立即清除拖拽状态，避免后续事件使用过期的区域键
                                state.current_drag_operation = DragOperation::None;

                                let ui_settings = &state.ui_settings.clone();

                                if let Err(e) = state.screen_map.apply_join_adjacent_areas(
                                    source_area_key,
                                    target_area_key,
                                    adjacency,
                                    ui_settings,
                                ) {
                                    eprintln!("执行区域合并失败: {}", e);
                                } else {
                                    // 重新计算所有区域的几何数据
                                    state.screen_map.update_all_areas_rcti(ui_settings);

                                    println!("合并后的所有区域信息:");
                                    for (area_key, area) in &state.screen_map.areas {
                                        let (rect, _, _) = area.geometry();
                                        println!(
                                            "  区域 {:?}: 位置=({}, {}, {}, {})",
                                            area_key, rect.xmin, rect.ymin, rect.xmax, rect.ymax
                                        );
                                    }
                                    println!("合并后的所有顶点信息:");
                                    for (vert_key, vert) in &state.screen_map.verts {
                                        println!(
                                            "  顶点 {:?}: 位置=({}, {})",
                                            vert_key, vert.vec.x, vert.vec.y
                                        );
                                    }
                                    println!("合并后的所有边信息:");
                                    for (edge_key, edge) in &state.screen_map.edges {
                                        let v1 = state.screen_map.verts.get(edge.v1).unwrap();
                                        let v2 = state.screen_map.verts.get(edge.v2).unwrap();
                                        println!(
                                            "  边 {:?}: v1=({}, {}) -> v2=({}, {}), border={}",
                                            edge_key,
                                            v1.vec.x,
                                            v1.vec.y,
                                            v2.vec.x,
                                            v2.vec.y,
                                            edge.border
                                        );
                                    }

                                    // 更新UI显示与边缘边标记
                                    let current_rect = state.current_window_rect();
                                    let slint_area_data = state.convert_areas_to_slint_data();
                                    instance.set_scrareas(slint_area_data.as_slice().into());
                                    // 更新 Edge 位置
                                    state
                                        .screen_map
                                        .mark_window_border_edges(&current_rect, ui_settings);
                                    let (slint_edges_display_data, slint_edges_touch_data) =
                                        state.convert_edges_to_slint_both_data();
                                    instance.set_scredgedisplays(
                                        slint_edges_display_data.as_slice().into(),
                                    );
                                    instance.set_scredgetouchareas(
                                        slint_edges_touch_data.as_slice().into(),
                                    );
                                }
                            }
                            _ => {
                                // println!("非 Action 拖动，取消操作");
                            }
                        }

                        // 清除拖拽状态和预览（如果还没清除的话）
                        state.current_drag_operation = DragOperation::None;
                        instance.set_show_split_preview_overlay(false);
                        instance.set_show_join_highlight_overlay(false);
                        instance.set_show_close_area_overlays(false);
                    }
                } else {
                    // println!("用户取消拖拽操作");
                    // 清除拖拽状态和预览
                    if let Some(instance) = weak_instance_for_drag_end.upgrade() {
                        let mut state = state_for_drag_end.borrow_mut();
                        state.current_drag_operation = DragOperation::None;
                        instance.set_show_split_preview_overlay(false);
                        instance.set_show_join_highlight_overlay(false);
                        instance.set_show_close_area_overlays(false);
                    }
                }
            });

        // 边拖拽处理
        let weak_instance_for_edge_drag = weak_instance.clone();
        let state_for_edge_drag = Rc::clone(&self.state);
        self.instance.global::<CallbackManager>().on_edge_drag(
            move |vert_keys, offset_x, offset_y| {
                let mut state = state_for_edge_drag.borrow_mut();
                let scale_factor = state.ui_settings.get_scale_factor();

                if let Some(instance) = weak_instance_for_edge_drag.upgrade() {
                    // 如果当前已经在边移动状态，继续移动
                    if let DragOperation::MoveEdge {
                        dir_axis_horizontal,
                        origval,
                        bigger,
                        smaller,
                        moving_vert_keys,
                    } = &state.current_drag_operation.clone()
                    {
                        let ui_settings = &state.ui_settings.clone();
                        let current_offset_x = offset_x as f64 - ui_settings.get_half_edge_width();
                        let current_offset_y = offset_y as f64 - ui_settings.get_half_edge_width();
                        match dir_axis_horizontal {
                            &true => {
                                state.screen_map.apply_edge_move(
                                    *dir_axis_horizontal,
                                    moving_vert_keys,
                                    *origval,
                                    (current_offset_y * scale_factor).round() as i32,
                                    *bigger,
                                    *smaller,
                                );
                            }
                            &false => {
                                state.screen_map.apply_edge_move(
                                    *dir_axis_horizontal,
                                    moving_vert_keys,
                                    *origval,
                                    (current_offset_x * scale_factor).round() as i32,
                                    *bigger,
                                    *smaller,
                                );
                            }
                        }
                        println!("offset: {current_offset_x}, {current_offset_y}");
                        // 更新UI
                        state.screen_map.update_all_areas_rcti(ui_settings);
                        let slint_area_data = state.convert_areas_to_slint_data();
                        instance.set_scrareas(slint_area_data.as_slice().into());
                        let slint_edges_data = state.convert_edges_to_slint_display_data();
                        instance.set_scredgedisplays(slint_edges_data.as_slice().into());
                        println!("边移动中...");
                        return;
                    }

                    // 开始新的边移动操作
                    let vert_keys_slice: Vec<SlintKey> = vert_keys.iter().collect();

                    // 转换 SlintKey 到 ScrVertKey
                    let moving_vert_keys: Vec<ScrVertKey> = vert_keys_slice
                        .iter()
                        .map(|k| slint_area_key_to_key(k))
                        .collect();

                    // 分析移动方向和原始坐标
                    if let Some((dir_axis_horizontal, origval)) =
                        state.screen_map.analyze_move_direction(&moving_vert_keys)
                    {
                        // 计算移动限制
                        const MIN_AREA_SIZE: u32 = 50; // TODO: 与 types.rs 中保持一致
                        let (bigger, smaller) = state.screen_map.compute_move_limits(
                            dir_axis_horizontal,
                            &moving_vert_keys,
                            origval,
                            MIN_AREA_SIZE,
                        );

                        // 设置拖拽状态
                        state.current_drag_operation = DragOperation::MoveEdge {
                            dir_axis_horizontal,
                            origval,
                            bigger,
                            smaller,
                            moving_vert_keys,
                        };

                        println!(
                            "开始边移动操作: 方向={}, 原始值={}, 限制=({}, {})",
                            if dir_axis_horizontal {
                                "水平"
                            } else {
                                "垂直"
                            },
                            origval,
                            bigger,
                            smaller
                        );
                    }
                }
            },
        );

        let weak_instance_for_edge_drag_end = weak_instance.clone();
        let state_for_edge_drag_end = Rc::clone(&self.state);
        self.instance
            .global::<CallbackManager>()
            .on_edge_drag_end(move |is_cancel| {
                println!("边拖动结束, 是否取消: {}", is_cancel);

                if let Some(instance) = weak_instance_for_edge_drag_end.upgrade() {
                    let mut state = state_for_edge_drag_end.borrow_mut();

                    if let DragOperation::MoveEdge {
                        dir_axis_horizontal,
                        origval,
                        bigger,
                        smaller,
                        moving_vert_keys,
                    } = &state.current_drag_operation.clone()
                    {
                        let ui_settings = &state.ui_settings.clone();

                        if is_cancel {
                            // 取消：恢复到原始位置
                            println!("取消边移动，恢复到原始位置");
                            state.screen_map.apply_edge_move(
                                *dir_axis_horizontal,
                                moving_vert_keys,
                                *origval,
                                0, // delta = 0 恢复原位置
                                *bigger,
                                *smaller,
                            );
                            state.screen_map.update_all_areas_rcti(ui_settings);
                        } else {
                            // 确认：保持当前位置
                            println!("确认边移动");
                            state.screen_map.remove_duplicate_vertices();
                        }

                        // 清理和更新UI
                        let current_rect = state.current_window_rect();
                        let slint_area_data = state.convert_areas_to_slint_data();
                        instance.set_scrareas(slint_area_data.as_slice().into());
                        state
                            .screen_map
                            .mark_window_border_edges(&current_rect, ui_settings);

                        // 边拖拽结束时，同时更新显示数据和触摸数据
                        let (slint_edges_display_data, slint_edges_touch_data) =
                            state.convert_edges_to_slint_both_data();
                        instance.set_scredgedisplays(slint_edges_display_data.as_slice().into());
                        instance.set_scredgetouchareas(slint_edges_touch_data.as_slice().into());
                    }

                    // 清除拖拽状态
                    state.current_drag_operation = DragOperation::None;
                }
            });

        // Winit 事件处理
        let state_for_event = Rc::clone(&self.state);
        let weak_instance_for_event = weak_instance.clone();
        self.instance
            .window()
            .on_winit_window_event(move |_slint_window, event| {
                match event {
                    winit::event::WindowEvent::Resized(new_size) => {
                        println!("窗口大小发生改变: {}x{}", new_size.width, new_size.height);
                        if let Some(instance) = weak_instance_for_event.upgrade() {
                            let mut state = state_for_event.borrow_mut();

                            // 调用新的、清晰的逻辑（需要 instance）
                            if let Err(e) = Self::handle_window_resize(&mut state, *new_size) {
                                eprintln!("处理窗口大小变化失败: {}", e);
                            }

                            // 更新 UI
                            let slint_area_data = state.convert_areas_to_slint_data();
                            instance.set_scrareas(slint_area_data.as_slice().into());
                            // 更新 Edge 位置
                            let (slint_edges_display_data, slint_edges_touch_data) =
                                state.convert_edges_to_slint_both_data();
                            instance
                                .set_scredgedisplays(slint_edges_display_data.as_slice().into());
                            instance
                                .set_scredgetouchareas(slint_edges_touch_data.as_slice().into());

                            // HACK: 更新字幕
                            // FIXME: 需要使用动态参数的数据
                            let subtitle_image_opt = Self::render_hello_world_subtitle(
                                &mut state,
                                &instance,
                                new_size.width as i32,
                                new_size.height as i32,
                            );
                            if let Some((subtitle_image, x, y)) = subtitle_image_opt {
                                instance
                                    .global::<SubtitleState>()
                                    .set_current_subtitle_frame(subtitle_image);
                                instance.set_subtitle_x(x as f32);
                                instance.set_subtitle_y(y as f32);
                            }
                        }
                    }
                    winit::event::WindowEvent::ScaleFactorChanged { scale_factor, .. } => {
                        println!("检测到屏幕缩放因子变化: {:.2}", scale_factor);
                        if let Some(instance) = weak_instance_for_event.upgrade() {
                            let mut state = state_for_event.borrow_mut();

                            if let Err(e) = Self::handle_scale_change(&mut state, *scale_factor) {
                                eprintln!("处理缩放因子变化失败: {}", e);
                            }

                            // 更新 UI
                            let slint_area_data = state.convert_areas_to_slint_data();
                            instance.set_scrareas(slint_area_data.as_slice().into());
                            let (slint_edges_display_data, slint_edges_touch_data) =
                                state.convert_edges_to_slint_both_data();
                            instance
                                .set_scredgedisplays(slint_edges_display_data.as_slice().into());
                            instance
                                .set_scredgetouchareas(slint_edges_touch_data.as_slice().into());
                        }
                    }
                    _ => {}
                }
                WinitWindowEventResult::Propagate
            });
    }

    fn initialize_layout(state_rc: &Rc<RefCell<AppState>>, instance: &MainWindow) -> Result<()> {
        let mut state = state_rc.borrow_mut();
        let (width, height, ui_settings) = {
            if state.initialized {
                return Ok(());
            }

            println!(
                "窗口已初始化，物理尺寸: {}x{}, 缩放因子: {:.2}",
                state.window_size.width,
                state.window_size.height,
                state.ui_settings.get_scale_factor()
            );
            (
                state.window_size.width,
                state.window_size.height,
                state.ui_settings.clone(),
            )
        };

        state
            .screen_map
            .create_three_area_layout(&width, &height, &ui_settings);
        state.initialized = true;

        let slint_area_data = state.convert_areas_to_slint_data();
        instance.set_scrareas(slint_area_data.as_slice().into());
        let current_rect = state.current_window_rect();
        let ui_settings = state.ui_settings;
        state
            .screen_map
            .mark_window_border_edges(&current_rect, &ui_settings);
        let (slint_edges_display_data, slint_edges_touch_data) =
            state.convert_edges_to_slint_both_data();
        instance.set_scredgedisplays(slint_edges_display_data.as_slice().into());
        instance.set_scredgetouchareas(slint_edges_touch_data.as_slice().into());

        Ok(())
    }

    fn handle_window_resize(
        state: &mut AppState,
        new_size: winit::dpi::PhysicalSize<u32>,
    ) -> Result<()> {
        let old_size = state.window_size;

        let old_window_rect = Rcti {
            xmin: 0,
            xmax: old_size.width.saturating_sub(1),
            ymin: 0,
            ymax: old_size.height.saturating_sub(1),
        };

        let new_window_rect = Rcti {
            xmin: 0,
            xmax: new_size.width.saturating_sub(1),
            ymin: 0,
            ymax: new_size.height.saturating_sub(1),
        };

        // 关键：缩放所有顶点
        state.screen_map.scale_vertices_to_window(
            &old_window_rect,
            &new_window_rect,
            &state.ui_settings,
        );

        // 更新状态中的窗口大小
        state.window_size = PhysicalSize {
            width: new_size.width,
            height: new_size.height,
        };

        // 处理边缘顶点，避免微小错位
        state
            .screen_map
            .enforce_window_edge_snap(&new_window_rect, &state.ui_settings);

        Ok(())
    }
    fn handle_scale_change(state: &mut AppState, new_scale_factor: f64) -> Result<()> {
        state.ui_settings.update_scale_factor(new_scale_factor);

        // 更新所有区域的几何属性，以应用新的边距等
        state.screen_map.update_all_areas_rcti(
            &state.ui_settings, // 直接传递引用
        );
        Ok(())
    }

    // TODO: WARN: 修改为通用转换函数，或修改 WGPU 直接输出 Slint Image
    /// 渲染 "Hello World!" 字幕
    fn render_hello_world_subtitle(
        state: &mut AppState,
        instance: &MainWindow,
        window_width: i32,
        window_height: i32,
    ) -> Option<(slint::Image, i32, i32)> {
        // 分别获取渲染器的引用，避免借用冲突
        let subtitle_renderer = {
            state
                .subtitle_renderer
                .as_ref()
                .map(|renderer| renderer.clone())
        };

        let mut subtitle_renderer = match subtitle_renderer {
            Some(renderer) => renderer,
            Option::None => {
                // 字幕渲染器未初始化，跳过
                return None;
            }
        };

        // 获取 libass 渲染器的可变引用
        let render_result = {
            match &mut state.libass_renderer {
                Some(libass_renderer) => {
                    // 设置播放分辨率为实际窗口大小
                    libass_renderer.set_frame_size(window_width, window_height as i32);

                    if let Err(e) = libass_renderer
                        .set_default_style_alignment(aweft_libass::types::Alignment::BottomCenter)
                    {
                        eprintln!("❌ [ERROR] 设置字幕对齐方式失败: {}", e);
                        return None;
                    }

                    // 使用 libass 创建 "Hello World!" 字幕事件
                    if let Err(e) = libass_renderer.create_event_with_default_style("Hello World!")
                    {
                        eprintln!("❌ [ERROR] 创建字幕事件失败: {}", e);
                        return None;
                    }

                    // 渲染字幕 (时间戳设为 1000ms，确保字幕显示)
                    match libass_renderer.render_frame(1000) {
                        Ok(result) => result,
                        Err(e) => {
                            eprintln!("❌ [ERROR] 渲染字幕失败: {}", e);
                            return None;
                        }
                    }
                }
                Option::None => {
                    eprintln!("❌ [ERROR] libass 渲染器未初始化，跳过");
                    return None;
                }
            }
        };

        // 检查是否有渲染结果
        if render_result.is_empty() {
            return None;
        }

        // 使用 WGPU 渲染器将 libass 结果转换为纹理
        match subtitle_renderer.render_subtitles_gpu(render_result.iter()) {
            Ok(rendered_texture) => {
                println!(
                    "width: {:?}, height: {:?}, x: {:?}, y: {:?}",
                    rendered_texture.width,
                    rendered_texture.height,
                    rendered_texture.x,
                    rendered_texture.y
                );
                // 将 WGPU 纹理转换为 Slint Image
                match slint::Image::try_from(rendered_texture.texture.clone()) {
                    Ok(image) => Some((image, rendered_texture.x, rendered_texture.y)),
                    Err(e) => {
                        eprintln!("❌ [ERROR] 转换字幕纹理为 Slint Image 失败: {:?}", e);
                        None
                    }
                }
            }
            Err(e) => {
                eprintln!("❌ [ERROR] WGPU 字幕渲染失败: {}", e);
                None
            }
        }
    }
}

// 导出一个公共函数，用于创建和运行窗口
pub fn create_window() -> Result<()> {
    let mut app = AppWindowManager::new().context("创建应用窗口管理器失败（是否nix develop?）")?;
    app.run().context("运行应用失败")
}
