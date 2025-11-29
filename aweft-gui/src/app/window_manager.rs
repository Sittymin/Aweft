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
        initialize_media(
            &mut store.borrow_mut(),
            "file:///home/Sittymin/Videos/IMG_0161.MP4",
        )
        .unwrap();

        // ===================================
        // 初始化字幕数据和过滤模型
        let filter_model = {
            let mut store_mut = store.borrow_mut();
            let line_editor_state = &mut store_mut.state_mut().line_editor_state;

            create_test_subtitle_data(line_editor_state);
            line_editor_state.rebuild_all_rows();
            line_editor_state.init_filter_model()
        };

        let subtitle_state = instance.global::<SubtitleState>();
        // NOTE: 之后 UI 数据需要在 filtered_model 修改
        // 不应当再使用 set_rows 修改数据
        subtitle_state.set_rows(filter_model);

        println!("✅ 已初始化字幕过滤模型");
        // ===================================

        Ok(Self { instance, store })
    }

    /// 获取 MainWindow 的引用
    pub fn get_instance(&self) -> &MainWindow {
        &self.instance
    }

    /// 运行应用
    pub fn run(&mut self) -> Result<()> {
        crate::app::callbacks::setup_callbacks(&self.instance, Rc::clone(&self.store));

        // 直接启动 UI 事件循环
        self.instance.run().context("启动 UI 事件循环失败")
    }
}

/// 创建测试字幕数据
fn create_test_subtitle_data(line_editor_state: &mut crate::store::subtitle::LineEditorState) {
    use aweft_ass::types::event::{Dialogue, Event, TextSegment};
    use aweft_ass::types::time::AssTime;

    // 创建根级别的事件
    let event1 = Event::Dialogue(Dialogue {
        layer: 0,
        start: AssTime::from_centiseconds(0),
        end: AssTime::from_centiseconds(500),
        style: "Default".to_string(),
        name: None,
        margin_l: 0,
        margin_r: 0,
        margin_v: 0,
        effect: None,
        content: vec![TextSegment::Text("这是第一行字幕（根级别）".to_string())],
    });
    let key1 = line_editor_state.ass_file.events.push(event1);
    line_editor_state.push_event_to_root(key1);

    // 创建一个分组
    let group_id = line_editor_state.push_group_to_root("对话组".to_string());

    // 在分组内创建事件
    let event2 = Event::Dialogue(Dialogue {
        layer: 0,
        start: AssTime::from_centiseconds(500),
        end: AssTime::from_centiseconds(1000),
        style: "Default".to_string(),
        name: None,
        margin_l: 0,
        margin_r: 0,
        margin_v: 0,
        effect: None,
        content: vec![TextSegment::Text("这是分组内的第一行".to_string())],
    });
    let key2 = line_editor_state.ass_file.events.push(event2);
    line_editor_state.push_event_to_group(key2, group_id);

    // 创建一个嵌套分组
    let nested_group_id =
        line_editor_state.insert_group_to_group("循环测试".to_string(), group_id, 1);

    // 在嵌套分组中循环创建100行字幕
    for i in 0..10000 {
        let event = Event::Dialogue(Dialogue {
            layer: 0,
            start: AssTime::from_centiseconds(1000 + i * 50),
            end: AssTime::from_centiseconds(1000 + (i + 1) * 50),
            style: "Default".to_string(),
            name: None,
            margin_l: 0,
            margin_r: 0,
            margin_v: 0,
            effect: None,
            content: vec![TextSegment::Text(format!("循环测试字幕 {}", i + 1))],
        });
        let key = line_editor_state.ass_file.events.push(event);
        line_editor_state.push_event_to_group(key, nested_group_id);
    }

    let event3 = Event::Dialogue(Dialogue {
        layer: 0,
        start: AssTime::from_centiseconds(6000),
        end: AssTime::from_centiseconds(6500),
        style: "Default".to_string(),
        name: None,
        margin_l: 0,
        margin_r: 0,
        margin_v: 0,
        effect: None,
        content: vec![TextSegment::Text("这是分组内的第二行".to_string())],
    });
    let key3 = line_editor_state.ass_file.events.push(event3);
    line_editor_state.push_event_to_group(key3, group_id);

    // 再创建一个根级别的事件
    let event4 = Event::Dialogue(Dialogue {
        layer: 0,
        start: AssTime::from_centiseconds(6500),
        end: AssTime::from_centiseconds(7000),
        style: "Default".to_string(),
        name: None,
        margin_l: 0,
        margin_r: 0,
        margin_v: 0,
        effect: None,
        content: vec![TextSegment::Text("又一行根级别字幕".to_string())],
    });
    let key4 = line_editor_state.ass_file.events.push(event4);
    line_editor_state.push_event_to_root(key4);

    println!("✅ 已创建测试字幕数据：104个事件，2个分组");
}
