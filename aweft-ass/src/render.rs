//! ASS 脚本 → 字形块：把 libass 挡在本 crate 后面。
//!
//! 本模块只管 ASS 领域的一段真相：给一份脚本字节与渲染帧尺寸，问某一时刻
//! 画面上有哪些字形块（一块单通道覆盖 + 一个颜色 + 一个帧内位置）。把这些块
//! 铺成纹理、贴进窗口是渲染层的事，它不需要知道 libass 的存在。

use aweft_libass::{color::AssColorRgba, AssScriptRenderer};
use thiserror::Error;

/// 脚本渲染失败。字形块的具体来源（libass）不外泄，只带一句人话。
#[derive(Debug, Error)]
#[error("ASS 脚本渲染失败: {message}")]
pub struct RenderError {
    pub message: String,
}

type Result<T> = std::result::Result<T, RenderError>;

/// 一块字形覆盖：libass 已把一段文字（或它的描边、阴影）合并成一张单通道图。
#[derive(Debug, Clone)]
pub struct Tile {
    /// 覆盖值，逐行按 `stride` 排布；每像素一字节，最后一行可以短于 `stride`
    pub bitmap: Vec<u8>,
    pub stride: usize,
    pub width: usize,
    pub height: usize,
    /// 块左上角在渲染帧内的像素坐标
    pub x: i32,
    pub y: i32,
    color: AssColorRgba,
}

impl Tile {
    ///  RGBA 颜色，各分量 0..1。alpha 是**不透明度**（libass 那个「低字节是
    /// 透明度」的约定已在本 crate 的 color 层翻转过来）。
    pub fn color(&self) -> [f32; 4] {
        self.color.to_f32_array()
    }
}

/// 一次 `render` 的结果。
#[derive(Debug)]
pub struct Frame {
    /// 该时刻画面上的字形块，按 libass 给出的顺序（同层内后画的压在前面的上面）
    pub tiles: Vec<Tile>,
    /// 与上一次调用相比字形块列表是否变了。静止的字幕据此跳过重绘。
    pub changed: bool,
}

/// 一份已灌进 libass 的 ASS 脚本。渲染帧尺寸可随时用 `set_size` 更换，
/// 不影响脚本字节。
pub struct Renderer {
    inner: AssScriptRenderer,
    size: (u32, u32),
}

impl Renderer {
    /// 用脚本字节与渲染帧尺寸建轨。`script` 与导出文件同源的字节
    /// （`crate::ASSFile::to_bytes`），这样预览与导出不会分叉。
    pub fn from_script(width: u32, height: u32, script: &[u8]) -> Result<Self> {
        if width == 0 || height == 0 {
            return Err(RenderError {
                message: format!("渲染帧尺寸必须为正，收到 {width}x{height}"),
            });
        }
        let inner =
            AssScriptRenderer::from_memory(width as i32, height as i32, script, None, false)
                .map_err(|error| RenderError {
                    message: error.to_string(),
                })?;
        Ok(Self {
            inner,
            size: (width, height),
        })
    }

    pub fn size(&self) -> (u32, u32) {
        self.size
    }

    /// 换脚本字节，渲染轨不重建：libass 只把新字节重新解析成一份轨道，字体提供器
    /// 与字形缓存原样留着。渲染帧尺寸（`size`）记在渲染器上，换脚本不动它。
    ///
    /// 换完之后调用方要把这一帧当作有变化：`render` 的 `changed` 是 libass 比较
    /// 前后两次字形块列表得出的，内容恰好相同的新脚本不会报变化。
    pub fn set_script(&mut self, script: &[u8]) -> Result<()> {
        self.inner
            .load_script(script, None)
            .map_err(|error| RenderError {
                message: error.to_string(),
            })?;
        Ok(())
    }

    /// 换渲染帧尺寸。脚本里的 `PlayResX/Y` 不动，libass 因此给出「同一份排版的
    /// 整体缩放」：字形按新尺寸重新光栅化，位置与比例按新/旧排版倍率缩放。
    /// 比 `from_script` 重建省掉重新解析脚本。返回 false 表示尺寸没变。
    pub fn set_size(&mut self, width: u32, height: u32) -> Result<bool> {
        if self.size == (width, height) {
            return Ok(false);
        }
        self.inner
            .set_frame_size(width as i32, height as i32)
            .map_err(|error| RenderError {
                message: error.to_string(),
            })?;
        self.size = (width, height);
        Ok(true)
    }

    /// 取某一毫秒时刻的字形块列表。时间与脚本里 `[Events]` 的起点同一基准。
    pub fn render(&mut self, now_ms: i64) -> Result<Frame> {
        let rendered = self
            .inner
            .render_frame(now_ms)
            .map_err(|error| RenderError {
                message: error.to_string(),
            })?;
        let tiles = rendered
            .images()
            .iter()
            .map(|image| Tile {
                bitmap: image.bitmap().to_vec(),
                stride: image.stride() as usize,
                width: image.width() as usize,
                height: image.height() as usize,
                x: image.dst_x(),
                y: image.dst_y(),
                color: image.color(),
            })
            .collect();
        Ok(Frame {
            tiles,
            changed: rendered.has_changed(),
        })
    }
}
