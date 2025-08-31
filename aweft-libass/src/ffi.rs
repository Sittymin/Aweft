mod raw {
    #![allow(non_upper_case_globals)]
    #![allow(non_camel_case_types)]
    #![allow(non_snake_case)]
    #![allow(dead_code)]
    include!(concat!(env!("OUT_DIR"), "/bindings.rs"));
}

use std::ffi::c_void;
use std::os::raw::{c_char, c_int};

pub use self::raw::{
    ASS_DefaultFontProvider,
    ASS_Event,
    ASS_Image,
    // 基础类型
    ASS_Library,
    ASS_OverrideBits,
    ASS_Renderer,
    ASS_Style,
    ASS_Track,
    ass_alloc_event,
    ass_alloc_style,
    ass_clear_fonts,

    ass_configure_prune,
    ass_flush_events,

    ass_free,

    ass_free_track,
    // 字体函数
    ass_get_available_font_providers,

    ass_library_done,
    // 库函数
    ass_library_init,
    ass_library_version,
    // 内存管理
    ass_malloc,
    // 轨道函数
    ass_new_track,
    ass_process_data,
    ass_process_force_style,
    ass_prune_events,
    ass_render_frame,
    ass_renderer_done,
    // 渲染器函数
    ass_renderer_init,
    ass_set_extract_fonts,
    ass_set_font_scale,
    ass_set_fonts,
    ass_set_fonts_dir,

    ass_set_frame_size,
    ass_set_hinting,
    ass_set_line_position,
    ass_set_line_spacing,
    ass_set_margins,
    ass_set_pixel_aspect,
    ass_set_selective_style_override,
    ass_set_selective_style_override_enabled,

    ass_set_shaper,
    ass_set_storage_size,
    ass_set_style_overrides,
    ass_set_use_margins,
    ass_style,
    // 包装器函数
    clear_message_callback,
    set_rust_message_callback,
};

// 重新定义 ass_add_font 以使用正确的 c_void 类型（而不是 c_char）
// 因为字体数据是二进制数据，应该使用 void* 而不是 char*
#[allow(clashing_extern_declarations)]
#[link(name = "ass")]
unsafe extern "C" {
    pub fn ass_add_font(
        library: *mut ASS_Library,
        name: *const c_char,
        data: *const c_void,
        data_size: c_int,
    );
}
