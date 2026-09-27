use crate::bridge::SubtitleState;
use crate::bridge::types::MainWindow;
use crate::store::AppStore;
use anyhow::{Context, Result};
use aweft_model::sequence::GroupData;
use slint::ComponentHandle;
use slint::wgpu_30::WGPUConfiguration;
use std::{cell::RefCell, rc::Rc};

/// 自建 wgpu 设备/队列（交 slint Manual 配置使用）。
/// TEXTURE_FORMAT_16BIT_NORM：P010 10bit 视频平面（R16Unorm/Rg16Unorm）必需。
fn create_wgpu_device() -> Result<(
    slint::wgpu_30::wgpu::Instance,
    slint::wgpu_30::wgpu::Adapter,
    slint::wgpu_30::wgpu::Device,
    slint::wgpu_30::wgpu::Queue,
)> {
    use slint::wgpu_30::wgpu;
    // surface 由 slint 后续从该 instance 创建；adapter 先按性能偏好选取
    // （原生平台 request_adapter 在首次 poll 即完成，block_on 不会卡）
    let instance = wgpu::Instance::new(wgpu::InstanceDescriptor::new_without_display_handle());
    let adapter =
        futures::executor::block_on(instance.request_adapter(&wgpu::RequestAdapterOptions {
            power_preference: wgpu::PowerPreference::HighPerformance,
            ..Default::default()
        }))
        .context("无可用 wgpu 适配器")?;
    let mut required_features = wgpu::Features::TEXTURE_FORMAT_16BIT_NORM;
    if adapter
        .features()
        .contains(wgpu::Features::VULKAN_EXTERNAL_MEMORY_DMA_BUF)
    {
        required_features |= wgpu::Features::VULKAN_EXTERNAL_MEMORY_DMA_BUF;
        log::info!("启用 wgpu Vulkan DMA-BUF 外部内存特性");
    } else {
        log::debug!("当前 wgpu 适配器不支持 Vulkan DMA-BUF 外部内存特性");
    }
    let (device, queue) =
        futures::executor::block_on(adapter.request_device(&wgpu::DeviceDescriptor {
            required_features,
            required_limits: wgpu::Limits::default().using_resolution(adapter.limits()),
            ..Default::default()
        }))
        .context("创建 wgpu 设备失败（TEXTURE_FORMAT_16BIT_NORM 不受支持？）")?;
    Ok((instance, adapter, device, queue))
}

/// 简化的应用窗口管理器
pub struct AppWindowManager {
    instance: MainWindow,
    store: Rc<RefCell<AppStore>>,
}

impl AppWindowManager {
    /// 创建新的应用窗口管理器
    pub fn new() -> Result<Self> {
        use slint::BackendSelector;
        // 初始化日志（放最前，确保后续所有 log::* 都可用）。
        // 默认只输出本 workspace（aweft*）的 debug+；第三方 crate（slint/
        // gstreamer/winit 等）默认 warn+，避免刷屏。可通过 RUST_LOG 环境变量
        // 覆盖（如 `RUST_LOG=debug` 或 `RUST_LOG=aweft_gui=trace`）。
        env_logger::Builder::from_env(
            env_logger::Env::default().default_filter_or(
                "aweft=debug,aweft_gui=debug,aweft_model=debug,aweft_convert=debug,aweft_libass=debug,warn",
            ),
        )
        .init();
        // 强制选择winit后端。Manual 自建 wgpu 设备（slint 官方推荐的纹理导入
        // 用法）：启用 16bit 归一化纹理格式（R16Unorm/Rg16Unorm，P010 10bit
        // 视频平面导入/上传必需；WGPUSettings 为 non_exhaustive 无法局部修改，
        // Automatic 默认不含该特性）。
        {
            let (instance, adapter, device, queue) = create_wgpu_device()?;
            let sel = BackendSelector::new();
            let res = sel
                .backend_name("winit".into())
                // 使用 WGPU 渲染以支持视频显示
                .require_wgpu_30(WGPUConfiguration::Manual {
                    instance,
                    adapter,
                    device,
                    queue,
                })
                .select();
            if let Err(e) = res {
                log::error!("Slint 后端选择失败: {:?}", e);
                log::error!("提示: 需要可用的 WGPU 环境与驱动，且请在 nix develop 中运行");
                return Err(e.into());
            }
        }
        let instance = MainWindow::new().context("创建 Slint 主窗口失败")?;
        // FIXME: 这里获得的是未初始化的数据
        let store = Rc::new(RefCell::new(AppStore::new(&instance)));
        // HACK: 给测试字幕数据接一条上下浮动的动画曲线（临时；正确路径是用户在曲线
        // 编辑器里 K 帧）。必须在节点图编辑器初始化之前——链菜单、第 0 条链与默认布局
        // 都从 model 派生（见 app::test_data）。
        crate::app::test_data::create_test_subtitle_animation(&mut store.borrow_mut());
        // 初始化节点图编辑器（建链/右键菜单、绑定全局、加载第 0 条链）
        store.borrow_mut().initialize_node_graph_editors(&instance);
        // HACK: 初始化曲线编辑器。视图/属性 Area 未实现、无法 K 帧，暂时直接用测试数据
        // 填充（便于交互测试）。正确路径是 `new_editing_action` 新建空 Action，由用户在
        // 编辑器中 K 帧构建——未来删除本行、改调 `new_editing_action`（见 create_test_animation_data）。
        store.borrow_mut().create_test_animation_data();
        store.borrow_mut().initialize_animation_editors(&instance);
        // HACK: 临时直接加载一个视频，并且失败直接崩溃
        // initialize_media(
        //     &mut store.borrow_mut(),
        //     "file:///home/Sittymin/Videos/IMG_0161.MP4",
        // )
        // .unwrap();

        // ===================================
        // 初始化字幕数据和过滤模型
        let (filter_model, timeline_root_blocks) = {
            let mut store_mut = store.borrow_mut();
            {
                // 先创建数据
                let state = store_mut.state_mut();
                create_test_subtitle_data(state.project.project_mut());
            }
            // 然后初始化 SequenceState（使用 UiModels 持有的模型）
            let filter = store_mut.rebuild_sequence_models();
            let timeline = slint::ModelRc::from(
                store_mut
                    .ui_models
                    .sequence_state
                    .timeline_root_blocks_model(),
            );
            (filter, timeline)
        };

        let subtitle_state = instance.global::<SubtitleState>();
        // NOTE: 之后 UI 数据需要在 filtered_model 修改
        // 不应当再使用 set_rows 修改数据
        subtitle_state.set_rows(filter_model);
        subtitle_state.set_timeline_root_blocks(timeline_root_blocks);

        // 初始化模型并绑定给 Slint（占位空数组，后续通过依赖图投影刷新）
        {
            let areas_model = store.borrow().ui_models().scrareas();
            let scr_edge_displays_model = store.borrow().ui_models().scredge_displays();
            let scr_edge_touch_areas_model = store.borrow().ui_models().scredge_touch_areas();
            instance.set_scr_areas(slint::ModelRc::from(areas_model));
            instance
                .set_scr_edge_touch_areas_displays(slint::ModelRc::from(scr_edge_displays_model));
            instance.set_scr_edge_touch_areas(slint::ModelRc::from(scr_edge_touch_areas_model));
        }

        log::info!("✅ 已初始化字幕过滤模型");

        Ok(Self { instance, store })
    }

    /// 获取 MainWindow 的引用
    pub fn get_instance(&self) -> &MainWindow {
        &self.instance
    }

    /// 运行应用
    pub fn run(&mut self) -> Result<()> {
        crate::bridge::callbacks::setup_callbacks(&self.instance, Rc::clone(&self.store));

        let result = self.instance.run().context("启动 UI 事件循环失败");

        // 退出前统一释放视频管线、appsink 和 GPU 资源。
        crate::render::video_sink::teardown(&mut self.store.borrow_mut());

        result
    }
}

/// 创建测试字幕数据
fn create_test_subtitle_data(project: &mut aweft_model::project::Project) {
    use aweft_model::sequence::{EventData, ItemLocation};
    use chrono::TimeDelta;

    let sequence = project.sequence_mut();

    // 创建根级别的事件
    let event1 = EventData {
        start_time: TimeDelta::try_milliseconds(0).unwrap(),
        end_time: TimeDelta::try_milliseconds(5000).unwrap(),
        actor: None,
        text: "这是第一行字幕（根级别）".to_string(),
        segment_timing: None,
        style: "Default".to_string(),
    };
    sequence
        .create_event(event1, ItemLocation::Root(0))
        .unwrap();

    // 创建一个分组
    let group = GroupData {
        name: "对话组".to_string(),
        collapsed: false,
        color: None,
        children: Vec::new(),
    };
    let group_id = sequence.create_group(group, ItemLocation::Root(1)).unwrap();

    // 在分组内创建事件
    let event2 = EventData {
        start_time: TimeDelta::try_milliseconds(5000).unwrap(),
        end_time: TimeDelta::try_milliseconds(10000).unwrap(),
        actor: None,
        text: "这是分组内的第一行".to_string(),
        segment_timing: None,
        style: "Default".to_string(),
    };
    sequence
        .create_event(event2, ItemLocation::InGroup(group_id, 0))
        .unwrap();

    // 创建多层嵌套分组：level1 -> level2 -> level3
    // L1 前置事件
    let l1_pre = EventData {
        start_time: TimeDelta::try_milliseconds(10000).unwrap(),
        end_time: TimeDelta::try_milliseconds(10500).unwrap(),
        actor: None,
        text: "L1 前置事件".to_string(),
        segment_timing: None,
        style: "Default".to_string(),
    };
    sequence
        .create_event(l1_pre, ItemLocation::InGroup(group_id, 1))
        .unwrap();
    // L1 分组
    let nested_group_lvl1 = sequence
        .create_group(
            GroupData {
                name: "循环测试 L1".to_string(),
                collapsed: false,
                color: None,
                children: Vec::new(),
            },
            ItemLocation::InGroup(group_id, 2),
        )
        .unwrap();
    // L1 后置事件
    let l1_post = EventData {
        start_time: TimeDelta::try_milliseconds(10500).unwrap(),
        end_time: TimeDelta::try_milliseconds(11000).unwrap(),
        actor: None,
        text: "L1 后置事件".to_string(),
        segment_timing: None,
        style: "Default".to_string(),
    };
    sequence
        .create_event(l1_post, ItemLocation::InGroup(group_id, 3))
        .unwrap();

    // L2 前置事件
    let l2_pre = EventData {
        start_time: TimeDelta::try_milliseconds(11000).unwrap(),
        end_time: TimeDelta::try_milliseconds(11500).unwrap(),
        actor: None,
        text: "L2 前置事件".to_string(),
        segment_timing: None,
        style: "Default".to_string(),
    };
    sequence
        .create_event(l2_pre, ItemLocation::InGroup(nested_group_lvl1, 0))
        .unwrap();
    // L2 分组
    let nested_group_lvl2 = sequence
        .create_group(
            GroupData {
                name: "循环测试 L2".to_string(),
                collapsed: false,
                color: None,
                children: Vec::new(),
            },
            ItemLocation::InGroup(nested_group_lvl1, 1),
        )
        .unwrap();
    // L2 后置事件
    let l2_post = EventData {
        start_time: TimeDelta::try_milliseconds(11500).unwrap(),
        end_time: TimeDelta::try_milliseconds(12000).unwrap(),
        actor: None,
        text: "L2 后置事件".to_string(),
        segment_timing: None,
        style: "Default".to_string(),
    };
    sequence
        .create_event(l2_post, ItemLocation::InGroup(nested_group_lvl1, 2))
        .unwrap();

    // L3 前置事件
    let l3_pre = EventData {
        start_time: TimeDelta::try_milliseconds(12000).unwrap(),
        end_time: TimeDelta::try_milliseconds(12500).unwrap(),
        actor: None,
        text: "L3 前置事件".to_string(),
        segment_timing: None,
        style: "Default".to_string(),
    };
    sequence
        .create_event(l3_pre, ItemLocation::InGroup(nested_group_lvl2, 0))
        .unwrap();
    // L3 分组
    let nested_group_lvl3 = sequence
        .create_group(
            GroupData {
                name: "循环测试 L3".to_string(),
                collapsed: false,
                color: None,
                children: Vec::new(),
            },
            ItemLocation::InGroup(nested_group_lvl2, 1),
        )
        .unwrap();
    // L3 后置事件
    let l3_post = EventData {
        start_time: TimeDelta::try_milliseconds(12500).unwrap(),
        end_time: TimeDelta::try_milliseconds(13000).unwrap(),
        actor: None,
        text: "L3 后置事件".to_string(),
        segment_timing: None,
        style: "Default".to_string(),
    };
    sequence
        .create_event(l3_post, ItemLocation::InGroup(nested_group_lvl2, 2))
        .unwrap();

    // 在最内层嵌套分组中循环创建 10 行字幕
    for i in 0..10 {
        let start = 13000 + i * 500;
        let end = 13000 + (i + 1) * 500;
        let event = EventData {
            start_time: TimeDelta::try_milliseconds(start).unwrap(),
            end_time: TimeDelta::try_milliseconds(end).unwrap(),
            actor: None,
            text: format!("循环测试字幕 L3-{}", i + 1),
            segment_timing: None,
            style: "Default".to_string(),
        };
        sequence
            .create_event(event, ItemLocation::InGroup(nested_group_lvl3, i as usize))
            .unwrap();
    }

    let event3 = EventData {
        start_time: TimeDelta::try_milliseconds(60000).unwrap(),
        end_time: TimeDelta::try_milliseconds(65000).unwrap(),
        actor: None,
        text: "这是分组内的第二行".to_string(),
        segment_timing: None,
        style: "Default".to_string(),
    };
    sequence
        .create_event(event3, ItemLocation::InGroup(group_id, 2))
        .unwrap();

    // 再创建一个根级别的事件
    let event4 = EventData {
        start_time: TimeDelta::try_milliseconds(65000).unwrap(),
        end_time: TimeDelta::try_milliseconds(70000).unwrap(),
        actor: None,
        text: "又一行根级别字幕".to_string(),
        segment_timing: None,
        style: "Default".to_string(),
    };
    sequence
        .create_event(event4, ItemLocation::Root(2))
        .unwrap();
}
