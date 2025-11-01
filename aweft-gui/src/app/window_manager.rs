use crate::store::AppStore;
use crate::store::reducers::media::initialize_media;
use crate::types::MainWindow;
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
        let store = Rc::new(RefCell::new(AppStore::new(&instance.window())));
        // HACK: 临时直接加载一个视频，并且失败直接崩溃
        initialize_media(
            &mut store.borrow_mut(),
            "file:///home/Sittymin/Videos/IMG_0161.MP4",
        )
        .unwrap();

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
