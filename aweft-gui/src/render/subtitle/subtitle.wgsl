// 实例化渲染：每个实例的数据结构
// 这个结构体必须和 Rust 中的 SubtitleInstanceData 内存布局一致
struct InstanceData {
    color: vec4<f32>,
    position: vec2<f32>,
    size: vec2<f32>,
    atlas_uv_offset: vec2<f32>,
    atlas_uv_size: vec2<f32>,
};

// 正交投影矩阵（全局共享）
@group(0) @binding(0)
var<uniform> u_projection: mat4x4<f32>;

@group(0) @binding(1)
var t_alpha_mask: texture_2d<f32>;

@group(0) @binding(2)
var s_sampler: sampler;

struct VertexOutput {
    @builtin(position) clip_position: vec4<f32>,
    @location(0) tex_coords: vec2<f32>,
    @location(1) color: vec4<f32>, // 从实例数据传递颜色到片元着色器
};

// 顶点着色器：使用实例化渲染，为每个实例生成矩形
@vertex
fn vs_main(
    @builtin(vertex_index) in_vertex_index: u32,
    @builtin(instance_index) instance_index: u32,
    // 从实例缓冲区读取每个字幕图块的属性
    @location(0) instance_color: vec4<f32>,
    @location(1) instance_position: vec2<f32>,
    @location(2) instance_size: vec2<f32>,
    @location(3) instance_atlas_uv_offset: vec2<f32>,
    @location(4) instance_atlas_uv_size: vec2<f32>,
) -> VertexOutput {
    // 定义矩形四个角的局部空间坐标 (0,0) 到 (1,1)
    // 屏幕坐标系：(0,0) 左上角，Y向下增长
    let positions = array<vec2<f32>, 4>(
        vec2<f32>(0.0, 0.0), // 0: 左上
        vec2<f32>(1.0, 0.0), // 1: 右上
        vec2<f32>(0.0, 1.0), // 2: 左下
        vec2<f32>(1.0, 1.0)  // 3: 右下
    );

    // 定义组成两个三角形的6个顶点的索引
    // 这会依次画出 (0,2,1) 和 (1,2,3) 两个三角形
    let indices = array<u32, 6>(0, 2, 1, 1, 2, 3);

    // 1. 根据 GPU 传入的顶点索引 (0-5)，从我们的索引数组中找到对应的“角”的索引
    let corner_index = indices[in_vertex_index];

    // 2. 使用“角”的索引，从位置数组中获取该角的局部坐标
    let local_pos = positions[corner_index];

    // 3. 将局部坐标 (0-1范围) 转换到屏幕像素坐标
    let screen_pos = instance_position + local_pos * instance_size;
    
    // 将 screen_pos 转换为裁剪空间坐标
    let clip_pos = u_projection * vec4<f32>(screen_pos, 0.0, 1.0);

    var out: VertexOutput;
    out.clip_position = clip_pos;

    // 根据图集信息计算正确的纹理坐标
    // local_pos 的范围是 [0.0, 1.0]，正好用来在图块内部进行插值
    // 不需要翻转Y轴，因为我们已经修正了顶点位置
    let uv_local = vec2<f32>(local_pos.x, local_pos.y);
    out.tex_coords = instance_atlas_uv_offset + uv_local * instance_atlas_uv_size;
    
    // 将实例颜色传递给片元着色器
    out.color = instance_color;
    return out;
}

// 片元着色器：计算最终颜色
@fragment
fn fs_main(in: VertexOutput) -> @location(0) vec4<f32> {
    // 1. 从 Alpha 纹理中采样，得到当前像素的形状覆盖
    let shape_alpha = textureSample(t_alpha_mask, s_sampler, in.tex_coords).r;

    // 2. 乘上图层自身的不透明度。in.color.a 已是不透明度语义：libass 的
    // 「低字节是透明度」在 aweft-libass 的 from_libass_u32 里翻转，渲染层经
    // aweft-ass 的 render::Tile::color 拿到它（libass 按透明度 0xFF 丢弃全透明
    // 位图，ass_render.c:3416）。
    // \alpha、\fad、半透明描边与阴影全靠这一项生效。
    let final_alpha = shape_alpha * in.color.a;

    // 3. 预乘输出，配合 wgpu 的 ALPHA_BLENDING
    let final_color = vec4<f32>(in.color.rgb * final_alpha, final_alpha);

    return final_color;
}
