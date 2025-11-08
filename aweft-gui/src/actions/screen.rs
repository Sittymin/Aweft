/// 屏幕/窗口相关 Action
#[derive(Debug, Clone)]
pub enum ScreenAction {
    /// 窗口大小改变
    WindowResized { width: u32, height: u32 },
    /// 缩放因子改变
    ScaleFactorChanged { scale_factor: f64 },
    /// 初始化布局
    InitializeLayout { width: u32, height: u32 },
}
