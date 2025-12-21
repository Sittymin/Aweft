use crate::bridge::SubtitleState;
use crate::bridge::types::MainWindow;
use crate::store::AppStore;
use crate::store::reducers::media::initialize_media;
use anyhow::{Context, Result};
use slint::ComponentHandle;
use slint::wgpu_27::{WGPUConfiguration, WGPUSettings};
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
                .require_wgpu_27(WGPUConfiguration::Automatic(WGPUSettings::default()))
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
            let state = store_mut.state_mut();
            // 先创建数据
            create_test_subtitle_data(&mut state.project);

            // 然后初始化 LineEditorState
            state.line_editor_state.rebuild_all_rows(&state.project);
            state.line_editor_state.init_filter_model()
        };

        let subtitle_state = instance.global::<SubtitleState>();
        // NOTE: 之后 UI 数据需要在 filtered_model 修改
        // 不应当再使用 set_rows 修改数据
        subtitle_state.set_rows(filter_model);

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
    let group_id = structure
        .create_group("对话组".to_string(), ItemLocation::Root(1))
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

    // 创建一个嵌套分组
    let nested_group_id = structure
        .create_group("循环测试".to_string(), ItemLocation::InGroup(group_id, 1))
        .unwrap();

    // 在嵌套分组中循环创建100行字幕
    for i in 0..10000 {
        let start = 10000 + i * 500;
        let end = 10000 + (i + 1) * 500;
        let event = EventData {
            start_time: TimeDelta::try_milliseconds(start).unwrap(),
            end_time: TimeDelta::try_milliseconds(end).unwrap(),
            actor: None,
            text: format!("循环测试字幕 {}", i + 1),
            segment_timing: None,
            style: "Default".to_string(),
        };
        structure
            .create_event(event, ItemLocation::InGroup(nested_group_id, i as usize))
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

    println!("✅ 已创建测试字幕数据：10004个事件，2个分组");
}
