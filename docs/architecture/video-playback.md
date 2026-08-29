# 视频播放

视频播放位于 `aweft-gui/src/render/video_sink/`，不参与编辑器数据投影。媒体
Action 由 `reducers/media.rs` 编排管线，渲染层负责取帧、颜色转换和向 Slint
提交图像；播放中的时间由视频管线位置驱动动画标量。

## 数据路径

```text
uridecodebin（硬解）
  → vapostproc（GPU 颜色转换）
  → appsink
  → wgpu YUV→RGB
  → slint::Image
```

默认路径会探测 Vulkan 外部内存能力，并在可用时协商并导入 dmabuf。运行时 caps 保留 vapostproc 提供的实际
fourcc/modifier，并过滤渲染器没有对应平面布局的格式；其余情况仍可协商到
系统内存上传路径。已协商为 DMA-BUF 但 Vulkan 导入失败的帧会被丢弃，不会把
DMA-BUF 错当成普通系统内存读取。
`AWEFT_SWAP_UV=1` 仅用于调试 U/V 顺序。

`video_sink/wgpu.rs` 负责 GStreamer sink、帧队列和 YUV 转换，`dmabuf.rs` 只
负责 DMA-BUF 平面导入。`video_sink/tests.rs` 覆盖着色器布局和真实管线入口。

输出 RGBA 纹理使用固定大小的槽位池，而不是每帧创建新纹理。Slint 在
`BeforeRendering` 收到当前图像后，视频 sink 在 `AfterRendering` 注册 queue
完成回调；对应 GPU submission 完成前不会复用槽位。分辨率切换时，旧槽位等
全部空闲后才整体替换。

## 生命周期

打开新视频时先构建候选会话；只有 GPU 连接并进入 `Playing` 成功后才释放旧管线，
失败会恢复旧会话。appsink 只保留最新帧，渲染线程在每次渲染通知中取帧；暂停时
编辑器时间轴保持时间权威。渲染器 teardown 会停止管线并释放 sink 的 GPU 资源。
