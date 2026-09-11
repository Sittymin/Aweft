//! 跨平台文件选择：桌面调 `pick_video_file`，Android 调 `android` 子模块。
//!
//! - 桌面：rfd 系统对话框（原逻辑原样搬过来，行为不变）。
//! - Android：MediaStore 原生媒体库 + Slint 自绘选择弹层，见 `android` 子模块。
//!   媒体库给的是 `content://` 地址，GStreamer 读不了，统一拷到应用私有缓存
//!   再返回本地路径，下游（URI 组装、dispatch）与桌面走同一条路。
//!   对话框要弹层配合（实例与 store 都在 callbacks 手里），所以 Android 没有
//!   无参的 `pick_video_file`，由 callbacks 经 `android::run_android_picker` 编排。

#[cfg(target_os = "android")]
pub mod android;

#[cfg(not(target_os = "android"))]
use std::path::PathBuf;

#[cfg(not(target_os = "android"))]
const VIDEO_EXTENSIONS: &[&str] = &["mp4", "mkv", "mov", "avi", "webm", "m4v"];

/// 打开视频：返回用户选中的本地文件路径，取消返回 `None`（仅桌面）。
#[cfg(not(target_os = "android"))]
pub async fn pick_video_file() -> Option<PathBuf> {
    rfd::AsyncFileDialog::new()
        .add_filter("视频文件", VIDEO_EXTENSIONS)
        .pick_file()
        .await
        .map(|file| file.path().to_path_buf())
}
