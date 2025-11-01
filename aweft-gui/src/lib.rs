pub mod actions;
pub mod app;
pub mod bridge;
pub mod screen;
pub mod slint_video_sink;
pub mod store;
pub mod types;
pub mod ui_update;
mod wgpu_subtitle_renderer;

use anyhow::{Context, Result};
use screen::area::ScrAreaKey;
use screen::primitives::Adjacency;
use screen::vertex::ScrVertKey;
use types::*;

/// 拖拽操作的状态信息
#[derive(Debug, Clone)]
pub enum DragOperation {
    /// 无拖拽操作
    None,
    /// 分割操作 - 存储分割的源区域和预览数据
    Split {
        source_area: ScrAreaKey,
        preview_data: SplitPreviewOverlayData,
    },
    /// 合并操作 - 存储源区域、目标区域和邻接关系
    Join {
        source_area_key: ScrAreaKey,
        target_area_key: ScrAreaKey,
        adjacency: Adjacency,
    },
    /// 边移动操作 - 存储移动状态信息
    MoveEdge {
        /// 轴方向：true=水平轴(改变Y坐标)，false=垂直轴(改变X坐标)
        dir_axis_horizontal: bool,
        /// 原始轴坐标值
        origval: u32,
        /// 向正方向的最大位移距离
        bigger: u32,
        /// 向负方向的最大位移距离
        smaller: u32,
        /// 参与移动的顶点键集合
        moving_vert_keys: Vec<ScrVertKey>,
    },
}

//         // TODO: WARN: 修改为通用转换函数，或修改 WGPU 直接输出 Slint Image
//         /// 渲染 "Hello World!" 字幕
//         fn render_hello_world_subtitle(
//             state: &mut AppState,
//             instance: &MainWindow,
//             window_width: i32,
//             window_height: i32,
//         ) -> Option<(slint::Image, i32, i32)> {
//             // 分别获取渲染器的引用，避免借用冲突
//             let subtitle_renderer = {
//                 state
//                     .subtitle_renderer
//                     .as_ref()
//                     .map(|renderer| renderer.clone())
//             };

//             let mut subtitle_renderer = match subtitle_renderer {
//                 Some(renderer) => renderer,
//                 Option::None => {
//                     // 字幕渲染器未初始化，跳过
//                     return None;
//                 }
//             };

//             // 获取 libass 渲染器的可变引用
//             let render_result = {
//                 match &mut state.libass_renderer {
//                     Some(libass_renderer) => {
//                         // 设置播放分辨率为实际窗口大小
//                         libass_renderer.set_frame_size(window_width, window_height as i32);

//                         if let Err(e) = libass_renderer.set_default_style_alignment(
//                             aweft_libass::types::Alignment::BottomCenter,
//                         ) {
//                             eprintln!("❌ [ERROR] 设置字幕对齐方式失败: {}", e);
//                             return None;
//                         }

//                         // 使用 libass 创建 "Hello World!" 字幕事件
//                         if let Err(e) =
//                             libass_renderer.create_event_with_default_style("Hello World!")
//                         {
//                             eprintln!("❌ [ERROR] 创建字幕事件失败: {}", e);
//                             return None;
//                         }

//                         // 渲染字幕 (时间戳设为 1000ms，确保字幕显示)
//                         match libass_renderer.render_frame(1000) {
//                             Ok(result) => result,
//                             Err(e) => {
//                                 eprintln!("❌ [ERROR] 渲染字幕失败: {}", e);
//                                 return None;
//                             }
//                         }
//                     }
//                     Option::None => {
//                         eprintln!("❌ [ERROR] libass 渲染器未初始化，跳过");
//                         return None;
//                     }
//                 }
//             };

//             // 检查是否有渲染结果
//             if render_result.is_empty() {
//                 return None;
//             }

//             // 使用 WGPU 渲染器将 libass 结果转换为纹理
//             match subtitle_renderer.render_subtitles_gpu(render_result.iter()) {
//                 Ok(rendered_texture) => {
//                     println!(
//                         "width: {:?}, height: {:?}, x: {:?}, y: {:?}",
//                         rendered_texture.width,
//                         rendered_texture.height,
//                         rendered_texture.x,
//                         rendered_texture.y
//                     );
//                     // 将 WGPU 纹理转换为 Slint Image
//                     match slint::Image::try_from(rendered_texture.texture.clone()) {
//                         Ok(image) => Some((image, rendered_texture.x, rendered_texture.y)),
//                         Err(e) => {
//                             eprintln!("❌ [ERROR] 转换字幕纹理为 Slint Image 失败: {:?}", e);
//                             None
//                         }
//                     }
//                 }
//                 Err(e) => {
//                     eprintln!("❌ [ERROR] WGPU 字幕渲染失败: {}", e);
//                     None
//                 }
//             }
//         }
//     }
// }

// 导出一个公共函数，用于创建和运行窗口
pub fn create_window() -> Result<()> {
    let mut app =
        app::AppWindowManager::new().context("创建应用窗口管理器失败（是否nix develop?）")?;
    app.run().context("运行应用失败")
}
