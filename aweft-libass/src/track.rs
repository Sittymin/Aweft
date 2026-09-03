use crate::color::AssColorRgba;
use crate::error::{LibassError, Result};
use crate::ffi;
use crate::library::Library;
use crate::renderer::AssStyle;
use crate::types::{
    Alignment, BorderStyle, Effect, Justify, LibassAlignment, TrackFeature, TrackType,
};
use libc::{c_char, c_int, c_void, size_t};
use std::ffi::{CStr, CString};
use std::ptr;
use std::sync::Arc;

#[derive(Debug, Clone)]
enum TrackSource {
    Manual,
    Script {
        data: Vec<u8>,
        codepage: Option<String>,
        events_flushed: bool,
    },
    Matroska {
        codec_private: Vec<u8>,
        operations: Vec<StreamOperation>,
    },
}

#[derive(Debug, Clone)]
struct StreamChunk {
    data: Vec<u8>,
    timecode_ms: i64,
    duration_ms: i64,
}

#[derive(Debug, Clone)]
enum StreamOperation {
    Chunk(StreamChunk),
    SetCheckReadorder(bool),
    PruneEvents(i64),
}

#[derive(Debug, Clone, Copy)]
struct FeatureSetting {
    feature: TrackFeature,
    enabled: bool,
}

#[derive(Clone)]
struct StyleSnapshot {
    name: Option<Vec<u8>>,
    font_name: Option<Vec<u8>>,
    font_size: f64,
    primary_colour: u32,
    secondary_colour: u32,
    outline_colour: u32,
    back_colour: u32,
    bold: i32,
    italic: i32,
    underline: i32,
    strike_out: i32,
    scale_x: f64,
    scale_y: f64,
    spacing: f64,
    angle: f64,
    border_style: i32,
    outline: f64,
    shadow: f64,
    alignment: i32,
    margin_l: i32,
    margin_r: i32,
    margin_v: i32,
    encoding: i32,
    treat_fontname_as_pattern: i32,
    blur: f64,
    justify: i32,
}

#[derive(Clone)]
struct EventSnapshot {
    start: i64,
    duration: i64,
    read_order: i32,
    layer: i32,
    style: i32,
    name: Option<Vec<u8>>,
    margin_l: i32,
    margin_r: i32,
    margin_v: i32,
    effect: Option<Vec<u8>>,
    text: Option<Vec<u8>>,
}

struct TrackSnapshot {
    styles: Vec<StyleSnapshot>,
    events: Vec<EventSnapshot>,
    style_format: Option<Vec<u8>>,
    event_format: Option<Vec<u8>>,
    track_type: u32,
    play_res_x: i32,
    play_res_y: i32,
    timer: f64,
    wrap_style: i32,
    scaled_border_and_shadow: i32,
    kerning: i32,
    language: Option<Vec<u8>>,
    ycbcr_matrix: u32,
    default_style: i32,
    name: Option<Vec<u8>>,
    layout_res_x: i32,
    layout_res_y: i32,
}
// ============================================================================
// Track 封装
// ============================================================================

pub(crate) struct Track {
    ptr: *mut ffi::ASS_Track,
    library: Arc<Library>,
    rendered: bool,
    /// Number of styles that existed at the most recent render. Styles
    /// appended after that render have IDs at or above this boundary and may
    /// be initialized in place without rebuilding the track.
    rendered_style_count: i32,
    /// Same boundary for events. Appending events remains a direct libass
    /// operation; only mutating an event below this boundary requires rebuild.
    rendered_event_count: i32,
    /// Explicit feature operations in call order. Order matters because
    /// `IncompatibleExtensions` is a meta-feature which changes several bits.
    feature_settings: Vec<FeatureSetting>,
    check_readorder: Option<bool>,
    prune_delay: Option<i64>,
    /// Whether embedded fonts in `[Fonts]` should be extracted via `ass_add_font`.
    /// Only meaningful for Script/Matroska sources.
    extract_fonts: bool,
    source: TrackSource,
}

impl Track {
    fn copy_c_string(ptr: *const c_char) -> Option<Vec<u8>> {
        if ptr.is_null() {
            None
        } else {
            Some(unsafe { CStr::from_ptr(ptr) }.to_bytes_with_nul().to_vec())
        }
    }

    unsafe fn replace_c_string(slot: *mut *mut c_char, value: Option<&[u8]>) -> Result<()> {
        if unsafe { !(*slot).is_null() } {
            unsafe { ffi::ass_free(*slot as *mut c_void) };
        }
        let Some(bytes) = value else {
            unsafe { *slot = ptr::null_mut() };
            return Ok(());
        };
        let allocation = unsafe { ffi::ass_malloc(bytes.len() as size_t) };
        if allocation.is_null() {
            unsafe { *slot = ptr::null_mut() };
            return Err(LibassError::MemoryAllocation {
                context: "重建字幕轨道字符串时分配内存失败".to_string(),
            });
        }
        unsafe {
            ptr::copy_nonoverlapping(bytes.as_ptr(), allocation.cast::<u8>(), bytes.len());
            *slot = allocation.cast::<c_char>();
        }
        Ok(())
    }

    fn snapshot(&self) -> TrackSnapshot {
        unsafe {
            let raw = &*self.ptr;
            let styles = (0..raw.n_styles)
                .map(|index| {
                    // 为指针移动位置
                    let style = &*raw.styles.add(index as usize);
                    StyleSnapshot {
                        name: Self::copy_c_string(style.Name),
                        font_name: Self::copy_c_string(style.FontName),
                        font_size: style.FontSize,
                        primary_colour: style.PrimaryColour,
                        secondary_colour: style.SecondaryColour,
                        outline_colour: style.OutlineColour,
                        back_colour: style.BackColour,
                        bold: style.Bold,
                        italic: style.Italic,
                        underline: style.Underline,
                        strike_out: style.StrikeOut,
                        scale_x: style.ScaleX,
                        scale_y: style.ScaleY,
                        spacing: style.Spacing,
                        angle: style.Angle,
                        border_style: style.BorderStyle,
                        outline: style.Outline,
                        shadow: style.Shadow,
                        alignment: style.Alignment,
                        margin_l: style.MarginL,
                        margin_r: style.MarginR,
                        margin_v: style.MarginV,
                        encoding: style.Encoding,
                        treat_fontname_as_pattern: style.treat_fontname_as_pattern,
                        blur: style.Blur,
                        justify: style.Justify,
                    }
                })
                .collect();
            let events = (0..raw.n_events)
                .map(|index| {
                    let event = &*raw.events.add(index as usize);
                    EventSnapshot {
                        start: event.Start,
                        duration: event.Duration,
                        read_order: event.ReadOrder,
                        layer: event.Layer,
                        style: event.Style,
                        name: Self::copy_c_string(event.Name),
                        margin_l: event.MarginL,
                        margin_r: event.MarginR,
                        margin_v: event.MarginV,
                        effect: Self::copy_c_string(event.Effect),
                        text: Self::copy_c_string(event.Text),
                    }
                })
                .collect();
            TrackSnapshot {
                styles,
                events,
                style_format: Self::copy_c_string(raw.style_format),
                event_format: Self::copy_c_string(raw.event_format),
                track_type: raw.track_type,
                play_res_x: raw.PlayResX,
                play_res_y: raw.PlayResY,
                timer: raw.Timer,
                wrap_style: raw.WrapStyle,
                scaled_border_and_shadow: raw.ScaledBorderAndShadow,
                kerning: raw.Kerning,
                language: Self::copy_c_string(raw.Language),
                ycbcr_matrix: raw.YCbCrMatrix,
                default_style: raw.default_style,
                name: Self::copy_c_string(raw.name),
                layout_res_x: raw.LayoutResX,
                layout_res_y: raw.LayoutResY,
            }
        }
    }

    fn restore_snapshot(library: Arc<Library>, snapshot: &TrackSnapshot) -> Result<Self> {
        let track = Self::new(library)?;
        unsafe {
            while (*track.ptr).n_styles < snapshot.styles.len() as i32 {
                // 会让 n_styles 增长
                if ffi::ass_alloc_style(track.ptr) < 0 {
                    return Err(LibassError::MemoryAllocation {
                        context: "重建字幕样式数组失败".to_string(),
                    });
                }
            }
            for (index, saved) in snapshot.styles.iter().enumerate() {
                let style = &mut *(*track.ptr).styles.add(index);
                Self::replace_c_string(&mut style.Name, saved.name.as_deref())?;
                Self::replace_c_string(&mut style.FontName, saved.font_name.as_deref())?;
                style.FontSize = saved.font_size;
                style.PrimaryColour = saved.primary_colour;
                style.SecondaryColour = saved.secondary_colour;
                style.OutlineColour = saved.outline_colour;
                style.BackColour = saved.back_colour;
                style.Bold = saved.bold;
                style.Italic = saved.italic;
                style.Underline = saved.underline;
                style.StrikeOut = saved.strike_out;
                style.ScaleX = saved.scale_x;
                style.ScaleY = saved.scale_y;
                style.Spacing = saved.spacing;
                style.Angle = saved.angle;
                style.BorderStyle = saved.border_style;
                style.Outline = saved.outline;
                style.Shadow = saved.shadow;
                style.Alignment = saved.alignment;
                style.MarginL = saved.margin_l;
                style.MarginR = saved.margin_r;
                style.MarginV = saved.margin_v;
                style.Encoding = saved.encoding;
                style.treat_fontname_as_pattern = saved.treat_fontname_as_pattern;
                style.Blur = saved.blur;
                style.Justify = saved.justify;
            }
            for saved in &snapshot.events {
                let event_id = ffi::ass_alloc_event(track.ptr);
                if event_id < 0 {
                    return Err(LibassError::MemoryAllocation {
                        context: "重建字幕事件数组失败".to_string(),
                    });
                }
                let event = &mut *(*track.ptr).events.add(event_id as usize);
                event.Start = saved.start;
                event.Duration = saved.duration;
                event.ReadOrder = saved.read_order;
                event.Layer = saved.layer;
                event.Style = saved.style;
                Self::replace_c_string(&mut event.Name, saved.name.as_deref())?;
                event.MarginL = saved.margin_l;
                event.MarginR = saved.margin_r;
                event.MarginV = saved.margin_v;
                Self::replace_c_string(&mut event.Effect, saved.effect.as_deref())?;
                Self::replace_c_string(&mut event.Text, saved.text.as_deref())?;
                event.render_priv = ptr::null_mut();
            }
            let raw = &mut *track.ptr;
            Self::replace_c_string(&mut raw.style_format, snapshot.style_format.as_deref())?;
            Self::replace_c_string(&mut raw.event_format, snapshot.event_format.as_deref())?;
            raw.track_type = snapshot.track_type;
            raw.PlayResX = snapshot.play_res_x;
            raw.PlayResY = snapshot.play_res_y;
            raw.Timer = snapshot.timer;
            raw.WrapStyle = snapshot.wrap_style;
            raw.ScaledBorderAndShadow = snapshot.scaled_border_and_shadow;
            raw.Kerning = snapshot.kerning;
            Self::replace_c_string(&mut raw.Language, snapshot.language.as_deref())?;
            raw.YCbCrMatrix = snapshot.ycbcr_matrix;
            raw.default_style = snapshot.default_style;
            Self::replace_c_string(&mut raw.name, snapshot.name.as_deref())?;
            raw.LayoutResX = snapshot.layout_res_x;
            raw.LayoutResY = snapshot.layout_res_y;
        }
        Ok(track)
    }

    fn rebuild(&mut self) -> Result<()> {
        let source = self.source.clone();
        let feature_settings = self.feature_settings.clone();
        let check_readorder = self.check_readorder;
        let prune_delay = self.prune_delay;
        let extract_fonts = self.extract_fonts;

        // Ensure the library flag matches the track's desired state before re-parsing.
        self.library.set_extract_fonts(extract_fonts);

        let mut rebuilt = match &source {
            TrackSource::Manual => {
                let snapshot = self.snapshot();
                Self::restore_snapshot(Arc::clone(&self.library), &snapshot)?
            }
            TrackSource::Script {
                data,
                codepage,
                events_flushed,
            } => {
                let mut track = Self::from_memory(
                    Arc::clone(&self.library),
                    data,
                    codepage.as_deref(),
                    extract_fonts,
                )?;
                if *events_flushed {
                    track.flush_events_raw();
                }
                track
            }
            TrackSource::Matroska {
                codec_private,
                operations,
            } => {
                let mut track =
                    Self::init_matroska(Arc::clone(&self.library), codec_private, extract_fonts)?;
                for operation in operations {
                    match operation {
                        StreamOperation::Chunk(chunk) => track.process_chunk_raw(
                            &chunk.data,
                            chunk.timecode_ms,
                            chunk.duration_ms,
                        )?,
                        StreamOperation::SetCheckReadorder(enabled) => unsafe {
                            ffi::ass_set_check_readorder(track.ptr, *enabled as c_int);
                        },
                        StreamOperation::PruneEvents(deadline) => unsafe {
                            ffi::ass_prune_events(track.ptr, *deadline);
                        },
                    }
                }
                track
            }
        };

        rebuilt.source = source;
        rebuilt.extract_fonts = extract_fonts;
        for setting in &feature_settings {
            rebuilt.set_feature_raw(setting.feature, setting.enabled)?;
        }
        rebuilt.feature_settings = feature_settings;
        rebuilt.check_readorder = check_readorder;
        if let Some(enabled) = check_readorder {
            unsafe { ffi::ass_set_check_readorder(rebuilt.ptr, enabled as c_int) };
        }
        rebuilt.prune_delay = prune_delay;
        if let Some(delay) = prune_delay {
            unsafe { ffi::ass_configure_prune(rebuilt.ptr, delay) };
        }

        *self = rebuilt;
        Ok(())
    }

    fn prepare_track_field_mutation(&mut self) -> Result<()> {
        if self.rendered {
            self.rebuild()?;
        }
        Ok(())
    }

    fn prepare_style_mutation(&mut self, id: StyleId) -> Result<()> {
        if self.rendered && id.0 < self.rendered_style_count {
            self.rebuild()?;
        }
        Ok(())
    }

    fn prepare_event_mutation(&mut self, id: EventId) -> Result<()> {
        if self.rendered && id.0 < self.rendered_event_count {
            self.rebuild()?;
        }
        Ok(())
    }

    pub(crate) fn mark_rendered(&mut self) {
        self.rendered = true;
        self.rendered_style_count = self.style_count();
        self.rendered_event_count = self.event_count();
    }

    /// 创建用于手动构造字幕的轨道。
    pub fn new(library: Arc<Library>) -> Result<Self> {
        unsafe {
            // 这里已经创建了一个默认样式
            let ptr = ffi::ass_new_track(library.as_ptr());
            if ptr.is_null() {
                return Err(LibassError::TrackInit {
                    context: "ass_new_track 返回空指针".to_string(),
                });
            }
            Ok(Self::from_raw(ptr, library))
        }
    }

    /// 从完整 ASS/SSA 文本创建轨道。文本由 libass 解析，适合加载已有脚本。
    pub fn from_memory(
        library: Arc<Library>,
        data: &[u8],
        codepage: Option<&str>,
        extract_fonts: bool,
    ) -> Result<Self> {
        library.set_extract_fonts(extract_fonts);
        let mut owned = data.to_vec();
        let saved_codepage = codepage.map(str::to_owned);
        let codepage = codepage.map(CString::new).transpose()?;
        let ptr = unsafe {
            ffi::ass_read_memory(
                library.as_ptr(),
                owned.as_mut_ptr() as *mut c_char,
                owned.len(),
                codepage.as_ref().map_or(ptr::null(), |s| s.as_ptr()),
            )
        };
        if ptr.is_null() {
            return Err(LibassError::TrackInit {
                context: "ass_read_memory 返回空指针".to_string(),
            });
        }
        let mut track = Self::from_raw(ptr, library);
        track.extract_fonts = extract_fonts;
        track.source = TrackSource::Script {
            data: data.to_vec(),
            codepage: saved_codepage,
            events_flushed: false,
        };
        Ok(track)
    }

    fn from_raw(ptr: *mut ffi::ASS_Track, library: Arc<Library>) -> Self {
        Self {
            ptr,
            library,
            rendered: false,
            rendered_style_count: 0,
            rendered_event_count: 0,
            feature_settings: Vec::new(),
            check_readorder: None,
            prune_delay: None,
            extract_fonts: false,
            source: TrackSource::Manual,
        }
    }

    pub fn set_play_resolution(&mut self, width: i32, height: i32) -> Result<()> {
        self.prepare_track_field_mutation()?;
        unsafe {
            (*self.ptr).PlayResX = width;
            (*self.ptr).PlayResY = height;
        }
        Ok(())
    }

    pub fn set_track_type(&mut self, track_type: TrackType) -> Result<()> {
        self.prepare_track_field_mutation()?;
        unsafe {
            (*self.ptr).track_type = track_type as u32;
        }
        Ok(())
    }

    // 创建新样式
    pub fn create_style(&mut self, name: &str, font_name: &str) -> Result<StyleHandle<'_>> {
        unsafe {
            let style_id = ffi::ass_alloc_style(self.ptr);
            if style_id < 0 {
                return Err(LibassError::MemoryAllocation {
                    context: "ass_alloc_style 分配样式失败".to_string(),
                });
            }

            Ok(StyleHandle {
                track: self,
                id: StyleId(style_id),
            }
            .set_name(name)?
            .set_font_name(font_name)?
            .set_font_size(18.0)
            .set_scale_x(1.0)
            .set_scale_y(1.0)
            .set_border_style(BorderStyle::Outline)
            .set_alignment(Alignment::BottomCenter))
        }
    }

    /// 获取样式数量
    pub fn style_count(&self) -> i32 {
        unsafe { (*self.ptr).n_styles }
    }

    /// 迭代所有样式ID
    pub fn style_ids(&self) -> impl Iterator<Item = StyleId> {
        let count = self.style_count();
        (0..count).map(StyleId)
    }

    /// 获取默认样式的可变引用（Rust风格的安全API）
    pub fn default_style_mut(&mut self) -> Result<StyleHandle<'_>> {
        unsafe {
            if (*self.ptr).styles.is_null() || (*self.ptr).n_styles == 0 {
                return Err(LibassError::ResourceUnavailable {
                    context: "没有可用的样式".to_string(),
                });
            }
            // 使用 libass 自动创建的默认样式（default_style 字段）
            let default_style_id = (*self.ptr).default_style;
            if default_style_id < 0 || default_style_id >= (*self.ptr).n_styles {
                return Err(LibassError::InvalidId {
                    resource_type: "样式".to_string(),
                    id: default_style_id,
                });
            }
            let id = StyleId(default_style_id);
            self.prepare_style_mutation(id)?;
            Ok(StyleHandle { track: self, id })
        }
    }

    pub fn default_style_id(&self) -> Result<StyleId> {
        unsafe {
            let id = (*self.ptr).default_style;
            if (*self.ptr).styles.is_null() || id < 0 || id >= (*self.ptr).n_styles {
                return Err(LibassError::ResourceUnavailable {
                    context: "没有可用的默认样式".to_string(),
                });
            }
            Ok(StyleId(id))
        }
    }

    pub fn style_mut(&mut self, id: StyleId) -> Result<StyleHandle<'_>> {
        if id.0 < 0 || id.0 >= self.style_count() {
            return Err(LibassError::InvalidId {
                resource_type: "样式".to_string(),
                id: id.0,
            });
        }
        self.prepare_style_mutation(id)?;
        Ok(StyleHandle { track: self, id })
    }

    fn convert_style(&self, index: i32) -> Result<AssStyle> {
        unsafe {
            let style_ptr = (*self.ptr).styles.add(index as usize);
            if (*style_ptr).Name.is_null() || (*style_ptr).FontName.is_null() {
                return Err(LibassError::ResourceUnavailable {
                    context: "样式字符串尚未初始化".to_string(),
                });
            }
            AssStyle::try_from(style_ptr as *const ffi::ass_style)
        }
    }

    /// 只读获取样式数据的 owned 拷贝，可与 `style_mut` 配对使用。
    pub fn style(&self, id: StyleId) -> Result<AssStyle> {
        if id.0 < 0 || id.0 >= self.style_count() {
            return Err(LibassError::InvalidId {
                resource_type: "样式".to_string(),
                id: id.0,
            });
        }
        self.convert_style(id.0)
    }

    /// 遍历全部样式，返回 `(StyleId, AssStyle)`，可直接 `map`/`filter`/`collect`。
    /// 尚未完全初始化的条目（字符串指针为空）会被跳过。
    pub fn styles(&self) -> impl Iterator<Item = (StyleId, AssStyle)> + '_ {
        (0..self.style_count()).filter_map(|index| {
            self.convert_style(index)
                .ok()
                .map(|style| (StyleId(index), style))
        })
    }

    /// 获取事件数量  
    pub fn event_count(&self) -> i32 {
        unsafe { (*self.ptr).n_events }
    }

    /// 迭代所有事件ID
    pub fn event_ids(&self) -> impl Iterator<Item = EventId> {
        let count = self.event_count();
        (0..count).map(EventId)
    }

    /// 处理 Matroska CodecPrivate 中的 ASS 头信息。
    fn process_codec_private_raw(&mut self, data: &[u8]) -> Result<()> {
        let size = i32::try_from(data.len()).map_err(|_| LibassError::InvalidParameter {
            context: "codec private 数据过大".to_string(),
        })?;
        unsafe { ffi::ass_process_codec_private(self.ptr, data.as_ptr() as *const c_char, size) };
        Ok(())
    }

    /// 增量处理一个 Matroska 字幕事件。时间单位为毫秒。
    fn process_chunk_raw(&mut self, data: &[u8], timecode_ms: i64, duration_ms: i64) -> Result<()> {
        let size = i32::try_from(data.len()).map_err(|_| LibassError::InvalidParameter {
            context: "字幕 chunk 数据过大".to_string(),
        })?;
        if timecode_ms < 0 || duration_ms < 0 {
            return Err(LibassError::InvalidParameter {
                context: "chunk 时间和持续时间不能为负".to_string(),
            });
        }
        unsafe {
            ffi::ass_process_chunk(
                self.ptr,
                data.as_ptr() as *const c_char,
                size,
                timecode_ms,
                duration_ms,
            );
        }
        Ok(())
    }

    /// 从 Matroska codec_private 初始化轨道，仅解析头信息（样式/字体/Format 行），
    /// Dialogue 事件需后续经 [`Self::process_chunk`] 流式喂入。
    pub(crate) fn init_matroska(
        library: Arc<Library>,
        codec_private: &[u8],
        extract_fonts: bool,
    ) -> Result<Self> {
        library.set_extract_fonts(extract_fonts);
        let mut track = Self::new(library)?;
        track.extract_fonts = extract_fonts;
        track.process_codec_private_raw(codec_private)?;
        track.source = TrackSource::Matroska {
            codec_private: codec_private.to_vec(),
            operations: Vec::new(),
        };
        Ok(track)
    }

    pub(crate) fn process_chunk(
        &mut self,
        data: &[u8],
        timecode_ms: i64,
        duration_ms: i64,
    ) -> Result<()> {
        if !matches!(self.source, TrackSource::Matroska { .. }) {
            return Err(LibassError::InvalidParameter {
                context: "只有 Matroska 轨道可以处理字幕 chunk".to_string(),
            });
        }
        self.process_chunk_raw(data, timecode_ms, duration_ms)?;
        let TrackSource::Matroska { operations, .. } = &mut self.source else {
            unreachable!("Matroska source checked above");
        };
        operations.push(StreamOperation::Chunk(StreamChunk {
            data: data.to_vec(),
            timecode_ms,
            duration_ms,
        }));
        Ok(())
    }

    /// 设置 libass Track 特性；必须在第一次渲染前调用。
    fn set_feature_raw(&mut self, feature: TrackFeature, enabled: bool) -> Result<()> {
        let result =
            unsafe { ffi::ass_track_set_feature(self.ptr, feature.to_raw(), enabled as c_int) };
        if result < 0 {
            return Err(LibassError::InvalidParameter {
                context: format!("libass 不支持 Track feature {:?}", feature),
            });
        }
        Ok(())
    }

    pub fn set_feature(&mut self, feature: TrackFeature, enabled: bool) -> Result<()> {
        if self.rendered {
            self.rebuild()?;
        }
        self.set_feature_raw(feature, enabled)?;
        self.feature_settings
            .push(FeatureSetting { feature, enabled });
        Ok(())
    }

    /// 设置 Matroska chunk 的 ReadOrder 去重策略。
    pub fn set_check_readorder(&mut self, enabled: bool) -> Result<()> {
        if self.rendered {
            self.rebuild()?;
        }
        unsafe { ffi::ass_set_check_readorder(self.ptr, enabled as c_int) };
        self.check_readorder = Some(enabled);
        if let TrackSource::Matroska { operations, .. } = &mut self.source {
            operations.push(StreamOperation::SetCheckReadorder(enabled));
        }
        Ok(())
    }

    /// 控制是否提取 `[Fonts]` 内嵌字体；仅对解析轨道有效。
    /// 构造后调用会触发重建以重新解析。
    pub(crate) fn set_extract_fonts(&mut self, extract: bool) -> Result<()> {
        if matches!(self.source, TrackSource::Manual) {
            return Err(LibassError::InvalidParameter {
                context: "手动轨道不支持提取内嵌字体".to_string(),
            });
        }
        if self.extract_fonts == extract {
            return Ok(());
        }
        self.extract_fonts = extract;
        self.library.set_extract_fonts(extract);
        self.rebuild()?;
        Ok(())
    }

    /// 创建新事件
    pub fn create_event(&mut self, text: &str, style_id: StyleId) -> Result<EventHandle<'_>> {
        if style_id.0 < 0 || style_id.0 >= self.style_count() {
            return Err(LibassError::InvalidId {
                resource_type: "样式".to_string(),
                id: style_id.0,
            });
        }
        unsafe {
            let event_id = ffi::ass_alloc_event(self.ptr);
            if event_id < 0 {
                return Err(LibassError::MemoryAllocation {
                    context: "ass_alloc_event 分配事件失败".to_string(),
                });
            }

            let id = EventId(event_id);
            Ok(EventHandle { track: self, id }
                .set_text(text)?
                .set_style(style_id)
                // 由于 aweft 每次仅用 libass 渲染当前 event
                // 并不会将全部 event 传递，所以这里设置必然的时间范围
                .set_timing(0, i64::MAX))
        }
    }

    // 清理 track 当中显示时间已过的 event
    pub(crate) fn prune_events(&mut self, deadline: i64) {
        unsafe { ffi::ass_prune_events(self.ptr, deadline) };
        if let TrackSource::Matroska { operations, .. } = &mut self.source {
            operations.push(StreamOperation::PruneEvents(deadline));
        }
    }

    // 配置每次调用 render_frame 时自动运行 prune_events 删除结束时间 < (当前渲染时间 now - delay) 的 event
    pub(crate) fn configure_prune(&mut self, delay: i64) {
        unsafe { ffi::ass_configure_prune(self.ptr, delay) };
        self.prune_delay = Some(delay);
    }

    fn flush_events_raw(&mut self) {
        unsafe { ffi::ass_flush_events(self.ptr) };
        self.rendered_event_count = 0;
    }

    // 删除 track 所有 event
    pub fn flush_events(&mut self) {
        self.flush_events_raw();
        match &mut self.source {
            TrackSource::Script { events_flushed, .. } => *events_flushed = true,
            TrackSource::Matroska { operations, .. } => {
                // A flush makes all earlier chunks irrelevant. Preserve only
                // the parser option that subsequent chunks observe.
                operations.clear();
                if let Some(enabled) = self.check_readorder {
                    operations.push(StreamOperation::SetCheckReadorder(enabled));
                }
            }
            TrackSource::Manual => {}
        }
    }

    /// Remove one event while preserving the order of remaining events.
    /// Removing an interior event rebuilds the Track, so IDs after the removed
    /// element shift down by one and must be reacquired by the caller.
    pub fn remove_event(&mut self, id: EventId) -> Result<()> {
        let count = self.event_count();
        if id.0 < 0 || id.0 >= count {
            return Err(LibassError::InvalidId {
                resource_type: "事件".to_string(),
                id: id.0,
            });
        }
        if !matches!(self.source, TrackSource::Manual) {
            return Err(LibassError::InvalidParameter {
                context: "解析轨道不支持按 ID 删除事件；请清空事件或重新加载输入".to_string(),
            });
        }

        // libass may still reference events that existed at the most recent
        // render. Only a newly appended, never-rendered tail event is safe to
        // release in place. All other removals use a fresh Track.
        if id.0 == count - 1 && id.0 >= self.rendered_event_count {
            unsafe {
                ffi::ass_free_event(self.ptr, id.0);
                (*self.ptr).n_events -= 1;
            }
            self.rendered_event_count = self.rendered_event_count.min(count - 1);
            return Ok(());
        }

        let mut snapshot = self.snapshot();
        snapshot.events.remove(id.0 as usize);
        let feature_settings = self.feature_settings.clone();
        let check_readorder = self.check_readorder;
        let prune_delay = self.prune_delay;
        let mut rebuilt = Self::restore_snapshot(Arc::clone(&self.library), &snapshot)?;
        for setting in &feature_settings {
            rebuilt.set_feature_raw(setting.feature, setting.enabled)?;
        }
        rebuilt.feature_settings = feature_settings;
        rebuilt.check_readorder = check_readorder;
        if let Some(enabled) = check_readorder {
            unsafe { ffi::ass_set_check_readorder(rebuilt.ptr, enabled as c_int) };
        }
        rebuilt.prune_delay = prune_delay;
        if let Some(delay) = prune_delay {
            unsafe { ffi::ass_configure_prune(rebuilt.ptr, delay) };
        }
        *self = rebuilt;
        Ok(())
    }

    pub fn event_mut(&mut self, id: EventId) -> Result<EventHandle<'_>> {
        if id.0 < 0 || id.0 >= self.event_count() {
            return Err(LibassError::InvalidId {
                resource_type: "事件".to_string(),
                id: id.0,
            });
        }
        self.prepare_event_mutation(id)?;
        Ok(EventHandle { track: self, id })
    }

    fn opt_string(ptr: *const c_char) -> Result<Option<String>> {
        if ptr.is_null() {
            return Ok(None);
        }
        unsafe { Ok(Some(CStr::from_ptr(ptr).to_str()?.to_owned())) }
    }

    fn convert_event(&self, index: i32) -> Result<EventInfo> {
        unsafe {
            let event_ptr = (*self.ptr).events.add(index as usize);
            Ok(EventInfo {
                start_ms: (*event_ptr).Start,
                duration_ms: (*event_ptr).Duration,
                read_order: (*event_ptr).ReadOrder,
                layer: (*event_ptr).Layer,
                style: StyleId((*event_ptr).Style),
                name: Self::opt_string((*event_ptr).Name)?,
                margin_l: (*event_ptr).MarginL,
                margin_r: (*event_ptr).MarginR,
                margin_v: (*event_ptr).MarginV,
                effect: Self::opt_string((*event_ptr).Effect)?,
                text: Self::opt_string((*event_ptr).Text)?,
            })
        }
    }

    /// 只读获取事件数据的 owned 拷贝，可与 `event_mut` 配对使用。
    pub fn event(&self, id: EventId) -> Result<EventInfo> {
        if id.0 < 0 || id.0 >= self.event_count() {
            return Err(LibassError::InvalidId {
                resource_type: "事件".to_string(),
                id: id.0,
            });
        }
        self.convert_event(id.0)
    }

    /// 遍历全部事件，返回 `(EventId, EventInfo)`，可直接 `map`/`filter`/`collect`。
    /// 含非法字符串指针的条目会被跳过。
    pub fn events(&self) -> impl Iterator<Item = (EventId, EventInfo)> + '_ {
        (0..self.event_count()).filter_map(|index| {
            self.convert_event(index)
                .ok()
                .map(|event| (EventId(index), event))
        })
    }

    pub(crate) fn as_ptr(&self) -> *mut ffi::ASS_Track {
        self.ptr
    }
}

impl Drop for Track {
    fn drop(&mut self) {
        unsafe {
            ffi::ass_free_track(self.ptr);
        }
    }
}
// ============================================================================
// Rust风格的Style和Event管理
// ============================================================================

/// 样式ID，用于安全地引用Track中的样式
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct StyleId(pub(crate) i32);

/// 事件ID，用于安全地引用Track中的事件
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct EventId(pub(crate) i32);

/// 事件数据的 owned 只读拷贝，由 [`Track::event`] / [`Track::events`] 返回。
#[derive(Debug, Clone)]
pub struct EventInfo {
    pub start_ms: i64,
    pub duration_ms: i64,
    pub read_order: i32,
    pub layer: i32,
    pub style: StyleId,
    pub name: Option<String>,
    pub margin_l: i32,
    pub margin_r: i32,
    pub margin_v: i32,
    pub effect: Option<String>,
    pub text: Option<String>,
}

/// 样式构建器，使用ID而不是直接指针
pub struct StyleHandle<'a> {
    track: &'a mut Track,
    id: StyleId,
}

/// 事件构建器，使用ID而不是直接指针
pub struct EventHandle<'a> {
    track: &'a mut Track,
    id: EventId,
}

impl StyleId {
    pub fn as_index(self) -> i32 {
        self.0
    }
}

impl EventId {
    pub fn as_index(self) -> i32 {
        self.0
    }
}

impl<'a> StyleHandle<'a> {
    /// 安全地获取样式指针（仅在需要时）
    fn as_mut_ptr(&mut self) -> *mut ffi::ASS_Style {
        unsafe {
            let track_ptr = self.track.as_ptr();
            debug_assert!(
                !(*track_ptr).styles.is_null()
                    && self.id.0 >= 0
                    && self.id.0 < (*track_ptr).n_styles
            );
            (*track_ptr).styles.offset(self.id.0 as isize)
        }
    }

    pub fn set_name(mut self, name: &str) -> Result<Self> {
        // C 侧虽然不会修改，但是它会 free 它，so can't direct use CString
        let c_name = CString::new(name)?;
        let c_name_bytes = c_name.as_bytes_with_nul();

        unsafe {
            let style_ptr = self.as_mut_ptr();
            if !(*style_ptr).Name.is_null() {
                ffi::ass_free((*style_ptr).Name as *mut c_void)
            }
            let p = ffi::ass_malloc(c_name_bytes.len() as size_t);
            if p.is_null() {
                return Err(LibassError::MemoryAllocation {
                    context: "为样式名分配内存失败".to_string(),
                });
            }
            ptr::copy_nonoverlapping(c_name_bytes.as_ptr(), p as *mut u8, c_name_bytes.len());
            (*style_ptr).Name = p as *mut c_char;
        }
        Ok(self)
    }

    pub fn set_font_name(mut self, font_name: &str) -> Result<Self> {
        let c_font_name = CString::new(font_name)?;
        let c_font_name_bytes = c_font_name.as_bytes_with_nul();
        unsafe {
            let style_ptr = self.as_mut_ptr();
            if !(*style_ptr).FontName.is_null() {
                ffi::ass_free((*style_ptr).FontName as *mut c_void);
            }
            let p = ffi::ass_malloc(c_font_name_bytes.len() as size_t);
            if p.is_null() {
                return Err(LibassError::MemoryAllocation {
                    context: "为字体名分配内存失败".to_string(),
                });
            }
            ptr::copy_nonoverlapping(
                c_font_name_bytes.as_ptr(),
                p as *mut u8,
                c_font_name_bytes.len(),
            );
            (*style_ptr).FontName = p as *mut c_char;
        }
        Ok(self)
    }

    pub fn set_font_size(mut self, size: f64) -> Self {
        unsafe {
            let style_ptr = self.as_mut_ptr();
            (*style_ptr).FontSize = size;
        }
        self
    }

    pub fn set_primary_color(mut self, color: AssColorRgba) -> Self {
        unsafe {
            let style_ptr = self.as_mut_ptr();
            (*style_ptr).PrimaryColour = color.to_libass_u32();
        }
        self
    }

    pub fn set_secondary_color(mut self, color: AssColorRgba) -> Self {
        unsafe {
            let style_ptr = self.as_mut_ptr();
            (*style_ptr).SecondaryColour = color.to_libass_u32();
        }
        self
    }

    pub fn set_outline_color(mut self, color: AssColorRgba) -> Self {
        unsafe {
            let style_ptr = self.as_mut_ptr();
            (*style_ptr).OutlineColour = color.to_libass_u32();
        }
        self
    }

    pub fn set_back_color(mut self, color: AssColorRgba) -> Self {
        unsafe {
            let style_ptr = self.as_mut_ptr();
            (*style_ptr).BackColour = color.to_libass_u32();
        }
        self
    }

    pub fn set_bold(mut self, bold: bool) -> Self {
        unsafe {
            let style_ptr = self.as_mut_ptr();
            (*style_ptr).Bold = if bold { 1 } else { 0 };
        }
        self
    }

    pub fn set_italic(mut self, italic: bool) -> Self {
        unsafe {
            let style_ptr = self.as_mut_ptr();
            (*style_ptr).Italic = if italic { 1 } else { 0 };
        }
        self
    }

    pub fn set_underline(mut self, underline: bool) -> Self {
        unsafe {
            let style_ptr = self.as_mut_ptr();
            (*style_ptr).Underline = if underline { 1 } else { 0 };
        }
        self
    }

    pub fn set_strike_out(mut self, strike_out: bool) -> Self {
        unsafe {
            let style_ptr = self.as_mut_ptr();
            (*style_ptr).StrikeOut = if strike_out { 1 } else { 0 };
        }
        self
    }

    /// 缩放以 1.0 为 100%（与脚本文件中写 100 不同，解析器会对后者除以 100）。
    /// 注意：0 会导致字形被跳过渲染。
    pub fn set_scale_x(mut self, scale_x: f64) -> Self {
        unsafe {
            let style_ptr = self.as_mut_ptr();
            (*style_ptr).ScaleX = scale_x;
        }
        self
    }

    /// 缩放以 1.0 为 100%（与脚本文件中写 100 不同，解析器会对后者除以 100）。
    /// 注意：0 会导致字形被跳过渲染。
    pub fn set_scale_y(mut self, scale_y: f64) -> Self {
        unsafe {
            let style_ptr = self.as_mut_ptr();
            (*style_ptr).ScaleY = scale_y;
        }
        self
    }

    pub fn set_spacing(mut self, spacing: f64) -> Self {
        unsafe {
            let style_ptr = self.as_mut_ptr();
            (*style_ptr).Spacing = spacing;
        }
        self
    }

    pub fn set_angle(mut self, angle: f64) -> Self {
        unsafe {
            let style_ptr = self.as_mut_ptr();
            (*style_ptr).Angle = angle;
        }
        self
    }

    pub fn set_border_style(mut self, border_style: BorderStyle) -> Self {
        unsafe {
            let style_ptr = self.as_mut_ptr();
            (*style_ptr).BorderStyle = border_style.to_raw();
        }
        self
    }

    pub fn set_outline(mut self, outline: f64) -> Self {
        unsafe {
            let style_ptr = self.as_mut_ptr();
            (*style_ptr).Outline = outline;
        }
        self
    }

    pub fn set_shadow(mut self, shadow: f64) -> Self {
        unsafe {
            let style_ptr = self.as_mut_ptr();
            (*style_ptr).Shadow = shadow;
        }
        self
    }

    pub fn set_alignment(mut self, alignment: Alignment) -> Self {
        let ffi_alignment: LibassAlignment = alignment.into();
        let ffi_value: i32 = ffi_alignment.bits();
        unsafe {
            let style_ptr = self.as_mut_ptr();
            (*style_ptr).Alignment = ffi_value;
        }
        self
    }

    pub fn set_margins(mut self, left: i32, right: i32, vertical: i32) -> Self {
        unsafe {
            let style_ptr = self.as_mut_ptr();
            (*style_ptr).MarginL = left;
            (*style_ptr).MarginR = right;
            (*style_ptr).MarginV = vertical;
        }
        self
    }

    pub fn set_encoding(mut self, encoding: i32) -> Self {
        unsafe {
            let style_ptr = self.as_mut_ptr();
            (*style_ptr).Encoding = encoding;
        }
        self
    }

    pub fn set_blur(mut self, blur: f64) -> Self {
        unsafe {
            let style_ptr = self.as_mut_ptr();
            (*style_ptr).Blur = blur;
        }
        self
    }

    pub fn set_justify(mut self, justify: Justify) -> Self {
        unsafe {
            let style_ptr = self.as_mut_ptr();
            (*style_ptr).Justify = justify.to_raw();
        }
        self
    }

    /// 获取样式ID
    pub fn id(&self) -> StyleId {
        self.id
    }
}

impl<'a> EventHandle<'a> {
    /// 安全地获取事件指针（仅在需要时）
    fn as_mut_ptr(&mut self) -> *mut ffi::ASS_Event {
        unsafe {
            let track_ptr = self.track.as_ptr();
            debug_assert!(
                !(*track_ptr).events.is_null()
                    && self.id.0 >= 0
                    && self.id.0 < (*track_ptr).n_events
            );
            (*track_ptr).events.offset(self.id.0 as isize)
        }
    }

    pub fn set_text(mut self, text: &str) -> Result<Self> {
        let c_text = CString::new(text)?;
        let c_text_bytes = c_text.as_bytes_with_nul();
        unsafe {
            let event_ptr = self.as_mut_ptr();
            if !(*event_ptr).Text.is_null() {
                ffi::ass_free((*event_ptr).Text as *mut c_void);
            }
            let p = ffi::ass_malloc(c_text_bytes.len() as size_t);
            if p.is_null() {
                return Err(LibassError::MemoryAllocation {
                    context: "为事件文本分配内存失败".to_string(),
                });
            }
            ptr::copy_nonoverlapping(c_text_bytes.as_ptr(), p as *mut u8, c_text_bytes.len());
            (*event_ptr).Text = p as *mut c_char;
        }
        Ok(self)
    }

    pub fn set_timing(mut self, start: i64, duration: i64) -> Self {
        unsafe {
            let event_ptr = self.as_mut_ptr();
            (*event_ptr).Start = start;
            (*event_ptr).Duration = duration;
        }
        self
    }

    pub fn set_style(mut self, style_id: StyleId) -> Self {
        unsafe {
            let event_ptr = self.as_mut_ptr();
            (*event_ptr).Style = style_id.0;
        }
        self
    }

    pub fn set_layer(mut self, layer: i32) -> Self {
        unsafe {
            let event_ptr = self.as_mut_ptr();
            (*event_ptr).Layer = layer;
        }
        self
    }

    pub fn set_margins(mut self, left: i32, right: i32, vertical: i32) -> Self {
        unsafe {
            let event_ptr = self.as_mut_ptr();
            (*event_ptr).MarginL = left;
            (*event_ptr).MarginR = right;
            (*event_ptr).MarginV = vertical;
        }
        self
    }

    pub fn set_effect(mut self, effect: Effect) -> Result<Self> {
        let c_effect = effect.to_cstring()?;
        let c_effect_bytes = c_effect.as_bytes_with_nul();
        unsafe {
            let event_ptr = self.as_mut_ptr();
            if !(*event_ptr).Effect.is_null() {
                ffi::ass_free((*event_ptr).Effect as *mut c_void);
            }
            let p = ffi::ass_malloc(c_effect_bytes.len() as size_t);
            if p.is_null() {
                return Err(LibassError::MemoryAllocation {
                    context: "为效果分配内存失败".to_string(),
                });
            }
            ptr::copy_nonoverlapping(c_effect_bytes.as_ptr(), p as *mut u8, c_effect_bytes.len());
            (*event_ptr).Effect = p as *mut c_char;
        }
        Ok(self)
    }
    pub fn set_name(mut self, name: &str) -> Result<Self> {
        let c_name = CString::new(name)?;
        let c_name_bytes = c_name.as_bytes_with_nul();
        unsafe {
            let event_ptr = self.as_mut_ptr();
            if !(*event_ptr).Name.is_null() {
                ffi::ass_free((*event_ptr).Name as *mut c_void);
            }
            let p = ffi::ass_malloc(c_name_bytes.len() as size_t);
            if p.is_null() {
                return Err(LibassError::MemoryAllocation {
                    context: "为事件人物名分配内存失败".to_string(),
                });
            }
            ptr::copy_nonoverlapping(c_name_bytes.as_ptr(), p as *mut u8, c_name_bytes.len());
            (*event_ptr).Name = p as *mut c_char;
        }
        Ok(self)
    }

    /// 获取事件ID
    pub fn id(&self) -> EventId {
        self.id
    }
}
