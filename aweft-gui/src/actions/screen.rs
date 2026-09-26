use crate::screen::primitives::Insets;

/// 屏幕/窗口相关 Action
#[derive(Debug, Clone)]
pub enum ScreenAction {
    /// 窗口大小改变
    WindowResized { width: u32, height: u32 },
    /// 缩放因子改变
    ScaleFactorChanged { scale_factor: f64 },
    /// 初始化布局：窗口物理尺寸 + 缩放因子 + 四周系统栏内缩。
    /// 缩放因子必须随本 Action 一起给：`ThemeSettings` 在窗口创建时读，
    /// Android 上那一刻 `window().scale_factor()` 还是 1.0，真值要到 RenderingSetup 才有。
    InitializeLayout {
        width: u32,
        height: u32,
        scale_factor: f64,
        inset: Insets,
    },
}
