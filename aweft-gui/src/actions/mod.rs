pub mod area;
pub mod drag;
pub mod media;
pub mod screen;

/// 顶层 Action - 按功能域分类
#[derive(Debug, Clone)]
pub enum Action {
    /// 屏幕/窗口相关操作
    Screen(screen::ScreenAction),
    /// 拖拽相关操作
    Drag(drag::DragAction),
    /// 区域相关操作
    Area(area::AreaAction),
    /// 媒体相关操作
    Media(media::MediaAction),
}


