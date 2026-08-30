# 视频播放

视频播放位于 `aweft-gui/src/render/video_sink/`。普通媒体 Action 由
`reducers/media.rs` 编排管线并返回 Video Tag，经 `projections/video.rs` 更新
派生 UI 模型；渲染层负责取帧、颜色转换和向 Slint 提交图像。播放中的时间由
视频管线位置驱动动画标量。暂停时不依赖渲染轮询推进时间：播放头/时间条的主动 seek
由 reducer 写入会话并返回 `VideoTimeChanged`，暂停态预览帧由 preroll 回调交付；
播放态才由渲染回调按帧读取管线位置并做实时增量同步。

普通会话结构、选择/主视频、视口位置、静音、seek 时间和 transport 播放状态都
必须经过 `Video DataId → TagKind → projection`。帧图像、逐帧播放时钟和后台波形
批次属于高频异步实时例外：它们可以直接对持久 `VecModel` 做行级增量更新，但
不得被当作普通 reducer UI 写入。

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

`video_sink/sink.rs` 负责 appsink、最新帧槽位与渲染资源生命周期，`frame.rs`
负责帧上传/导入和 YUV 转换，`dmabuf.rs` 只负责 DMA-BUF 平面导入。
`video_sink/tests.rs` 覆盖着色器布局和真实管线入口。

生产 appsink 保持与 `main` 相同的低延迟语义：由媒体时钟同步，内部队列上限为
一帧且允许丢弃过期帧。GStreamer 的 `new_sample` 回调取出播放帧，
`new_preroll` 回调取出暂停/seek 的预滚动帧；两者都只覆盖同一个最新帧槽位并请求
一次 Slint 重绘。`BeforeRendering` 仅在槽位确有新帧时更新图像和播放位置，不使用
定时刷新或自激 `request_redraw` 循环，也不让解码器等待 UI 逐帧追赶。

输出 RGBA 纹理使用固定大小的槽位池，而不是每帧创建新纹理。Slint 在
`BeforeRendering` 收到当前图像后，视频 sink 在 `AfterRendering` 注册 queue
完成回调；对应 GPU submission 完成前不会复用槽位。分辨率切换时，旧槽位等
全部空闲后才整体替换。

## 生命周期

支持多视频会话：`AppState::video_sessions` 保存每条会话（管线 + sink + WGPU
输出 + 视口位置 + 播放位置），`main_video: Option<usize>` 为主视频下标（唯一带视口
矩形参考线，无会话时为 `None`）。`selected_videos` 是有序去重的选择集合，末项为当前
焦点；集合为空时焦点回落主视频。

- **追加**：`VideoAction::Append` 先构建候选会话，GPU 连接并完成暂停态预滚动后才
  入列；失败回滚、不改动 store。首个会话自动成为主视频；新会话相对当前选中
  （无则主视频）级联错开落点（`media_video_cascade_anchor`），非主视频可在视口拖拽移动。
- **原地替换**（右键"更换视频"）：`VideoAction::Replace` 构建新会话连接成功后才
  拆旧会话，失败保留旧会话；位置/主/选中关系不变。
- **选中/主视频**：点击视口视频（重叠按选中顺序循环，`pick_video_at`）或点击
  各面板控制时间条（`Select`）选中；右键非主视频"设为主视频"（`SetMain`，与旧主
  视频交换视口位置）。选中视频视口描边（常驻彩虹色描边，非选中暗淡）。
- **时间条**：每个视频在各面板（曲线/摄影表/NLA/时间线）有独立彩虹色控制时间条
  （颜色复用 `store/utils/rainbow.rs` 曲线编辑器同款调色板）；时间条内嵌于各面板
  `TimeRuler`（竖线绘制在播放头控制柄之下，不遮挡显示时间的框框；当前焦点视频的
  时间显示框与竖线均可作为拖拽激活区），画布层叠加 `VideoTimeBarLines` 彩色竖线
  （标尺/画布竖线均居中于时间点）。
  点击选中；普通拖拽只 seek 当前焦点，Shift 拖拽按同一时间增量移动全部选中视频，
  保持它们的相对时间差。拖拽会话显式记录目标和各自播放态，播放中
  `render_sync_video_playback_time`
  随帧写入各会话位置并局部刷新时间条模型；**活跃视频（选中无则主视频）**驱动动画
  域当前时间——显示时间的框框/播放头跟随当前选中视频，而不是始终在主视频。
- **播放/暂停**：时间线 Header 中区的播放/暂停按钮是全局 transport；
  `VideoAction::Play/Pause/TogglePlayback` 同时作用于全部视频会话，各会话从各自当前
  位置继续。选择只决定 seek/时间显示和唯一音频 owner（无选中时为主视频），避免
  多路参考音频叠加。`video_playing` 由 `media_apply_video_playback` 即时设置，并随帧按全部管线
  状态校正。暂停态 seek 不临时切到 Playing，由 preroll 回调交付预览帧。

## 音频波形

音频波形使用独立解码管线，不消费播放会话的 appsink。所有媒体共享一个后台
worker，任务串行解码，避免追加多个视频时同时全速占用 CPU/解码器。每个媒体项
持有取消令牌；替换媒体和 teardown 会取消对应任务，旧任务不能覆盖新模型。

波形以固定 1ms 时间桶累计峰值；横向主标尺与波形共享 1/2/5×10^k 的 nice 时间粒度。每个 AudioWaveform Area 按当前主刻度的十分之一选择 2ms、5ms、10ms、20ms、50ms……等预聚合层，因此一个主刻度固定约显示 10 个柱子；不同 Area 各自根据自己的视图缩放选择层级。appsink 每产生一个 PCM 缓冲区就立即提交发生变化的区间，不使用固定轮询等待；UI 保留持久的
`VecModel<f32>`，通过 `set_row_data`/`push` 原地更新，不按批次克隆完整振幅数组或
重建嵌套模型。

渲染器 teardown 会停止全部管线并释放 sink 的 GPU 资源。
