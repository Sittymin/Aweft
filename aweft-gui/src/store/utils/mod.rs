//! 通用逻辑文件夹：框选、选中修饰键、拖动校验等跨编辑器复用工具。
//! 新增功能前请先在此查找是否已有通用函数，避免重复造轮子。
//! - `marquee`  : 时间/值归一化、行区间 clamp（曲线/摄影表/时间线/NLA 复用）
//! - `selection`: 修饰键语义 `is_replacing`/`is_additive`/`is_toggling`（点选/框选全局复用）
//! - `drag`     : 拖动增量校验（字幕行时间拖动、NLA 片断拖动、Area 拆分合并共用）
//! - `rainbow`  : 自动彩虹调色板（曲线编辑器通道色 / 多视频控制时间条共用）

pub mod drag;
pub mod marquee;
pub mod rainbow;
pub mod selection;
