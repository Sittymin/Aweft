use crate::error::Result;
use crate::renderer::Renderer;
use crate::track::Track;
use std::path::Path;
use std::sync::Arc;

pub mod color;
pub mod error;
mod ffi;
pub mod image;
mod library;
mod renderer;
mod track;
pub mod types;

pub use crate::color::{AssColorRgb, AssColorRgba};
pub use crate::error::{AlignmentParseError, LibassError};
pub use crate::image::{Image, ImageType};
pub use crate::library::{Library, MessageLevel};
pub use crate::renderer::{AssStyle, RenderResult};
pub use crate::track::{EventHandle, EventId, EventInfo, StyleHandle, StyleId};
pub use crate::types::{
    Alignment, BorderStyle, DefaultFontProvider, Effect, HintingType, Justify, LibassAlignment,
    OverrideBits, RenderChange, ShapingLevel, TrackFeature, TrackType,
};

pub fn version() -> i32 {
    Library::version()
}

struct RenderCore {
    library: Arc<Library>,
    renderer: Renderer,
    track: Track,
}

/// Mutable access to settings owned by `ASS_Renderer`, kept separate from
/// Track mutation APIs.
pub struct RendererSettings<'a> {
    renderer: &'a mut Renderer,
}

impl RendererSettings<'_> {
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

    pub fn set_shaper(&mut self, level: ShapingLevel) -> &mut Self {
        self.renderer.set_shaper(level);
        self
    }

    pub fn set_margins(&mut self, top: i32, bottom: i32, left: i32, right: i32) -> &mut Self {
        self.renderer.set_margins(top, bottom, left, right);
        self
    }

    pub fn set_use_margins(&mut self, enabled: bool) -> &mut Self {
        self.renderer.set_use_margins(enabled);
        self
    }

    pub fn set_pixel_aspect(&mut self, pixel_aspect: f64) -> &mut Self {
        self.renderer.set_pixel_aspect(pixel_aspect);
        self
    }

    pub fn set_font_scale(&mut self, scale: f64) -> &mut Self {
        self.renderer.set_font_scale(scale);
        self
    }

    pub fn set_hinting(&mut self, hinting: HintingType) -> &mut Self {
        self.renderer.set_hinting(hinting);
        self
    }

    pub fn set_line_spacing(&mut self, spacing: f64) -> &mut Self {
        self.renderer.set_line_spacing(spacing);
        self
    }

    pub fn set_line_position(&mut self, position_percent: f64) -> &mut Self {
        self.renderer.set_line_position(position_percent);
        self
    }

    pub fn set_selective_style_override(
        &mut self,
        style: AssStyle,
        override_bits: OverrideBits,
    ) -> Result<&mut Self> {
        self.renderer
            .selective_style_override(style, override_bits)?;
        Ok(self)
    }

    pub fn set_cache_limits(
        &mut self,
        glyph_max: usize,
        bitmap_max_size: usize,
    ) -> Result<&mut Self> {
        self.renderer.set_cache_limits(glyph_max, bitmap_max_size)?;
        Ok(self)
    }
}

impl RenderCore {
    fn with_track(
        width: i32,
        height: i32,
        create_track: impl FnOnce(Arc<Library>) -> Result<Track>,
    ) -> Result<Self> {
        if width <= 0 || height <= 0 {
            return Err(LibassError::InvalidParameter {
                context: "渲染尺寸必须为正数".to_string(),
            });
        }
        let library = Arc::new(Library::new()?);
        let mut renderer = Renderer::new(Arc::clone(&library))?;
        renderer.set_storage_size(width, height);
        renderer.set_frame_size(width, height);
        renderer.set_fonts(None, None, DefaultFontProvider::AutoDetect, None)?;
        let track = create_track(Arc::clone(&library))?;
        Ok(Self {
            library,
            renderer,
            track,
        })
    }

    fn set_output_size(&mut self, width: i32, height: i32) -> Result<()> {
        if width <= 0 || height <= 0 {
            return Err(LibassError::InvalidParameter {
                context: "渲染尺寸必须为正数".to_string(),
            });
        }
        self.renderer.set_storage_size(width, height);
        self.renderer.set_frame_size(width, height);
        Ok(())
    }

    fn set_fonts(
        &mut self,
        default_font: Option<&Path>,
        default_family: Option<&str>,
        font_provider: DefaultFontProvider,
        config: Option<&Path>,
    ) -> Result<()> {
        self.renderer
            .set_fonts(default_font, default_family, font_provider, config)
    }

    fn render_frame(&mut self, timestamp: i64) -> Result<RenderResult> {
        self.renderer.render_frame(&mut self.track, timestamp)
    }

    fn set_message_callback<F>(&mut self, callback: F) -> Result<()>
    where
        F: Fn(MessageLevel, &str) + Send + Sync + 'static,
    {
        self.library.set_message_callback(callback)
    }

    fn clear_message_callback(&mut self) -> Result<()> {
        self.library.clear_message_callback()
    }
}

/// Renderer for manually constructed styles and events.
///
/// Existing elements are rebuilt transparently when changed after rendering.
/// Appending a style or event remains an in-place libass operation.
pub struct SubtitleRenderer {
    core: RenderCore,
}

impl SubtitleRenderer {
    pub fn new(width: i32, height: i32) -> Result<Self> {
        let mut core = RenderCore::with_track(width, height, Track::new)?;
        core.track.set_play_resolution(width, height)?;
        Ok(Self { core })
    }

    pub fn set_frame_size(&mut self, width: i32, height: i32) -> Result<&mut Self> {
        self.core.track.set_play_resolution(width, height)?;
        self.core.set_output_size(width, height)?;
        Ok(self)
    }

    pub fn set_play_resolution(&mut self, width: i32, height: i32) -> Result<&mut Self> {
        self.core.track.set_play_resolution(width, height)?;
        Ok(self)
    }

    pub fn set_track_type(&mut self, track_type: TrackType) -> Result<&mut Self> {
        self.core.track.set_track_type(track_type)?;
        Ok(self)
    }

    pub fn set_track_feature(&mut self, feature: TrackFeature, enabled: bool) -> Result<&mut Self> {
        self.core.track.set_feature(feature, enabled)?;
        Ok(self)
    }

    pub fn set_default_style_alignment(
        &mut self,
        alignment: types::Alignment,
    ) -> Result<&mut Self> {
        self.core
            .track
            .default_style_mut()?
            .set_alignment(alignment);
        Ok(self)
    }

    pub fn set_fonts(
        &mut self,
        default_font: Option<&Path>,
        default_family: Option<&str>,
        font_provider: DefaultFontProvider,
        config: Option<&Path>,
    ) -> Result<&mut Self> {
        self.core
            .set_fonts(default_font, default_family, font_provider, config)?;
        Ok(self)
    }

    pub fn renderer_settings(&mut self) -> RendererSettings<'_> {
        RendererSettings {
            renderer: &mut self.core.renderer,
        }
    }

    pub fn default_style_mut(&mut self) -> Result<StyleHandle<'_>> {
        self.core.track.default_style_mut()
    }

    pub fn create_style(&mut self, name: &str, font_name: &str) -> Result<StyleHandle<'_>> {
        self.core.track.create_style(name, font_name)
    }

    pub fn style_mut(&mut self, id: StyleId) -> Result<StyleHandle<'_>> {
        self.core.track.style_mut(id)
    }

    pub fn style(&self, id: StyleId) -> Result<AssStyle> {
        self.core.track.style(id)
    }

    pub fn style_count(&self) -> i32 {
        self.core.track.style_count()
    }

    pub fn style_ids(&self) -> impl Iterator<Item = StyleId> {
        self.core.track.style_ids()
    }

    pub fn styles(&self) -> impl Iterator<Item = (StyleId, AssStyle)> + '_ {
        self.core.track.styles()
    }

    pub fn create_event_with_default_style(&mut self, text: &str) -> Result<&mut Self> {
        let style_id = self.core.track.default_style_id()?;
        self.core.track.create_event(text, style_id)?;
        Ok(self)
    }

    pub fn add_event(&mut self, text: &str, start_ms: i64, duration_ms: i64) -> Result<EventId> {
        let style_id = self.core.track.default_style_id()?;
        self.add_event_with_style(text, start_ms, duration_ms, style_id)
    }

    pub fn add_event_with_style(
        &mut self,
        text: &str,
        start_ms: i64,
        duration_ms: i64,
        style_id: StyleId,
    ) -> Result<EventId> {
        if start_ms < 0 || duration_ms < 0 {
            return Err(LibassError::InvalidParameter {
                context: "事件开始时间和持续时间不能为负".to_string(),
            });
        }
        let event = self
            .core
            .track
            .create_event(text, style_id)?
            .set_timing(start_ms, duration_ms);
        Ok(event.id())
    }

    pub fn event_mut(&mut self, id: EventId) -> Result<EventHandle<'_>> {
        self.core.track.event_mut(id)
    }

    pub fn event(&self, id: EventId) -> Result<EventInfo> {
        self.core.track.event(id)
    }

    pub fn remove_event(&mut self, id: EventId) -> Result<()> {
        self.core.track.remove_event(id)
    }

    pub fn event_count(&self) -> i32 {
        self.core.track.event_count()
    }

    pub fn event_ids(&self) -> impl Iterator<Item = EventId> {
        self.core.track.event_ids()
    }

    pub fn events(&self) -> impl Iterator<Item = (EventId, EventInfo)> + '_ {
        self.core.track.events()
    }

    pub fn clear_events(&mut self) {
        self.core.track.flush_events();
    }

    pub fn render_frame(&mut self, timestamp: i64) -> Result<RenderResult> {
        self.core.render_frame(timestamp)
    }

    pub fn set_message_callback<F>(&mut self, callback: F) -> Result<()>
    where
        F: Fn(MessageLevel, &str) + Send + Sync + 'static,
    {
        self.core.set_message_callback(callback)
    }

    pub fn clear_message_callback(&mut self) -> Result<()> {
        self.core.clear_message_callback()
    }
}

/// Renderer for a complete ASS/SSA script. Manual mutation is not exposed.
pub struct AssScriptRenderer {
    core: RenderCore,
}

impl AssScriptRenderer {
    pub fn from_memory(
        width: i32,
        height: i32,
        data: &[u8],
        codepage: Option<&str>,
        extract_fonts: bool,
    ) -> Result<Self> {
        let core = RenderCore::with_track(width, height, |library| {
            Track::from_memory(library, data, codepage, extract_fonts)
        })?;
        Ok(Self { core })
    }

    pub fn from_file(
        width: i32,
        height: i32,
        filename: impl AsRef<Path>,
        codepage: Option<&str>,
        extract_fonts: bool,
    ) -> Result<Self> {
        let data = std::fs::read(filename)?;
        Self::from_memory(width, height, &data, codepage, extract_fonts)
    }

    pub fn set_frame_size(&mut self, width: i32, height: i32) -> Result<&mut Self> {
        self.core.set_output_size(width, height)?;
        Ok(self)
    }

    /// 换脚本字节而不重建渲染器：`Library`（字体数据）与 `Renderer`（字形缓存与
    /// 字体提供器）都保持原样，只有 `ASS_Track` 按新字节重新解析一次。帧尺寸记在
    /// 渲染器上，因此换脚本不会把它带走。
    pub fn load_script(&mut self, data: &[u8], codepage: Option<&str>) -> Result<&mut Self> {
        self.core.track.load_script(data, codepage)?;
        Ok(self)
    }

    pub fn renderer_settings(&mut self) -> RendererSettings<'_> {
        RendererSettings {
            renderer: &mut self.core.renderer,
        }
    }

    pub fn set_track_feature(&mut self, feature: TrackFeature, enabled: bool) -> Result<&mut Self> {
        self.core.track.set_feature(feature, enabled)?;
        Ok(self)
    }

    /// 切换内嵌字体提取；会触发重建以重新解析脚本。
    pub fn set_extract_fonts(&mut self, extract: bool) -> Result<&mut Self> {
        self.core.track.set_extract_fonts(extract)?;
        Ok(self)
    }

    pub fn clear_events(&mut self) {
        self.core.track.flush_events();
    }

    pub fn event_count(&self) -> i32 {
        self.core.track.event_count()
    }

    pub fn render_frame(&mut self, timestamp: i64) -> Result<RenderResult> {
        self.core.render_frame(timestamp)
    }
}

/// Renderer for a Matroska ASS stream. Chunks can be appended after rendering.
pub struct MatroskaSubtitleRenderer {
    core: RenderCore,
}

impl MatroskaSubtitleRenderer {
    pub fn new(width: i32, height: i32, codec_private: &[u8], extract_fonts: bool) -> Result<Self> {
        let core = RenderCore::with_track(width, height, |library| {
            Track::init_matroska(library, codec_private, extract_fonts)
        })?;
        Ok(Self { core })
    }

    pub fn set_frame_size(&mut self, width: i32, height: i32) -> Result<&mut Self> {
        self.core.set_output_size(width, height)?;
        Ok(self)
    }

    pub fn renderer_settings(&mut self) -> RendererSettings<'_> {
        RendererSettings {
            renderer: &mut self.core.renderer,
        }
    }

    pub fn set_track_feature(&mut self, feature: TrackFeature, enabled: bool) -> Result<&mut Self> {
        self.core.track.set_feature(feature, enabled)?;
        Ok(self)
    }

    pub fn set_check_readorder(&mut self, enabled: bool) -> Result<&mut Self> {
        self.core.track.set_check_readorder(enabled)?;
        Ok(self)
    }

    /// 切换内嵌字体提取；会触发重建以重新解析 codec_private。
    pub fn set_extract_fonts(&mut self, extract: bool) -> Result<&mut Self> {
        self.core.track.set_extract_fonts(extract)?;
        Ok(self)
    }

    pub fn process_chunk(
        &mut self,
        data: &[u8],
        timecode_ms: i64,
        duration_ms: i64,
    ) -> Result<&mut Self> {
        self.core
            .track
            .process_chunk(data, timecode_ms, duration_ms)?;
        Ok(self)
    }

    pub fn clear_events(&mut self) {
        self.core.track.flush_events();
    }

    pub fn configure_prune(&mut self, delay_ms: i64) -> &mut Self {
        self.core.track.configure_prune(delay_ms);
        self
    }

    pub fn prune_events(&mut self, deadline_ms: i64) -> &mut Self {
        self.core.track.prune_events(deadline_ms);
        self
    }

    pub fn event_count(&self) -> i32 {
        self.core.track.event_count()
    }

    pub fn render_frame(&mut self, timestamp: i64) -> Result<RenderResult> {
        self.core.render_frame(timestamp)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::ffi::CStr;

    fn manual_track() -> (Renderer, Track) {
        let library = Arc::new(Library::new().unwrap());
        let mut renderer = Renderer::new(Arc::clone(&library)).unwrap();
        renderer.set_storage_size(640, 360);
        renderer.set_frame_size(640, 360);
        renderer
            .set_fonts(None, None, DefaultFontProvider::AutoDetect, None)
            .unwrap();
        let mut track = Track::new(library).unwrap();
        track.set_play_resolution(640, 360).unwrap();
        (renderer, track)
    }

    fn event_text(track: &Track, id: EventId) -> String {
        unsafe {
            let event = &*(*track.as_ptr()).events.add(id.0 as usize);
            CStr::from_ptr(event.Text).to_string_lossy().into_owned()
        }
    }

    #[test]
    fn version_is_available() {
        assert!(version() > 0);
    }

    #[test]
    fn append_after_render_does_not_rebuild() {
        let (mut renderer, mut track) = manual_track();
        let style = track.default_style_id().unwrap();
        track.create_event("A", style).unwrap().set_timing(0, 1_000);
        renderer.render_frame(&mut track, 100).unwrap();
        let ptr = track.as_ptr();

        track
            .create_event("B", style)
            .unwrap()
            .set_timing(1_000, 1_000);
        assert_eq!(track.as_ptr(), ptr);
        assert_eq!(track.event_count(), 2);

        track.create_style("Added", "Arial").unwrap();
        assert_eq!(track.as_ptr(), ptr);
    }

    #[test]
    fn changing_rendered_event_and_style_rebuilds_and_preserves_data() {
        let (mut renderer, mut track) = manual_track();
        let style = track.default_style_id().unwrap();
        let first = track
            .create_event("before", style)
            .unwrap()
            .set_timing(0, 1_000)
            .id();
        let second = track
            .create_event("keep", style)
            .unwrap()
            .set_timing(1_000, 1_000)
            .id();
        renderer.render_frame(&mut track, 100).unwrap();

        let original = track.as_ptr();
        track.event_mut(first).unwrap().set_text("after").unwrap();
        assert_ne!(track.as_ptr(), original);
        assert_eq!(event_text(&track, first), "after");
        assert_eq!(event_text(&track, second), "keep");

        renderer.render_frame(&mut track, 100).unwrap();
        let after_event_rebuild = track.as_ptr();
        track.style_mut(style).unwrap().set_font_size(42.0);
        assert_ne!(track.as_ptr(), after_event_rebuild);
        assert_eq!(track.event_count(), 2);
    }

    #[test]
    fn changing_track_fields_and_features_after_render_rebuilds() {
        let (mut renderer, mut track) = manual_track();
        let style = track.default_style_id().unwrap();
        track.create_event("A", style).unwrap().set_timing(0, 1_000);
        renderer.render_frame(&mut track, 100).unwrap();

        let rendered_ptr = track.as_ptr();
        track.set_play_resolution(1280, 720).unwrap();
        assert_ne!(track.as_ptr(), rendered_ptr);
        assert_eq!(
            unsafe { ((*track.as_ptr()).PlayResX, (*track.as_ptr()).PlayResY) },
            (1280, 720)
        );

        renderer.render_frame(&mut track, 100).unwrap();
        let rerendered_ptr = track.as_ptr();
        track
            .set_feature(TrackFeature::WholeTextLayout, true)
            .unwrap();
        assert_ne!(track.as_ptr(), rerendered_ptr);
        assert_eq!(track.event_count(), 1);
    }

    #[test]
    fn removes_last_in_place_and_middle_transactionally() {
        let (_renderer, mut track) = manual_track();
        let style = track.default_style_id().unwrap();
        let a = track.create_event("A", style).unwrap().id();
        let b = track.create_event("B", style).unwrap().id();
        let c = track.create_event("C", style).unwrap().id();

        let ptr = track.as_ptr();
        track.remove_event(c).unwrap();
        assert_eq!(track.as_ptr(), ptr);
        assert_eq!(track.event_count(), 2);

        track.remove_event(a).unwrap();
        assert_ne!(track.as_ptr(), ptr);
        assert_eq!(track.event_count(), 1);
        assert_eq!(event_text(&track, EventId(0)), "B");
        assert!(matches!(
            track.event_mut(b),
            Err(LibassError::InvalidId { .. })
        ));
    }

    #[test]
    fn removing_a_rendered_tail_rebuilds_but_new_tail_is_removed_in_place() {
        let (mut renderer, mut track) = manual_track();
        let style = track.default_style_id().unwrap();
        let rendered_tail = track.create_event("rendered", style).unwrap().id();
        renderer.render_frame(&mut track, 100).unwrap();

        let rendered_ptr = track.as_ptr();
        track.remove_event(rendered_tail).unwrap();
        assert_ne!(track.as_ptr(), rendered_ptr);
        assert_eq!(track.event_count(), 0);

        renderer.render_frame(&mut track, 100).unwrap();
        let after_empty_render = track.as_ptr();
        let appended_tail = track.create_event("new", style).unwrap().id();
        track.remove_event(appended_tail).unwrap();
        assert_eq!(track.as_ptr(), after_empty_render);
        assert_eq!(track.event_count(), 0);
    }

    const SCRIPT: &[u8] = br#"[Script Info]
ScriptType: v4.00+
PlayResX: 640
PlayResY: 360

[V4+ Styles]
Format: Name, Fontname, Fontsize, PrimaryColour, SecondaryColour, OutlineColour, BackColour, Bold, Italic, Underline, StrikeOut, ScaleX, ScaleY, Spacing, Angle, BorderStyle, Outline, Shadow, Alignment, MarginL, MarginR, MarginV, Encoding
Style: Default,Arial,24,&H00FFFFFF,&H000000FF,&H00000000,&H80000000,0,0,0,0,100,100,0,0,1,2,1,2,10,10,10,1

[Events]
Format: Layer, Start, End, Style, Name, MarginL, MarginR, MarginV, Effect, Text
Dialogue: 0,0:00:00.00,0:00:02.00,Default,,0,0,0,,Hello"#;

    #[test]
    fn script_flush_survives_feature_rebuild() {
        let library = Arc::new(Library::new().unwrap());
        let mut renderer = Renderer::new(Arc::clone(&library)).unwrap();
        renderer.set_frame_size(640, 360);
        renderer.set_storage_size(640, 360);
        let mut track = Track::from_memory(library, SCRIPT, None, false).unwrap();
        assert_eq!(track.event_count(), 1);
        renderer.render_frame(&mut track, 100).unwrap();
        track.flush_events();
        track
            .set_feature(TrackFeature::WholeTextLayout, true)
            .unwrap();
        assert_eq!(track.event_count(), 0);
    }

    const SECOND_SCRIPT: &[u8] = br#"[Script Info]
ScriptType: v4.00+
PlayResX: 640
PlayResY: 360

[V4+ Styles]
Format: Name, Fontname, Fontsize, PrimaryColour, SecondaryColour, OutlineColour, BackColour, Bold, Italic, Underline, StrikeOut, ScaleX, ScaleY, Spacing, Angle, BorderStyle, Outline, Shadow, Alignment, MarginL, MarginR, MarginV, Encoding
Style: Default,Arial,24,&H00FFFFFF,&H000000FF,&H00000000,&H80000000,0,0,0,0,100,100,0,0,1,2,1,2,10,10,10,1

[Events]
Format: Layer, Start, End, Style, Name, MarginL, MarginR, MarginV, Effect, Text
Dialogue: 0,0:00:03.00,0:00:05.00,Default,,0,0,0,,World"#;

    /// 换脚本只换轨道：`Library`/`Renderer` 连同字体提供器与字形缓存必须原样留着，
    /// 否则每次改字幕都要重做一次字体匹配、并把所有字形重新光栅化。
    ///
    /// 两份脚本的时刻刻意不重叠（`SCRIPT` 是 0..2s，`SECOND_SCRIPT` 是 3..5s），
    /// 于是「新时刻出图、旧时刻不再出图」只有真的换掉了轨道才成立。
    #[test]
    fn load_script_replaces_the_track_but_keeps_the_renderer() {
        let mut script = AssScriptRenderer::from_memory(640, 360, SCRIPT, None, false).unwrap();
        assert_eq!(script.event_count(), 1);
        assert!(!script.render_frame(100).unwrap().images().is_empty());
        assert!(script.render_frame(3_100).unwrap().images().is_empty());
        let library_ptr = script.core.library.as_ptr();
        let track_ptr = script.core.track.as_ptr();

        script.load_script(SECOND_SCRIPT, None).unwrap();

        assert_eq!(
            script.core.library.as_ptr(),
            library_ptr,
            "换脚本把 Library 换掉了：字体提供器与已加载字体一起丢失"
        );
        assert_ne!(
            script.core.track.as_ptr(),
            track_ptr,
            "换脚本没有重新解析出新的 Track"
        );
        assert_eq!(script.event_count(), 1);
        assert!(
            !script.render_frame(3_100).unwrap().images().is_empty(),
            "换完脚本后新时刻仍然空，说明渲染器读的还是旧轨道"
        );
        assert!(
            script.render_frame(100).unwrap().images().is_empty(),
            "换完脚本后旧时刻还在出图，说明渲染器读的是旧轨道"
        );
    }

    const CODEC_PRIVATE: &[u8] = br#"[Script Info]
ScriptType: v4.00+
PlayResX: 640
PlayResY: 360

[V4+ Styles]
Format: Name, Fontname, Fontsize, PrimaryColour, SecondaryColour, OutlineColour, BackColour, Bold, Italic, Underline, StrikeOut, ScaleX, ScaleY, Spacing, Angle, BorderStyle, Outline, Shadow, Alignment, MarginL, MarginR, MarginV, Encoding
Style: Default,Arial,24,&H00FFFFFF,&H000000FF,&H00000000,&H80000000,0,0,0,0,100,100,0,0,1,2,1,2,10,10,10,1

[Events]
Format: Layer, Start, End, Style, Name, MarginL, MarginR, MarginV, Effect, Text"#;

    #[test]
    fn matroska_flush_and_readorder_survive_feature_rebuild() {
        let library = Arc::new(Library::new().unwrap());
        let mut renderer = Renderer::new(Arc::clone(&library)).unwrap();
        renderer.set_frame_size(640, 360);
        renderer.set_storage_size(640, 360);
        let mut track = Track::init_matroska(library, CODEC_PRIVATE, false).unwrap();
        track.set_check_readorder(true).unwrap();
        track
            .process_chunk(b"0,0,Default,,0,0,0,,old", 0, 1_000)
            .unwrap();
        assert_eq!(track.event_count(), 1);
        renderer.render_frame(&mut track, 100).unwrap();

        track.flush_events();
        track
            .process_chunk(b"0,0,Default,,0,0,0,,new", 0, 1_000)
            .unwrap();
        assert_eq!(track.event_count(), 1);
        track
            .set_feature(TrackFeature::WholeTextLayout, true)
            .unwrap();

        assert_eq!(track.event_count(), 1);
        assert_eq!(event_text(&track, EventId(0)), "new");
    }

    #[test]
    fn styles_and_events_iterate_with_map_and_filter() {
        let mut renderer = SubtitleRenderer::new(640, 360).unwrap();
        let title = renderer
            .create_style("Title", "Arial")
            .unwrap()
            .set_font_size(36.0)
            .id();
        let body = renderer
            .create_style("Body", "Arial")
            .unwrap()
            .set_font_size(18.0)
            .id();
        renderer
            .add_event_with_style("hello", 0, 1_000, title)
            .unwrap();
        renderer
            .add_event_with_style("world", 1_000, 1_000, body)
            .unwrap();

        let mut names: Vec<String> = renderer.styles().map(|(_, s)| s.name).collect();
        names.sort();
        assert_eq!(
            names,
            vec![
                "Body".to_string(),
                "Default".to_string(),
                "Title".to_string()
            ]
        );
        assert_eq!(renderer.style(title).unwrap().font_size, 36.0);

        let texts: Vec<String> = renderer
            .events()
            .filter(|(_, e)| e.start_ms >= 1_000)
            .filter_map(|(_, e)| e.text)
            .collect();
        assert_eq!(texts, vec!["world".to_string()]);

        let first = renderer.event(EventId(0)).unwrap();
        assert_eq!(first.text.as_deref(), Some("hello"));
        assert_eq!(first.style, title);

        assert!(renderer.style(StyleId(99)).is_err());
        assert!(renderer.event(EventId(99)).is_err());
    }

    #[test]
    fn owned_frame_survives_next_render() {
        let (mut renderer, mut track) = manual_track();
        let style = track.default_style_id().unwrap();
        track
            .create_event("owned", style)
            .unwrap()
            .set_timing(0, 1_000);
        let first = renderer.render_frame(&mut track, 100).unwrap();
        let saved: Vec<Vec<u8>> = first
            .images()
            .iter()
            .map(|image| image.bitmap().to_vec())
            .collect();
        renderer.render_frame(&mut track, 200).unwrap();
        let still_owned: Vec<Vec<u8>> = first
            .images()
            .iter()
            .map(|image| image.bitmap().to_vec())
            .collect();
        assert_eq!(saved, still_owned);
    }
}
