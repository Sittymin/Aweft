//! 字幕合成的无头像素校验：`\pos` 说的位置，必须就是输出纹理上墨迹的位置；
//! 光栅尺寸换了倍率，画面必须跟着等比放大而不是被插值拉糊。
//!
//! 这里刻意不走 `SubtitlePipeline`（它只做「脚本字节 → 出帧」的调度），而是
//! `Renderer` + `SubtitleCompositor` 两段真实生产代码直接串起来，然后回读整帧。
//! 覆盖的是只有 GPU 才能发现的问题：usage 位不足、投影符号、三角形朝向、图集行距。
//!
//! 对账用「内容包围盒」而不是图块的名义盒：见 `covered_bounds` 的说明。

use std::sync::Arc;

use aweft_ass::render::{Renderer, Tile};
use slint::wgpu_30::wgpu;

use super::compositor::{Composition, SubtitleCompositor};
use super::pipeline::raster_size;
use crate::render::video_sink::WgpuHandles;
use crate::render::wgpu_test_support::{headless_wgpu, readback_rgba};

const FRAME_W: u32 = 640;
const FRAME_H: u32 = 360;

/// 一行带 `\pos` 的字幕。样式对齐 2（底部居中），所以墨迹的下边界≈锚点 y、
/// 水平中心≈锚点 x。锚点刻意取在非居中的 (160,300)：任何一轴的翻转或偏移都会
/// 把墨迹推到明显不同的位置。
const SCRIPT: &[u8] = b"\
[Script Info]
ScriptType: v4.00+
PlayResX: 640
PlayResY: 360
WrapStyle: 0
ScaledBorderAndShadow: yes

[V4+ Styles]
Format: Name, Fontname, Fontsize, PrimaryColour, SecondaryColour, OutlineColour, BackCol\
our, Bold, Italic, Underline, StrikeOut, ScaleX, ScaleY, Spacing, Angle, BorderStyle, Out\
line, Shadow, Alignment, MarginL, MarginR, MarginV, Encoding
Style: Default,DejaVu Sans,48,&H00FFFFFF,&H000000FF,&H00000000,&H00000000,0,0,0,0,100,10\
0,0,0,1,2,0,2,10,10,10,1

[Events]
Format: Layer, Start, End, Style, Name, MarginL, MarginR, MarginV, Effect, Text
Dialogue: 0,0:00:00.00,0:00:10.00,Default,,0,0,0,,{\\pos(160,300)}Qa
";

/// 12 行长文本、左上对齐、字号 120。算式（实测）：96 号时每行块宽 995，图集一排塞得
/// 下两个，12 行只堆到 960 高，撑不爆初值 1024；120 号每行 ~1244 > 2048/2，一排只能
/// 放一个，于是每行两块（填充 + 描边）共 24 块竖着堆 ~2400 行，必然超出初值 ⇒
/// 合成时必须把图集长大。帧高 1500 让内容完整落在帧内（12 行 × 120 + 边距），
/// 免得像素对账被裁剪掩盖。
const TALL_SCRIPT: &[u8] = b"\
[Script Info]
ScriptType: v4.00+
PlayResX: 1920
PlayResY: 1500
WrapStyle: 2
ScaledBorderAndShadow: yes

[V4+ Styles]
Format: Name, Fontname, Fontsize, PrimaryColour, SecondaryColour, OutlineColour, BackCol\
our, Bold, Italic, Underline, StrikeOut, ScaleX, ScaleY, Spacing, Angle, BorderStyle, Out\
line, Shadow, Alignment, MarginL, MarginR, MarginV, Encoding
Style: Default,DejaVu Sans,120,&H00FFFFFF,&H000000FF,&H00000000,&H00000000,0,0,0,0,100,1\
00,0,0,1,2,0,7,10,10,10,1

[Events]
Format: Layer, Start, End, Style, Name, MarginL, MarginR, MarginV, Effect, Text
Dialogue: 0,0:00:00.00,0:00:10.00,Default,,0,0,0,,WWWWWWWWWWWW\\NWWWWWWWWWWWW\\NWWWWWWWWW\
WWW\\NWWWWWWWWWWWW\\NWWWWWWWWWWWW\\NWWWWWWWWWWWW\\NWWWWWWWWWWWW\\NWWWWWWWWWWWW\\NWWWWWWWW\
WWWW\\NWWWWWWWWWWWW\\NWWWWWWWWWWWW\\NWWWWWWWWWWWW
";

const ANCHOR: (f32, f32) = (160.0, 300.0);
type Box2 = (i64, i64, i64, i64);

/// 图块里真正有覆盖的最小矩形，换算到整帧坐标，含端点。
///
/// 名义盒（`x, y, width, height`）比内容宽是 libass 的正常形态：`stride` 是行字节距
/// （实测可远大于 `width`），尾部若干行/列全零，`bitmap` 甚至短于 `stride * height`。
/// 所以像素对账只能按内容盒来，按名义盒会稳定误报「少画了一圈」。
fn covered_bounds(tile: &Tile) -> Option<Box2> {
    let mut bounds: Option<Box2> = None;
    for row in 0..tile.height {
        let start = row * tile.stride;
        // 尾行可以短于一整行：取不到就到此为止（行序递增，后面也没有）
        let Some(bytes) = tile.bitmap.get(start..start + tile.width) else {
            break;
        };
        for (column, value) in bytes.iter().enumerate() {
            if *value == 0 {
                continue;
            }
            let x = tile.x as i64 + column as i64;
            let y = tile.y as i64 + row as i64;
            let (x0, y0, x1, y1) = bounds.unwrap_or((x, y, x, y));
            bounds = Some((x0.min(x), y0.min(y), x1.max(x), y1.max(y)));
        }
    }
    bounds
}

/// 输出纹理上有覆盖（alpha > 0）的像素包围盒，含端点。
fn ink_bounds(pixels: &[u8], width: u32, height: u32) -> Option<Box2> {
    let mut bounds: Option<Box2> = None;
    for y in 0..height {
        for x in 0..width {
            if pixels[(y * width * 4 + x * 4 + 3) as usize] == 0 {
                continue;
            }
            let (x0, y0, x1, y1) = bounds.unwrap_or((x as i64, y as i64, x as i64, y as i64));
            bounds = Some((
                x0.min(x as i64),
                y0.min(y as i64),
                x1.max(x as i64),
                y1.max(y as i64),
            ));
        }
    }
    bounds
}

/// 一次「libass 出块 → 合成器出纹理 → 回读整帧」的结果。
struct Raster {
    pixels: Vec<u8>,
    /// libass 给出的字形块内容盒并集（整帧坐标）
    expected: Box2,
    /// 输出纹理上的墨迹包围盒
    ink: Box2,
    /// 本帧之后图集的容量
    atlas: (u32, u32),
}

fn union(items: impl Iterator<Item = Box2>) -> Option<Box2> {
    items.reduce(|(x0, y0, x1, y1), (x2, y2, x3, y3)| {
        (x0.min(x2), y0.min(y2), x1.max(x3), y1.max(y3))
    })
}

/// 出一次帧。核心不变量（墨迹 == libass 的块内容，逐像素）收在这里，
/// 每个用它的测试都顺带覆盖了投影朝向、图集行距与 UV。
fn raster(handles: &WgpuHandles, size: (u32, u32), script: &[u8]) -> Raster {
    let (device, queue) = (handles.1.clone(), handles.2.clone());

    let mut renderer = Renderer::from_script(size.0, size.1, script).expect("libass 建轨失败");
    let frame = renderer.render(1_000).expect("libass 出帧失败");
    assert!(frame.changed, "首帧就报未变化，changed 判定有问题");
    assert!(!frame.tiles.is_empty(), "字幕行没有产生字形块");

    let expected =
        union(frame.tiles.iter().filter_map(covered_bounds)).expect("字形块全都没有覆盖内容");
    let (width, height) = size;
    // 前提自检：内容必须完整落在帧内，否则下面的对账会被裁剪掩盖
    assert!(
        expected.0 >= 0
            && expected.1 >= 0
            && expected.2 < width as i64
            && expected.3 < height as i64,
        "字形内容超出整帧范围：{expected:?}"
    );

    let mut compositor = SubtitleCompositor::new(handles);
    compositor.set_raster_size(size);
    assert!(
        matches!(
            compositor.compose(&frame.tiles).expect("字幕合成失败"),
            Composition::Frame(_)
        ),
        "有字形块时应当交出一帧纹理"
    );
    compositor.after_rendering();

    let textures: Vec<&wgpu::Texture> = compositor.target_textures().collect();
    assert_eq!(textures.len(), 1, "本帧应当只用掉一个输出槽位");
    let pixels = readback_rgba(&device, &queue, textures[0], width, height);
    let ink = ink_bounds(&pixels, width, height).expect("输出纹理全透明：一个字都没画上");
    // 图集纹素与帧像素严格 1:1（邻域采样 + 半纹素对齐的 UV），所以逐像素相等
    assert_eq!(ink, expected, "墨迹位置与 libass 给的块内容不一致");

    Raster {
        pixels,
        expected,
        ink,
        atlas: compositor.atlas_size(),
    }
}

fn handles() -> WgpuHandles {
    let (instance, device, queue) = headless_wgpu();
    (Arc::new(instance), Arc::new(device), Arc::new(queue))
}

#[test]
fn ink_lands_on_the_frame_coordinates_libass_gave() {
    let handles = handles();
    let shot = raster(&handles, (FRAME_W, FRAME_H), SCRIPT);

    // 再对一次脚本里的 \pos：底部居中对齐时墨迹水平中心≈锚点 x、下边界≈锚点 y。
    // 这一条挡的是「整块自洽地画错地方」（投影偏移、Y 轴翻转等）。
    let (x0, _y0, x1, y1) = shot.ink;
    let center_x = (x0 + x1) as f32 / 2.0;
    assert!(
        (center_x - ANCHOR.0).abs() <= 12.0,
        "墨迹水平中心 {center_x} 离 \\pos 锚点 {} 太远",
        ANCHOR.0
    );
    assert!(
        (y1 as f32 - ANCHOR.1).abs() <= 12.0,
        "墨迹下边界 {y1} 离 \\pos 锚点 {} 太远（Y 轴或对齐出错？）",
        ANCHOR.1
    );
    // 远离锚点的区域必须仍然干净：证明没有整屏误绘
    let far_corner = shot.pixels[((FRAME_H - 2) * FRAME_W * 4 + (FRAME_W - 2) * 4 + 3) as usize];
    assert_eq!(far_corner, 0, "右下角被误绘，字幕覆盖层会糊满整帧");
    // 小帧不该占走大块显存
    assert_eq!(shot.atlas, (2048, 1024), "小帧把图集撑大了");
}

#[test]
fn a_doubled_raster_is_a_uniform_magnification_of_the_layout() {
    let handles = handles();
    let one = raster(&handles, (FRAME_W, FRAME_H), SCRIPT);
    let two = raster(&handles, (FRAME_W * 2, FRAME_H * 2), SCRIPT);

    // 光栅换成 2 倍后，libass 是按 2 倍帧重排并重栅化的：内容盒应当整体放大 2 倍。
    // 挡的是「只把小图拉大」（那样墨迹仍按 1 倍块画，内容盒不会翻倍）与
    // 「PlayRes 跟着变」（那样位置不再与导出的 .ass 同比例）。
    // 容差 6px：字形 hinting 让 2 倍的墨迹边界不是严格的 2 倍。
    let (a0, a1, a2, a3) = one.expected;
    let want = (2 * a0, 2 * a1, 2 * a2 + 1, 2 * a3 + 1);
    let (g0, g1, g2, g3) = two.ink;
    let (w0, w1, w2, w3) = want;
    for (index, (got, want)) in [(g0, w0), (g1, w1), (g2, w2), (g3, w3)]
        .into_iter()
        .enumerate()
    {
        assert!(
            (got - want).abs() <= 6,
            "2 倍光栅的第 {index} 条边界 {got} 不在 1 倍结果的两倍位置 {want} 附近：\
             1 倍墨迹 {:?}，2 倍墨迹 {:?}",
            one.ink,
            two.ink
        );
    }
}

#[test]
fn a_glyph_stack_taller_than_the_atlas_grows_it() {
    let handles = handles();
    let shot = raster(&handles, (1920, 1500), TALL_SCRIPT);
    let content_height = shot.expected.3 - shot.expected.1 + 1;

    assert!(
        content_height > 1024,
        "测试用例没起到增长图集的作用：字形内容只有 {content_height} 高，没超过图集初值 1024"
    );
    assert!(
        shot.atlas.1 > 1024,
        "字形内容超过初值容量却没让图集长大：{:?}",
        shot.atlas
    );
    // 宽度这一轴不需要动（单行没超过 2048），只长高
    assert_eq!(shot.atlas.0, 2048, "宽度没超容量却被撑大：{:?}", shot.atlas);
}

#[test]
fn raster_size_buckets_up_and_stays_inside_the_limits() {
    const MAX: u32 = 8192;
    let layout = (1920, 1080);

    // 覆盖层还没报尺寸：按排版尺寸 1:1
    assert_eq!(raster_size(layout, (0.0, 0.0), MAX), layout);
    // 正好 2 倍（HiDPI 下放大到满屏）：严格 2 倍，一个像素不多一个像素不少
    assert_eq!(raster_size(layout, (3840.0, 2160.0), MAX), (3840, 2160));
    // 凑不满一档的零头向上吸收：0.89 倍 → 1 倍（永不生成比屏幕小的纹理）
    assert_eq!(raster_size(layout, (1707.0, 961.0), MAX), layout);
    // 两轴取较大值：非等比的矩形也保证覆盖住较大的那一轴
    assert_eq!(raster_size(layout, (1920.0, 2160.0), MAX), (3840, 2160));
    // 下限 1/4：缩到很小看一眼时也不给 libass 一个退化尺寸
    assert_eq!(raster_size(layout, (96.0, 54.0), MAX), (480, 270));

    // 极端放大：钳进像素预算与设备单边上限，两轴同倍率（比例不变）
    let huge = raster_size(layout, (30_000.0, 16_875.0), MAX);
    assert!(huge.0 <= MAX && huge.1 <= MAX, "超出设备单边上限: {huge:?}");
    assert!(
        f64::from(huge.0) * f64::from(huge.1) <= 16_000_000.0,
        "超出像素预算: {huge:?}"
    );
    let ratio = f64::from(huge.0) / f64::from(huge.1);
    assert!(
        (ratio - 1920.0 / 1080.0).abs() < 0.01,
        "钳完把画面比例钳变了: {huge:?}（{ratio}）"
    );
}
