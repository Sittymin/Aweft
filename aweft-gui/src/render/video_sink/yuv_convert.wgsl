// YUV → RGBA 离屏转换（BT.709 / BT.601 矩阵与 limited/full 范围按 uniform 自适应）。
// 支持多种平面布局（uniform frame_layout.mode）：
//   0 = NV12：Y(R8) + UV 交错(RG8)
//   1 = I420/YV12/Y42B/Y444：Y + U + V 三个独立 R8 平面
//   2 = P010：Y(R16) + UV 交错(RG16)，16bit 容器存 10bit 高位
//   3 = Y410：一个 packed 32-bit 平面；sysmem 用 Rgba8Unorm 保存原始字节，
//              DMABuf 用原生 Rgb10a2Unorm 采样
// color 中的三个开关分别表示矩阵、范围和交错 UV 顺序；frame_layout.mode 表示布局。
// 目前只实现 BT.601/BT.709；BT.2020 等 primaries/white-point 组合需要新增
// 对应的矩阵选择值，不能把当前 BT.709 系数当作通用值。

struct Params {
    color: ColorParams,
    frame_layout: LayoutParams,
};

struct ColorParams {
    matrix_is_601: f32,
    full_range: f32,
    swap_uv: f32,
    _padding: f32,
};

struct LayoutParams {
    mode: u32,
    packed_10bit: u32,
    _padding: vec2<u32>,
};

const MODE_NV12: u32 = 0u;
const MODE_PLANAR: u32 = 1u;
const MODE_P010: u32 = 2u;
const MODE_Y410: u32 = 3u;

@group(0) @binding(0) var tex_sampler: sampler;
@group(0) @binding(1) var y_tex: texture_2d<f32>;
@group(0) @binding(2) var uv_tex: texture_2d<f32>;
@group(0) @binding(3) var u_tex: texture_2d<f32>;
@group(0) @binding(4) var v_tex: texture_2d<f32>;
@group(0) @binding(5) var<uniform> p: Params;

struct VsOut {
    @builtin(position) pos: vec4<f32>,
    @location(0) uv: vec2<f32>,
};

@vertex
fn vs(@builtin(vertex_index) vi: u32) -> VsOut {
    // 全屏大三角形覆盖 [-1,1]，uv 翻转 Y（纹理原点在左上）
    var out: VsOut;
    let vertex_x = f32((vi << 1u) & 2u);
    let vertex_y = f32(vi & 2u);
    let vertex_uv = vec2<f32>(vertex_x, vertex_y);
    out.pos = vec4<f32>(vertex_uv * 2.0 - 1.0, 0.0, 1.0);
    out.uv = vec2<f32>(vertex_x, 1.0 - vertex_y);
    return out;
}

// ITU-R BT.709-6 / BT.601-7 limited-range conversion coefficients.
const Y_EXPAND_LIMITED: f32 = 255.0 / 219.0;
const Y_BLACK_LEVEL: f32 = 16.0 / 256.0;
const CHROMA_MIDPOINT: f32 = 0.5;

// ITU-R BT.709-6, Table 3 (8-bit limited-range Y'CbCr coefficients).
const BT709_CR_TO_R: f32 = 1.793;
const BT709_CB_TO_G: f32 = -0.213;
const BT709_CR_TO_G: f32 = -0.533;
const BT709_CB_TO_B: f32 = 2.112;

// ITU-R BT.601-7, studio-swing Y'CbCr coefficients.
const BT601_CR_TO_R: f32 = 1.596;
const BT601_CB_TO_G: f32 = -0.391;
const BT601_CR_TO_G: f32 = -0.813;
const BT601_CB_TO_B: f32 = 2.018;

// JPEG/full-range Y'CbCr coefficients (BT.601-style primaries).
const FULL_CR_TO_R: f32 = 1.402;
const FULL_CB_TO_G: f32 = -0.344;
const FULL_CR_TO_G: f32 = -0.714;
const FULL_CB_TO_B: f32 = 1.772;

// limited range：Y 已含展开，色度以 0.5 为零点
// WGSL mat3x3 为列主序：列 0=Y 系数，列 1=U，列 2=V，rgb = M * yuv
fn matrix_709_limited() -> mat3x3<f32> {
    return mat3x3<f32>(
        vec3<f32>(Y_EXPAND_LIMITED, Y_EXPAND_LIMITED, Y_EXPAND_LIMITED),
        vec3<f32>(0.000, BT709_CB_TO_G, BT709_CB_TO_B),
        vec3<f32>(BT709_CR_TO_R, BT709_CR_TO_G, 0.000),
    );
}

fn matrix_601_limited() -> mat3x3<f32> {
    return mat3x3<f32>(
        vec3<f32>(Y_EXPAND_LIMITED, Y_EXPAND_LIMITED, Y_EXPAND_LIMITED),
        vec3<f32>(0.000, BT601_CB_TO_G, BT601_CB_TO_B),
        vec3<f32>(BT601_CR_TO_R, BT601_CR_TO_G, 0.000),
    );
}

// full range：无展开，系数为标准 full 形式
fn matrix_full() -> mat3x3<f32> {
    return mat3x3<f32>(
        vec3<f32>(1.000, 1.000, 1.000),
        vec3<f32>(0.000, FULL_CB_TO_G, FULL_CB_TO_B),
        vec3<f32>(FULL_CR_TO_R, FULL_CR_TO_G, 0.000),
    );
}

@fragment
fn fs(in: VsOut) -> @location(0) vec4<f32> {
    let mode = p.frame_layout.mode;
    var y: f32;
    var uv: vec2<f32>;

    switch mode {
    case MODE_NV12, MODE_P010: {
        // NV12/P010：Y + UV 交错色度平面。
        // P010 stores the 10 significant bits in bits 6..15 of a 16-bit word.
        const P010_MSB_SCALE: f32 = 65535.0 / (1023.0 * 64.0);
        let scale = select(1.0f, P010_MSB_SCALE, mode == MODE_P010);
        let sampled_y = textureSampleLevel(y_tex, tex_sampler, in.uv, 0.0);
        let sampled_uv = textureSampleLevel(uv_tex, tex_sampler, in.uv, 0.0);
        let sampled_y_luma = sampled_y.r;
        let sampled_u = sampled_uv.r;
        let sampled_v = sampled_uv.g;
        y = sampled_y_luma * scale;
        uv = vec2<f32>(sampled_u, sampled_v) * scale;
        // UV 交换只适用于 NV12/P010 的交错色度平面；独立 U/V 平面
        // 的顺序已由布局解析确定。
        if (p.color.swap_uv != 0.0) {
            uv = uv.yx;
        }
    }
    case MODE_PLANAR: {
        // I420/YV12/Y42B/Y444：U、V 各自独立平面。
        let sampled_y = textureSampleLevel(y_tex, tex_sampler, in.uv, 0.0);
        let sampled_u = textureSampleLevel(u_tex, tex_sampler, in.uv, 0.0);
        let sampled_v = textureSampleLevel(v_tex, tex_sampler, in.uv, 0.0);
        y = sampled_y.r;
        uv = vec2<f32>(sampled_u.r, sampled_v.r);
    }
    case MODE_Y410: {
        // GStreamer Y410 little-endian 位域：
        // bits 0..9=U，10..19=Y，20..29=V，30..31=A。
        if (p.frame_layout.packed_10bit != 0u) {
            // Rgb10a2Unorm 已由 Vulkan 按 U/Y/V/A 归一化采样。
            let size = vec2<f32>(textureDimensions(y_tex, 0));
            let coord = vec2<i32>(floor(in.uv * size));
            let packed = textureLoad(y_tex, coord, 0);
            let packed_u = packed.r;
            let packed_y = packed.g;
            let packed_v = packed.b;
            y = packed_y;
            uv = vec2<f32>(packed_u, packed_v);
        } else {
            // sysmem Rgba8Unorm 只保存原始 byte；分别解码各 byte，避免用
            // f32 重建 32-bit word 时丢失低位。
            let size = vec2<f32>(textureDimensions(y_tex, 0));
            let coord = vec2<i32>(floor(in.uv * size));
            let packed = textureLoad(y_tex, coord, 0) * 255.0;
            let b0 = floor(packed.r + 0.5);
            let b1 = floor(packed.g + 0.5);
            let b2 = floor(packed.b + 0.5);
            let b3 = floor(packed.a + 0.5);
            let u10 = b0 + (b1 - floor(b1 / 4.0) * 4.0) * 256.0;
            let y10 = floor(b1 / 4.0) + (b2 - floor(b2 / 16.0) * 16.0) * 64.0;
            let v10 = floor(b2 / 16.0) + (b3 - floor(b3 / 64.0) * 64.0) * 16.0;
            y = y10 / 1023.0;
            uv = vec2<f32>(u10, v10) / 1023.0;
        }
    }
    default: {
        // Rust 侧只会生成已知模式；未知值使用 NV12 解释，避免未初始化输出。
        let sampled_y = textureSampleLevel(y_tex, tex_sampler, in.uv, 0.0);
        let sampled_uv = textureSampleLevel(uv_tex, tex_sampler, in.uv, 0.0);
        y = sampled_y.r;
        uv = vec2<f32>(sampled_uv.r, sampled_uv.g);
    }
    }

    var rgb: vec3<f32>;
    if (p.color.full_range != 0.0) {
        // full range：Y 不做 16-235 展开
            let yuv = vec3<f32>(y, uv.x - CHROMA_MIDPOINT, uv.y - CHROMA_MIDPOINT);
        rgb = matrix_full() * yuv;
    } else {
        // limited range：Y 减黑电平后由 Y_EXPAND_LIMITED 列完成展开
        // （WGSL 的 select 不支持矩阵类型，用分支选择）
        let yuv = vec3<f32>(
            y - Y_BLACK_LEVEL,
            uv.x - CHROMA_MIDPOINT,
            uv.y - CHROMA_MIDPOINT,
        );
        if (p.color.matrix_is_601 != 0.0) {
            rgb = matrix_601_limited() * yuv;
        } else {
            rgb = matrix_709_limited() * yuv;
        }
    }
    return vec4<f32>(clamp(rgb, vec3<f32>(0.0), vec3<f32>(1.0)), 1.0);
}
