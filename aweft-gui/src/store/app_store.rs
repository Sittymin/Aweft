use crate::actions::Action;
use crate::clipboard::Clipboard;
use crate::depsgraph::{Depsgraph, Tag};
use crate::store::state::AppState;
use crate::store::ui_models::UiModels;
use anyhow::Result;

/// 应用状态存储 - 单一数据源
pub struct AppStore {
    pub(crate) state: AppState,
    pub(crate) ui_models: UiModels,
    pub(crate) depsgraph: Depsgraph,
    /// OverlayState global 弱引用：overlay 投影经此写出拖拽预览。
    /// 在 `AppStore::new` 中从 MainWindow 注入。
    pub(crate) overlay_state: Option<slint::Weak<crate::bridge::types::OverlayState<'static>>>,
    /// AnimationState global 弱引用：动画投影经此同步标量/重新绑定过滤视图到 Slint。
    pub(crate) animation_state: Option<slint::Weak<crate::bridge::types::AnimationState<'static>>>,
    /// VideoState global 弱引用：视频投影经此同步选中、主视频和播放标量到 Slint。
    pub(crate) video_state: Option<slint::Weak<crate::bridge::types::VideoState<'static>>>,
    /// 主窗口弱引用：视频输出后端（wayland 原始句柄/帧通知）按需升级使用。
    pub(crate) main_window: Option<slint::Weak<crate::bridge::types::MainWindow>>,
    pub(crate) clipboard: Clipboard,
    /// 外来写入的排队句柄：`project_watch` 的订阅者往里放，`dispatch` 收尾时取走换 tag。
    pub(crate) refresh_queue: crate::store::project_watch::RefreshQueue,
}

impl AppStore {
    /// 创建新的 Store（从 MainWindow 以访问主题系统）
    pub fn new(main_window: &crate::bridge::types::MainWindow) -> Self {
        use slint::{ComponentHandle, Global};

        let theme_settings = crate::bridge::types::ThemeSettings::from_main_window(main_window);
        let mut depsgraph = Depsgraph::new();
        crate::projections::register_all(&mut depsgraph);
        let overlay_state = main_window
            .global::<crate::bridge::types::OverlayState>()
            .as_weak();
        let animation_state = main_window
            .global::<crate::bridge::types::AnimationState>()
            .as_weak();
        let video_state = main_window
            .global::<crate::bridge::types::VideoState>()
            .as_weak();
        // 订阅者注册要借用工程存储，所以状态先建好、再登记接入方。
        let mut state = AppState::new(&main_window.window(), theme_settings);
        let refresh_queue = crate::store::project_watch::register(&mut state.project);
        Self {
            state,
            ui_models: UiModels::new(),
            depsgraph,
            overlay_state: Some(overlay_state),
            animation_state: Some(animation_state),
            video_state: Some(video_state),
            main_window: Some(main_window.as_weak()),
            clipboard: Clipboard::new(),
            refresh_queue,
        }
    }

    /// 分发 Action：修改数据并向依赖图打 tag。
    pub fn dispatch(&mut self, action: Action) -> Result<()> {
        // 一次交互可能改多行：批量作用域把其间的写入合成一次广播。
        let batch = self.state.project.begin_batch();
        let outcome = self.dispatch_action(action);
        // 无论成败都要收口：`hold` 落不回零，之后的写入就永远不再通知订阅者。
        self.state.project.end_batch(batch);
        // 报错前已落进真相的写入同样要刷出来，所以队列先于判错处理。
        self.apply_refresh_queue();
        for tag in outcome? {
            self.depsgraph.tag(tag);
        }
        Ok(())
    }

    fn dispatch_action(&mut self, action: Action) -> Result<Vec<Tag>> {
        match action {
            Action::Screen(screen_action) => self.dispatch_screen(screen_action),
            Action::Drag(drag_action) => self.dispatch_drag(drag_action),
            Action::Area(area_action) => self.dispatch_area(area_action),
            Action::Media(act) => self.dispatch_media(act),
            Action::Sequence(act) => self.dispatch_sequence(act),
            Action::NodeGraph(act) => self.dispatch_node_graph(act),
            Action::Animation(act) => self.dispatch_animation(act),
        }
    }

    /// 把排队的外来写入换成刷新 tag。
    /// 界面自己的写入不在此列：那些 tag 由 reducer 当场打。
    pub(crate) fn apply_refresh_queue(&mut self) {
        let receipts = self.refresh_queue.take();
        if receipts.is_empty() {
            return;
        }
        for tag in crate::store::project_watch::refresh_tags(self, &receipts) {
            self.depsgraph.tag(tag);
        }
    }

    /// 把已挂起的依赖图标记应用到 UI 投影。
    ///
    /// 通过 `mem::take` 暂时取出 depsgraph，避免 `&mut self` 与 `&mut self.depsgraph`
    /// 同时借用的冲突（投影重算需要访问 store 的其他部分）。
    pub fn evaluate_depsgraph(&mut self) {
        if !self.depsgraph.has_pending() {
            return;
        }
        let mut depsgraph = std::mem::take(&mut self.depsgraph);
        depsgraph.evaluate(self);
        self.depsgraph = depsgraph;
    }

    /// 获取当前状态（只读）
    pub fn state(&self) -> &AppState {
        &self.state
    }

    /// 获取可变状态（内部使用）
    pub fn state_mut(&mut self) -> &mut AppState {
        &mut self.state
    }

    /// 获取 UI 模型（只读）
    pub fn ui_models(&self) -> &UiModels {
        &self.ui_models
    }

    /// 获取 UI 模型（可变）
    pub fn ui_models_mut(&mut self) -> &mut UiModels {
        &mut self.ui_models
    }

    /// 同时获取只读 `state` 与可变 `ui_models`。
    ///
    /// 用于需要“读取 Domain、写入 UI Projection”的场景（典型是投影重算应用 Sequence 更新）。
    pub fn split_mut(&mut self) -> (&AppState, &mut UiModels) {
        (&self.state, &mut self.ui_models)
    }

    /// 初始化 Sequence 的行数据与过滤模型，返回过滤后的 ModelRc。
    pub fn rebuild_sequence_models(
        &mut self,
    ) -> slint::ModelRc<crate::bridge::types::SlintSubtitleRow> {
        self.ui_models
            .sequence_state
            .initialize_rows(self.state.project.sequence());
        self.ui_models.sequence_state.init_filter_model()
    }
}
