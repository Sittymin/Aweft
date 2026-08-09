use crate::bridge::SubtitleState;
use crate::bridge::types::MainWindow;
use crate::store::AppStore;
use crate::store::reducers::media::initialize_media;
use anyhow::{Context, Result};
use aweft_model::structure::GroupData;
use slint::ComponentHandle;
use slint::wgpu_29::{WGPUConfiguration, WGPUSettings};
use std::{cell::RefCell, rc::Rc};

/// 简化的应用窗口管理器
pub struct AppWindowManager {
    instance: MainWindow,
    store: Rc<RefCell<AppStore>>,
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
                .require_wgpu_29(WGPUConfiguration::Automatic(WGPUSettings::default()))
                .select();
            if let Err(e) = res {
                eprintln!("Slint 后端选择失败: {:?}", e);
                eprintln!("提示: 需要可用的 WGPU 环境与驱动，且请在 nix develop 中运行");
                return Err(e.into());
            }
        }
        let instance = MainWindow::new().context("创建 Slint 主窗口失败")?;
        // FIXME: 这里获得的是未初始化的数据
        let store = Rc::new(RefCell::new(AppStore::new(&instance)));
        // HACK: 临时直接加载一个视频，并且失败直接崩溃
        // initialize_media(
        //     &mut store.borrow_mut(),
        //     "file:///home/Sittymin/Videos/IMG_0161.MP4",
        // )
        // .unwrap();

        // ===================================
        // 初始化字幕数据和过滤模型
        let filter_model = {
            let mut store_mut = store.borrow_mut();
            {
                // 先创建数据
                let state = store_mut.state_mut();
                create_test_subtitle_data(&mut state.project);
            }
            // 然后初始化 LineEditorState（使用 UiModels 持有的模型）
            store_mut.rebuild_line_editor_models()
        };

        // ===================================
        // 初始化节点图编辑器（样式图/应用图）：测试链数据 + 绑定模型到全局
        {
            let mut store_mut = store.borrow_mut();
            create_test_graphs(&mut store_mut.state_mut().project);
            store_mut.initialize_graph_editors(&instance);
        }

        // ===================================
        // 初始化动画编辑器（曲线编辑器/NLA 编辑器）：临时测试数据 + 绑定模型到全局
        {
            let mut store_mut = store.borrow_mut();
            create_test_animation_data(&mut store_mut);
            store_mut.initialize_animation_editors(&instance);
        }

        let subtitle_state = instance.global::<SubtitleState>();
        // NOTE: 之后 UI 数据需要在 filtered_model 修改
        // 不应当再使用 set_rows 修改数据
        subtitle_state.set_rows(filter_model);

        // 初始化模型并绑定给 Slint（占位空数组，后续通过 UIUpdater 刷新）
        {
            let areas_model = store.borrow().ui_models().scrareas();
            let scr_edge_displays_model = store.borrow().ui_models().scredge_displays();
            let scr_edge_touch_areas_model = store.borrow().ui_models().scredge_touch_areas();
            instance.set_scr_areas(slint::ModelRc::from(areas_model));
            instance
                .set_scr_edge_touch_areas_displays(slint::ModelRc::from(scr_edge_displays_model));
            instance.set_scr_edge_touch_areas(slint::ModelRc::from(scr_edge_touch_areas_model));
        }

        println!("✅ 已初始化字幕过滤模型");
        // ===================================
        // 初始化日志
        env_logger::init();

        Ok(Self { instance, store })
    }

    /// 获取 MainWindow 的引用
    pub fn get_instance(&self) -> &MainWindow {
        &self.instance
    }

    /// 运行应用
    pub fn run(&mut self) -> Result<()> {
        crate::bridge::callbacks::setup_callbacks(&self.instance, Rc::clone(&self.store));

        // 直接启动 UI 事件循环
        self.instance.run().context("启动 UI 事件循环失败")
    }
}

/// 创建测试字幕数据
fn create_test_subtitle_data(project: &mut aweft_model::project::Project) {
    use aweft_model::structure::{EventData, ItemLocation};
    use chrono::TimeDelta;

    let structure = &mut project.structure;

    // 创建根级别的事件
    let event1 = EventData {
        start_time: TimeDelta::try_milliseconds(0).unwrap(),
        end_time: TimeDelta::try_milliseconds(5000).unwrap(),
        actor: None,
        text: "这是第一行字幕（根级别）".to_string(),
        segment_timing: None,
        style: "Default".to_string(),
    };
    structure
        .create_event(event1, ItemLocation::Root(0))
        .unwrap();

    // 创建一个分组
    let group = GroupData {
        name: "对话组".to_string(),
        collapsed: false,
        color: None,
        children: Vec::new(),
    };
    let group_id = structure
        .create_group(group, ItemLocation::Root(1))
        .unwrap();

    // 在分组内创建事件
    let event2 = EventData {
        start_time: TimeDelta::try_milliseconds(5000).unwrap(),
        end_time: TimeDelta::try_milliseconds(10000).unwrap(),
        actor: None,
        text: "这是分组内的第一行".to_string(),
        segment_timing: None,
        style: "Default".to_string(),
    };
    structure
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
    structure
        .create_event(l1_pre, ItemLocation::InGroup(group_id, 1))
        .unwrap();
    // L1 分组
    let nested_group_lvl1 = structure
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
    structure
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
    structure
        .create_event(l2_pre, ItemLocation::InGroup(nested_group_lvl1, 0))
        .unwrap();
    // L2 分组
    let nested_group_lvl2 = structure
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
    structure
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
    structure
        .create_event(l3_pre, ItemLocation::InGroup(nested_group_lvl2, 0))
        .unwrap();
    // L3 分组
    let nested_group_lvl3 = structure
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
    structure
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
        structure
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
    structure
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
    structure
        .create_event(event4, ItemLocation::Root(2))
        .unwrap();
}

/// 创建测试节点图数据：一条样式链 + 一条应用链（位置由编辑器自动排布）。
fn create_test_graphs(project: &mut aweft_model::project::Project) {
    let style = crate::store::graph_editor::new_style_chain("样式链 1");
    project.library.node_graphs.insert(style.id, style);

    let app = crate::store::graph_editor::new_app_chain("应用链 1");
    project.style_layer.graphs.push(app);
}

// ── 动画编辑器临时测试数据（UI 先行，未接线 model） ──────────────────────────
// NOTE: 该函数为临时占位，后续接入 aweft-model 的 animation 模块后直接删除，
// 改为从 `Project.library.animations`（或类似源）投影出编辑器状态。

/// 创建曲线编辑器 / NLA 编辑器的测试数据。
fn create_test_animation_data(store: &mut AppStore) {
    use crate::bridge::types::{
        SlintBezierHandle, SlintInterpolation, SlintKeyframe, SlintNlaStrip, SlintStripType,
    };
    use slint::Color;

    fn bez(
        time: f32,
        value: f32,
        in_to: f32,
        in_vo: f32,
        out_to: f32,
        out_vo: f32,
    ) -> SlintKeyframe {
        SlintKeyframe {
            time,
            value,
            interpolation: SlintInterpolation::Bezier,
            handle_in: SlintBezierHandle {
                time_offset: in_to,
                value_offset: in_vo,
            },
            handle_out: SlintBezierHandle {
                time_offset: out_to,
                value_offset: out_vo,
            },
            selected: false,
        }
    }
    fn lin(time: f32, value: f32) -> SlintKeyframe {
        SlintKeyframe {
            time,
            value,
            interpolation: SlintInterpolation::Linear,
            handle_in: SlintBezierHandle {
                time_offset: 0.0,
                value_offset: 0.0,
            },
            handle_out: SlintBezierHandle {
                time_offset: 0.0,
                value_offset: 0.0,
            },
            selected: false,
        }
    }
    fn hold(time: f32, value: f32) -> SlintKeyframe {
        SlintKeyframe {
            time,
            value,
            interpolation: SlintInterpolation::Hold,
            handle_in: SlintBezierHandle {
                time_offset: 0.0,
                value_offset: 0.0,
            },
            handle_out: SlintBezierHandle {
                time_offset: 0.0,
                value_offset: 0.0,
            },
            selected: false,
        }
    }

    // ── 曲线编辑器：三条不同插值的通道 ──
    let curve = &mut store.ui_models_mut().curve_editor;

    curve.push_channel(
        "FontSize",
        Color::from_rgb_u8(242, 109, 120), // 红
        vec![
            bez(0.0, 30.0, 0.0, 0.0, 2.0, 18.0),
            bez(3.0, 66.0, -1.0, -14.0, 1.2, 12.0),
            bez(6.0, 52.0, -0.8, -10.0, 1.0, 16.0),
            bez(9.0, 80.0, -0.9, -14.0, 1.4, 12.0),
            bez(12.0, 44.0, -1.2, -16.0, 0.8, 8.0),
            bez(15.0, 60.0, -0.8, -8.0, 0.0, 0.0),
        ],
    );

    curve.push_channel(
        "PositionX",
        Color::from_rgb_u8(112, 178, 255), // 蓝
        vec![
            lin(0.0, -0.5),
            lin(2.5, 0.6),
            lin(5.0, -0.2),
            lin(7.5, 0.8),
            lin(10.0, -0.6),
            lin(12.5, 0.3),
            lin(15.0, 0.0),
        ],
    );

    curve.push_channel(
        "Rotation",
        Color::from_rgb_u8(240, 193, 78), // 黄
        vec![
            hold(0.0, 0.0),
            hold(4.0, 30.0),
            hold(8.0, 60.0),
            hold(12.0, 90.0),
            hold(15.0, 120.0),
        ],
    );

    curve.push_channel(
        "BorderX",
        Color::from_rgb_u8(99, 199, 99), // 绿
        vec![
            lin(0.0, 2.0),
            lin(3.0, 6.0),
            lin(6.0, 3.0),
            lin(9.0, 8.0),
            lin(12.0, 4.0),
            lin(15.0, 5.0),
        ],
    );

    curve.action_name = "主 Action".into();
    curve.current_time = 3.0;
    curve.rebuild_all();

    // ── NLA 编辑器：三条轨道 ──
    let nla = &mut store.ui_models_mut().nla_editor;
    let action = Color::from_rgb_u8(99, 199, 99);
    let transition = Color::from_rgb_u8(169, 173, 184);
    let meta = Color::from_rgb_u8(112, 178, 255);

    nla.push_track(
        "Base",
        vec![SlintNlaStrip {
            name: "基础动作".into(),
            strip_type: SlintStripType::Action,
            start: 0.0,
            end: 15.0,
            color: action,
            muted: false,
            selected: false,
        }],
    );

    nla.push_track(
        "Idle",
        vec![
            SlintNlaStrip {
                name: "Idle A".into(),
                strip_type: SlintStripType::Action,
                start: 1.0,
                end: 5.0,
                color: action,
                muted: false,
                selected: false,
            },
            SlintNlaStrip {
                name: "过渡".into(),
                strip_type: SlintStripType::Transition,
                start: 5.0,
                end: 7.0,
                color: transition,
                muted: false,
                selected: false,
            },
            SlintNlaStrip {
                name: "Idle B".into(),
                strip_type: SlintStripType::Action,
                start: 7.0,
                end: 12.0,
                color: action,
                muted: false,
                selected: false,
            },
        ],
    );

    nla.push_track(
        "Meta",
        vec![SlintNlaStrip {
            name: "组合片段".into(),
            strip_type: SlintStripType::Meta,
            start: 2.0,
            end: 10.0,
            color: meta,
            muted: false,
            selected: false,
        }],
    );

    nla.current_time = 3.0;
}
