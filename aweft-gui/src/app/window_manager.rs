use crate::bridge::types::MainWindow;
use crate::bridge::{SlintSubtitleRow, SubtitleState};
use crate::store::AppStore;
use crate::store::reducers::media::initialize_media;
use anyhow::{Context, Result};
use slint::wgpu_27::{WGPUConfiguration, WGPUSettings};
use slint::{ComponentHandle, SharedString};
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
        // 创建测试字幕数据
        let test_rows = create_test_subtitle_rows();

        // 设置到Slint
        let subtitle_state = instance.global::<SubtitleState>();
        subtitle_state.set_rows(slint::ModelRc::new(slint::VecModel::from_slice(&test_rows)));

        println!("✅ 已加载 {} 行测试字幕数据", test_rows.len());
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

pub fn create_test_subtitle_rows() -> Vec<SlintSubtitleRow> {
    vec![
        // 根级别事件
        SlintSubtitleRow {
            row_type: crate::bridge::types::SlintSubtitleRowType::Event,
            depth: 0,
            ancestor_groups: slint::ModelRc::new(slint::VecModel::default()),

            event_key: crate::bridge::types::SlintKey {
                key_index: 1,
                key_version: 0,
            },
            start_time: SharedString::from("0:00:00.00"),
            end_time: SharedString::from("0:00:05.00"),
            text: SharedString::from("这是第一行字幕（根级别）"),
            style: SharedString::from("Default"),
            layer: 0,
            is_comment: false,

            group_id_str: SharedString::default(),
            group_name: SharedString::default(),
            collapsed: false,
            total_row_count: 0,
        },
        // 分组头
        SlintSubtitleRow {
            row_type: crate::bridge::types::SlintSubtitleRowType::GroupHeader,
            depth: 0,
            ancestor_groups: slint::ModelRc::new(slint::VecModel::default()),

            event_key: crate::bridge::types::SlintKey {
                key_index: 0,
                key_version: 0,
            },
            start_time: SharedString::default(),
            end_time: SharedString::default(),
            text: SharedString::default(),
            style: SharedString::default(),
            layer: 0,
            is_comment: false,

            group_id_str: SharedString::from("group-1"),
            group_name: SharedString::from("对话组"),
            collapsed: false,
            total_row_count: 3, // 包含分组头自己+2个事件
        },
        // 分组内的事件1
        SlintSubtitleRow {
            row_type: crate::bridge::types::SlintSubtitleRowType::Event,
            depth: 1,
            ancestor_groups: {
                let model = slint::VecModel::default();
                model.push(SharedString::from("group-1"));
                slint::ModelRc::new(model)
            },

            event_key: crate::bridge::types::SlintKey {
                key_index: 2,
                key_version: 0,
            },
            start_time: SharedString::from("0:00:05.00"),
            end_time: SharedString::from("0:00:10.00"),
            text: SharedString::from("这是分组内的第一行"),
            style: SharedString::from("Default"),
            layer: 0,
            is_comment: false,

            group_id_str: SharedString::default(),
            group_name: SharedString::default(),
            collapsed: false,
            total_row_count: 0,
        },
        // 分组内的事件2
        SlintSubtitleRow {
            row_type: crate::bridge::types::SlintSubtitleRowType::Event,
            depth: 1,
            ancestor_groups: {
                let model = slint::VecModel::default();
                model.push(SharedString::from("group-1"));
                slint::ModelRc::new(model)
            },

            event_key: crate::bridge::types::SlintKey {
                key_index: 3,
                key_version: 0,
            },
            start_time: SharedString::from("0:00:10.00"),
            end_time: SharedString::from("0:00:15.00"),
            text: SharedString::from("这是分组内的第二行"),
            style: SharedString::from("Default"),
            layer: 0,
            is_comment: false,

            group_id_str: SharedString::default(),
            group_name: SharedString::default(),
            collapsed: false,
            total_row_count: 0,
        },
        // 根级别事件
        SlintSubtitleRow {
            row_type: crate::bridge::types::SlintSubtitleRowType::Event,
            depth: 0,
            ancestor_groups: slint::ModelRc::new(slint::VecModel::default()),

            event_key: crate::bridge::types::SlintKey {
                key_index: 4,
                key_version: 0,
            },
            start_time: SharedString::from("0:00:15.00"),
            end_time: SharedString::from("0:00:20.00"),
            text: SharedString::from("又一行根级别字幕"),
            style: SharedString::from("Default"),
            layer: 0,
            is_comment: false,

            group_id_str: SharedString::default(),
            group_name: SharedString::default(),
            collapsed: false,
            total_row_count: 0,
        },
    ]
}
