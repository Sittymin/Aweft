use crate::library::{Library, MessageLevel};
use crate::renderer::{DefaultFontProvider, RenderResult, Renderer};
use crate::track::{Track, TrackType};
use crate::types::Color;
use anyhow::Result;
use std::path::Path;
use std::sync::Arc;

mod ffi;
mod image;
mod library;
mod renderer;
mod track;
mod types;

/// 获取 libass 版本（保持向后兼容）
pub fn version() -> i32 {
    Library::version()
}

// TODO: 创建一个存储所有数据的结构体，然后将几个函数实现
// 1. ass_track_set_feature 限制：只能在调用 ass_render_frame 之前使用

pub struct SubtitleRenderer {
    library: Arc<Library>,
    renderer: Renderer,
}

impl SubtitleRenderer {
    pub fn new(width: i32, height: i32) -> Result<Self> {
        let library = Arc::new(Library::new()?);

        let mut renderer = Renderer::new(Arc::clone(&library))?;

        // NOTE: 如果 aweft-ass 当中存在定义分辨率不一致，需要先**应用**好缩放然后传递过来
        renderer.set_storage_size(width, height);
        renderer.set_frame_size(width, height);
        renderer.set_fonts(None, None, DefaultFontProvider::AutoDetect, None)?;
        Ok(Self { library, renderer })
    }

    // NOTE: 这里之前也要应用一下缩放，切换视频源应该不常见
    pub fn set_frame_size(&mut self, width: i32, height: i32) -> &mut Self {
        self.renderer.set_storage_size(width, height);
        self.renderer.set_frame_size(width, height);
        self
    }

    pub fn set_fonts(
        &mut self,
        default_font: Option<&Path>,
        default_family: Option<&str>,
        font_provider: DefaultFontProvider,
        config: Option<&Path>,
    ) -> Result<&mut Self> {
        self.renderer
            .set_fonts(default_font, default_family, font_provider, config)?;
        Ok(self)
    }

    pub fn create_track(&self) -> Result<Track> {
        Track::new(Arc::clone(&self.library))
    }

    pub fn render_frame(&mut self, track: &Track, timestamp: i64) -> Result<RenderResult> {
        self.renderer.render_frame(track, timestamp)
    }

    /// 设置消息回调函数的便利方法
    pub fn set_message_callback<F>(&mut self, callback: F) -> Result<()>
    where
        F: Fn(MessageLevel, &str) + Send + Sync + 'static,
    {
        self.library.set_message_callback(callback)
    }

    /// 清除消息回调函数的便利方法
    pub fn clear_message_callback(&mut self) -> Result<()> {
        self.library.clear_message_callback()
    }
}

#[cfg(test)]
mod tests {
    use crate::track::Alignment;

    use super::*;

    #[test]
    fn test_libass_version() {
        let ver = version();
        println!("libass version: {:#08x}", ver);
        assert!(ver > 0);
    }

    #[test]
    fn test_color_conversion() {
        let color = Color::new(255, 128, 64, 192);
        let libass_format = color.to_libass_format();
        let converted_back = Color::from_libass_format(libass_format);

        assert_eq!(color, converted_back);

        // 测试已知值
        assert_eq!(Color::red().to_libass_format(), 0xFF0000FF);
        assert_eq!(Color::green().to_libass_format(), 0x00FF00FF);
        assert_eq!(Color::blue().to_libass_format(), 0x0000FFFF);
    }

    #[test]
    fn test_manual_workflow() {
        // 测试手动工作流程：使用默认样式 + alloc_event
        let library = Arc::new(Library::new().expect("创建库失败"));

        let mut renderer = Renderer::new(Arc::clone(&library)).expect("创建渲染器失败");
        renderer.set_storage_size(1280, 720);
        renderer.set_frame_size(1280, 720);
        // 这个是必须设置的
        renderer
            .set_fonts(None, None, DefaultFontProvider::AutoDetect, None)
            .expect("设置字体失败");

        let mut track = Track::new(Arc::clone(&library)).expect("创建轨道失败");
        track.set_play_resolution(1280, 720);
        track.set_track_type(TrackType::Ass);

        // 使用 libass 自动创建的默认样式（新的安全API）
        let default_style_id = {
            let style = track.default_style_mut().expect("获取默认样式失败");
            let style = style
                .set_font_size(48.0)
                .set_primary_color(Color::new(255, 255, 255, 255))
                .set_outline_color(Color::black())
                .set_outline(2.0)
                .set_shadow(3.0)
                .set_alignment(Alignment::TopCenter);
            style.id()
        };

        // 手动创建事件（新的安全API）
        let _event = {
            let event = track
                .create_event("手动工作流程测试", default_style_id)
                .expect("创建事件失败");
            event.set_timing(0, 3000)
        };

        // 渲染
        let result = renderer.render_frame(&track, 1500).expect("渲染失败");

        assert!(!result.is_empty(), "手动工作流程应该生成图像");
        let image_count = result.iter().count();
        println!("✅ 手动工作流程成功生成了 {} 个图像层", image_count);
        assert!(image_count > 0);
    }

    //     #[test]
    //     fn test_ass_script_workflow() {
    //         // 测试 ASS 脚本工作流程：process_data
    //         let mut library = Library::new().expect("创建库失败");
    //         library.set_extract_fonts(true);

    //         let mut renderer = Renderer::new(&library).expect("创建渲染器失败");
    //         renderer.set_storage_size(1280, 720);
    //         renderer.set_frame_size(1280, 720);
    //         renderer
    //             .set_fonts(None, None, DefaultFontProvider::AutoDetect, None)
    //             .expect("设置字体失败");

    //         let mut track = Track::new(library).expect("创建轨道失败");
    //         track.set_play_resolution(1280, 720);
    //         track.set_track_type(TrackType::Ass);

    //         // 使用完整的 ASS 脚本
    //         let ass_script = r#"[V4+ Styles]
    // Format: Name, Fontname, Fontsize, PrimaryColour, SecondaryColour, OutlineColour, BackColour, Bold, Italic, Underline, StrikeOut, ScaleX, ScaleY, Spacing, Angle, BorderStyle, Outline, Shadow, Alignment, MarginL, MarginR, MarginV, Encoding
    // Style: Default,,48,&H00FF0000,&H000000FF,&H00000000,&H80000000,0,0,0,0,100,100,0,0,1,2,3,2,20,20,20,1

    // [Events]
    // Format: Layer, Start, End, Style, Name, MarginL, MarginR, MarginV, Effect, Text
    // Dialogue: 0,0:00:00.00,0:00:05.00,Default,,0,0,0,,Hello, World!"#;

    //         track.process_data(ass_script);

    //         // 渲染
    //         let result = renderer.render_frame(&track, 2500).expect("渲染失败");

    //         assert!(!result.is_empty(), "ASS 脚本工作流程应该生成图像");
    //         let image_count = result.iter().count();
    //         println!("✅ ASS 脚本工作流程成功生成了 {} 个图像层", image_count);
    //         assert!(image_count > 0);
    //     }

    //     #[test]
    //     fn test_message_callback() {
    //         use std::sync::{Arc, Mutex};

    //         // 测试消息回调功能
    //         let messages = Arc::new(Mutex::new(Vec::<(MessageLevel, String)>::new()));
    //         let messages_clone = messages.clone();

    //         let mut library = Library::new().expect("创建库失败");

    //         // 先设置回调，再进行其他操作
    //         library
    //             .set_message_callback(move |level, message| {
    //                 println!(
    //                     "📨 [test_message_callback] 收到消息 [{:?}]: {}",
    //                     level, message
    //                 );
    //                 if let Ok(mut msgs) = messages_clone.lock() {
    //                     msgs.push((level, message.to_string()));
    //                 }
    //             })
    //             .expect("设置消息回调失败");

    //         let mut renderer = Renderer::new(&library).expect("创建渲染器失败");
    //         renderer.set_storage_size(1280, 720);
    //         renderer.set_frame_size(1280, 720);

    //         // 主动触发字体设置来产生消息
    //         renderer
    //             .set_fonts(None, None, DefaultFontProvider::AutoDetect, None)
    //             .expect("设置字体失败");

    //         // 触发一些 libass 操作来生成消息
    //         let mut track = Track::new(library.clone()).expect("创建轨道失败");
    //         track.set_track_type(TrackType::Ass);

    //         let ass_script = r#"[V4+ Styles]
    // Format: Name, Fontname, Fontsize, PrimaryColour, SecondaryColour, OutlineColour, BackColour, Bold, Italic, Underline, StrikeOut, ScaleX, ScaleY, Spacing, Angle, BorderStyle, Outline, Shadow, Alignment, MarginL, MarginR, MarginV, Encoding
    // Style: Default,,20,&H00ffffff,&H000000ff,&H00000000,&H80000000,0,0,0,0,100,100,0,0,1,2,0,2,10,10,10,1

    // [Events]
    // Format: Layer, Start, End, Style, Name, MarginL, MarginR, MarginV, Effect, Text
    // Dialogue: 0,0:00:01.00,0:00:05.00,Default,,0,0,0,,消息回调测试"#;

    //         track.process_data(ass_script);
    //         let _result = renderer.render_frame(&track, 3000);

    //         // 检查是否收到了消息
    //         let collected_messages = messages.lock().unwrap();
    //         println!(
    //             "📊 [test_message_callback] 收到 {} 条 libass 消息",
    //             collected_messages.len()
    //         );

    //         // 清除回调
    //         library.clear_message_callback().expect("清除回调失败");

    //         println!("✅ 消息回调测试完成");
    //     }

    //     #[test]
    //     fn test_message_callback_detailed() {
    //         use std::sync::{Arc, Mutex};

    //         // 测试消息回调功能 - 更详细的版本
    //         let messages = Arc::new(Mutex::new(Vec::<(MessageLevel, String)>::new()));
    //         let messages_clone = messages.clone();

    //         let mut library = Library::new().expect("创建库失败");

    //         // 先设置回调，再进行其他操作
    //         library
    //             .set_message_callback(move |level, message| {
    //                 println!(
    //                     "📨 [test_message_callback_detailed] 收到消息 [{:?}]: {}",
    //                     level, message
    //                 );
    //                 if let Ok(mut msgs) = messages_clone.lock() {
    //                     msgs.push((level, message.to_string()));
    //                 }
    //             })
    //             .expect("设置消息回调失败");

    //         // 设置一些可能触发消息的操作
    //         library.set_extract_fonts(true);

    //         let mut renderer = Renderer::new(&library).expect("创建渲染器失败");
    //         renderer.set_storage_size(1280, 720);
    //         renderer.set_frame_size(1280, 720);

    //         // 尝试设置一个可能不存在的字体路径来触发警告
    //         let _ = renderer.set_fonts(
    //             Some(Path::new("/nonexistent/font.ttf")),
    //             Some("NonexistentFont"),
    //             DefaultFontProvider::AutoDetect,
    //             None,
    //         );

    //         let mut track = Track::new(library.clone()).expect("创建轨道失败");
    //         track.set_track_type(TrackType::Ass);

    //         // 故意使用一个可能有问题的 ASS 脚本来触发警告
    //         let problematic_ass_script = r#"[V4+ Styles]
    // Format: Name, Fontname, Fontsize, PrimaryColour, SecondaryColour, OutlineColour, BackColour, Bold, Italic, Underline, StrikeOut, ScaleX, ScaleY, Spacing, Angle, BorderStyle, Outline, Shadow, Alignment, MarginL, MarginR, MarginV, Encoding
    // Style: Default,NonexistentFont,20,&H00ffffff,&H000000ff,&H00000000,&H80000000,0,0,0,0,100,100,0,0,1,2,0,2,10,10,10,1

    // [Events]
    // Format: Layer, Start, End, Style, Name, MarginL, MarginR, MarginV, Effect, Text
    // Dialogue: 0,0:00:01.00,0:00:05.00,Default,,0,0,0,,测试消息回调 - 详细版本"#;

    //         track.process_data(problematic_ass_script);
    //         let _result = renderer.render_frame(&track, 3000);

    //         // 检查是否收到了消息
    //         let collected_messages = messages.lock().unwrap();
    //         println!(
    //             "📊 [test_message_callback_detailed] 总共收到 {} 条 libass 消息",
    //             collected_messages.len()
    //         );

    //         for (i, (level, msg)) in collected_messages.iter().enumerate() {
    //             println!("  消息 #{}: [{:?}] {}", i + 1, level, msg);
    //         }

    //         // 清除回调
    //         library.clear_message_callback().expect("清除回调失败");

    //         println!("✅ 详细消息回调测试完成");
    //     }
}

// ============================================================================
// 使用示例
// ============================================================================

/// 完整的使用示例，展示新的 Rust 风格 API
pub mod examples {
    use crate::track::Alignment;

    use super::*;

    /// 手动工作流程示例 - 使用新的安全 API
    pub fn manual_workflow_example() -> Result<()> {
        // 创建渲染器
        let mut renderer = SubtitleRenderer::new(1920, 1080)?;
        renderer.set_frame_size(1280, 720);

        // 创建轨道
        let mut track = renderer.create_track()?;
        track.set_play_resolution(1280, 720);
        track.set_track_type(TrackType::Ass);

        // 创建自定义样式
        let custom_style_id = {
            let style = track.create_style("CustomStyle", "Arial")?;
            let style = style
                .set_font_size(48.0)
                .set_primary_color(Color::new(255, 255, 0, 255)) // 黄色
                .set_outline_color(Color::black())
                .set_outline(2.0)
                .set_shadow(1.0)
                .set_alignment(Alignment::BottomCenter); // 底部居中
            style.id()
        };

        // 创建事件
        let _event1 = {
            let event1 = track.create_event("第一行字幕", custom_style_id)?;
            event1.set_timing(0, 3000) // 0-3秒
        };

        let _event2 = {
            let event2 = track.create_event("第二行字幕", custom_style_id)?;
            event2.set_timing(3000, 3000) // 3-6秒
        };

        // 渲染不同时间点的帧
        for timestamp in [1500, 4500] {
            let result = renderer.render_frame(&track, timestamp)?;
            println!(
                "时间戳 {}ms: 渲染了 {} 个图层，变化: {}",
                timestamp,
                result.iter().count(),
                result.has_changed()
            );
        }

        println!("✅ 手动工作流程示例完成");
        Ok(())
    }

    /// ASS 脚本工作流程示例 - 简化版本（仅渲染导向）
    pub fn ass_script_workflow_example() -> Result<()> {
        // 创建渲染器
        let mut renderer = SubtitleRenderer::new(1920, 1080)?;
        renderer.set_frame_size(1280, 720);

        // 创建轨道
        let mut track = renderer.create_track()?;
        track.set_track_type(TrackType::Ass);

        // 使用完整的 ASS 脚本
        let ass_script = r#"[Script Info]
Title: 测试脚本
ScriptType: v4.00+

[V4+ Styles]
Format: Name, Fontname, Fontsize, PrimaryColour, SecondaryColour, OutlineColour, BackColour, Bold, Italic, Underline, StrikeOut, ScaleX, ScaleY, Spacing, Angle, BorderStyle, Outline, Shadow, Alignment, MarginL, MarginR, MarginV, Encoding
Style: Default,Arial,48,&H00FFFF00,&H000000FF,&H00000000,&H80000000,0,0,0,0,100,100,0,0,1,2,1,2,20,20,20,1

[Events]
Format: Layer, Start, End, Style, Name, MarginL, MarginR, MarginV, Effect, Text
Dialogue: 0,0:00:00.00,0:00:05.00,Default,,0,0,0,,ASS 脚本工作流程测试
Dialogue: 0,0:00:05.00,0:00:10.00,Default,,0,0,0,,第二行字幕"#;

        // 处理 ASS 脚本
        track.process_data(ass_script)?;

        // 检查解析结果（基本统计信息）
        println!("解析后的轨道信息:");
        // 解析完成后可以直接渲染，无需检查内部状态
        println!("  样式数量: {}", track.style_count());
        println!("  事件数量: {}", track.event_count());

        // 渲染 - 这是这个绑定的主要目的
        let result = renderer.render_frame(&track, 2500)?;
        println!("渲染结果: {} 个图层", result.iter().count());

        println!("✅ ASS 脚本工作流程示例完成");
        Ok(())
    }

    /// 样式管理示例
    pub fn style_management_example() -> Result<()> {
        let mut renderer = SubtitleRenderer::new(1920, 1080)?;
        let mut track = renderer.create_track()?;
        track.set_play_resolution(1280, 720);
        track.set_track_type(TrackType::Ass);

        // 创建多个样式
        let (title_style_id, subtitle_style_id) = {
            let title_style = track.create_style("Title", "Arial")?;
            let title_style = title_style
                .set_font_size(64.0)
                .set_primary_color(Color::new(255, 255, 255, 255)) // 白色
                .set_outline_color(Color::black())
                .set_outline(3.0)
                .set_bold(true)
                .set_alignment(Alignment::BottomCenter); // 底部居中
            let title_id = title_style.id();

            let subtitle_style = track.create_style("Subtitle", "Arial")?;
            let subtitle_style = subtitle_style
                .set_font_size(36.0)
                .set_primary_color(Color::new(255, 255, 0, 255)) // 黄色
                .set_outline_color(Color::new(0, 0, 0, 128)) // 半透明黑色
                .set_outline(2.0)
                .set_alignment(Alignment::BottomCenter);
            let subtitle_id = subtitle_style.id();

            (title_id, subtitle_id)
        };

        // 创建使用不同样式的事件
        let _title_event = {
            let title_event = track.create_event("电影标题", title_style_id)?;
            title_event.set_timing(0, 5000)
        };

        let _subtitle_event = {
            let subtitle_event = track.create_event("这是字幕文本", subtitle_style_id)?;
            subtitle_event.set_timing(1000, 4000)
        };

        println!(
            "创建了 {} 个样式和 {} 个事件",
            track.style_count(),
            track.event_count()
        );

        // 渲染并检查结果
        let result = renderer.render_frame(&track, 2500)?;
        println!("渲染了 {} 个图层", result.iter().count());

        println!("✅ 样式管理示例完成");
        Ok(())
    }
}

// ============================================================================
