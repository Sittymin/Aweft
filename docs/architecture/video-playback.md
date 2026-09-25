# 视频播放

视频播放位于 `aweft-gui/src/render/video_sink/`。普通媒体 Action 由
`reducers/media.rs` 编排管线并返回 Video Tag；`reducers/media/playback.rs` 集中
GStreamer transport、seek、EOS 与播放时钟查询，`reducers/media/waveform.rs` 集中
后台波形任务，`reducers/media/tests.rs` 保留媒体回归测试。视频状态变更经
`projections/video.rs` 更新 `VideoState` 派生模型，音频模型经 `projections/audio.rs`
更新 `AudioState` 派生模型（两者由同一 `VideoStructureChanged` tag 驱动，拓扑不变）；
帧、播放时钟和波形批次遵循下文的实时例外。
渲染层负责取帧、颜色转换和向 Slint 提交图像。播放中的时间由
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

appsink 的 `propose_allocation` 回调（`sink.rs` 的 `build_gst_appsink_callbacks`）必须在管线进入协商之前
安装，即先 `SlintWGPUSink::connect` 再 `set_state`：DMABuf 的 VideoMeta 请求发生在 vapostproc 的
`decide_allocation`（协商阶段），错过这次查询时 vapostproc 会以 `not-negotiated` 停流，此后拉帧恒为空。
自行组装 appsink 的测试入口同受这条时序约束；绕过 `connect` 直接调 `dmabuf_planes` 的渲染入口还需自备
申请了 `VULKAN_EXTERNAL_MEMORY_DMA_BUF` 的设备与 `DmabufImporter`（见 `tests.rs` 的
`render_decoded_frame_with_skip`）。

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

## 无视频时的播放与字幕

没有视频会话就不存在媒体时钟，也就没有唤醒 Slint 事件循环的来源：
`bridge/preview_clock.rs` 的一个 repeated `slint::Timer` 在这种情况下接管播放头，
按**墙钟**累加当前时间，写入走既有的 `GraphAction::SetCurrentTime` reducer——与手动
拖播放头同一条路（同一份投影、同一次字幕出图）。上文"不使用定时刷新或自激
`request_redraw` 循环"这条禁令约束的是视频通路，理由是不让解码器等 UI 逐帧追赶；
无解码器可追时，心跳是唯一的驱动源。

心跳的存活条件是"播放中且无视频会话"，两者任一不成立（含时间条拖拽中）立即停表，
因此空闲时不产生任何唤醒。帧尺寸在同一情形下兜底为 1080P
（`AppState::preview_frame_size` / `DEFAULT_PREVIEW_FRAME_SIZE`），与视口世界尺寸的
兜底同值：空视口里字幕的位置、排版基准和覆盖层矩形仍然三者一致。

## 字幕的两个尺寸：排版尺寸与光栅尺寸

字幕通路同时有两个帧尺寸，混用会导致位置或清晰度其中之一出错：

- **排版尺寸** = `AppState::preview_frame_size()`（主视频按 PAR 校正后的显示像素尺寸，
  无视频时为 1080P 兜底）。它是写进 ASS 脚本的 `PlayResX/Y`，也就是覆盖层矩形的比例，
  与导出的 .ass 同源同字节。
- **光栅尺寸** = 字幕覆盖层在屏幕上的**物理**像素尺寸，按档位量化并钳制后的结果。
  它只用于 libass 的渲染帧与输出纹理。

libass 的 `set_frame_size` 换渲染帧而**不动脚本里的 PlayRes**，因此光栅尺寸下的画面是
同一份排版的整体等比放大——字形重新光栅化，位置与比例按新/旧排版倍率缩放。这就是
「放大到满屏仍清晰」而预览位置依然与导出一致的原因：清晰度换的是字形光栅化的分辨率，
不是坐标。

光栅尺寸由 `render::subtitle::pipeline::raster_size` 决定，三条约束都不显然：

1. **倍率档化**（1/8 档，向上取整）。按**倍率**而不是按绝对像素量化，两轴共用同一倍率，
   因此不会把画面比例量化歪；向上取整保证永不生成比屏幕矩形小的纹理。倍率下限 1/4，
   极小时不给 libass 退化尺寸；覆盖层还没报尺寸（首帧布局前）时回落到 1:1 排版尺寸。
2. **上限**取设备 `max_texture_dimension_2d` 与 16MP 像素预算中更严的一侧。预算触发时
   会掉到 1/8 档以下，即主动放弃 1:1 清晰度换取不超显存——只有极端放大才会走到这里。
3. **.slint 读不到窗口 scale factor**（本仓库无任何读取点），覆盖层只能报**逻辑**尺寸，
   Rust 侧乘 `window().scale_factor()` 才是物理像素。

字号随光栅尺寸一起放大，所以脚本里的 `Fontsize` 仍按排版尺寸取值——不要为了"看起来更大"
去改脚本，那会让预览与导出分叉。

图集（glyph bitmap 暂存纹理）随光栅尺寸**按需增长**：光栅放大后单行字形块的宽或高就可能
超出初值 2048×1024，越界的是 `copy_buffer_to_texture` 而不是画面。宽度必须始终是
`wgpu::COPY_BYTES_PER_ROW_ALIGNMENT`（256）的整数倍，暂存行距才等于图集宽度。

换过尺寸后必须**强制合成一次**：空帧→空帧时 libass 的 `has_changed` 为 false，
只按 `changed` 闸门会在改完尺寸后继续显示旧尺寸的上一帧。换脚本也是同一个道理，
所以 `SubtitlePipeline::set_script` 只置一个待应用标记，标记生效的那一帧同样算强制合成。

**换脚本不换 libass 渲染器。** 编脚本的投影挂在用户操作节奏的 tag 上，拖动一行字幕的
时间会产生一串 motion 事件，每个事件都要重编整份脚本。libass 的渲染器 `ASS_Renderer`
持有字形缓存，`ASS_Library` 持有字体提供器与已加载字体，重建一次就要重做一遍字体匹配、
并把这一屏所有字形重新光栅化。这个绑定本来就把两者分开了：`Track` 存着自己的脚本字节，
在字段被改动后自动重新解析（`aweft-libass/src/track.rs` 的 `rebuild`），而
`render_frame` 是逐调用接收轨道的（`aweft-libass/src/renderer.rs`）。所以轨道可以随时换，
渲染器不该换——`AssScriptRenderer::load_script` 就只做前者。两条测试钉住这个行为：
`aweft-libass` 的 `load_script_replaces_the_track_but_keeps_the_renderer` 断言换脚本后
`Library` 指针不变、轨道指针变了、并且只有新脚本里的事件时刻出图；
`aweft-gui` 的 `a_new_script_reaches_the_next_frame` 断言换脚本一定落到下一帧。

## 视口里选中与拖动一行字幕

视口要能点中一行字幕、按住把它拖走，前提是有一份「每行一个框」的几何。libass 不给这个：
它的渲染结果只有一张扁平的字形块列表（`aweft-ass::render::Tile`），块上没有「属于哪一行」。
`render/subtitle/hits.rs` 把它还原成 `LineBox`，依据两条都已核实的事实：块按事件连续接进
列表（libass 先按 `(Layer, ReadOrder)` 排序，再逐事件尾部拼接），且同层内参与碰撞避让的行
互不交叠（`fix_collisions` 整块挤开，多行 `\N` 也当一个整体）。于是「按矩形相交把块并成
不相交的簇」得到的簇序就是脚本里 `Dialogue` 的同序，逐簇配上 `aweft_convert::LineHit` 即得
每行的身份。

两条事实都有反例，都由 `LineHit::anchor`（也就是 `\pos`）兜住：带 `\pos` 或 `\move` 的事件
在 libass 里直接置 `detect_collisions = 0`，用户把两行拖到重叠时两簇会并成一簇；某行完全拖到
画面外时根本不产出块，也少一簇。这两种情况下簇数与行数不再相等，配对改为「每簇取锚点最近且
尚未被占用的行」——锚点全相同（没拖过的行都钉在底部中点）时它退化成按序配，正好等于常态。
阴影块（`\3cXy` 那个投影）带着固定偏移，算进去会让框朝阴影方向虚出一截，聚类时直接跳过。

**单位是排版像素，不是光栅像素。** 块坐标跟着 libass 的渲染帧走（上一节的光栅尺寸），而
视口世界坐标、`SetPosition` 写进链的值、Slint 画框的坐标都是排版像素，所以 `line_boxes`
出口按两轴各除一次倍率（`raster_size` 逐边取整，倍率严格说只近似相等，因此两轴分开换）。
选框在 `.slint` 里只乘 `nav-zoom` 就能对上画面，因为视口世界原点就是主视频矩形的左上角，
而后者与脚本的排版帧原点同一（`ui/areas/viewport/view.slint` 的 `main-rect-x`）——不需要
再换算平移，拖动增量也不需要换算。

**位置的唯一真相是链，不是行上的字段。** 落点写进自动生成的一对链（`store/subtitle_chains.rs`）：
视口样式链（`样式源 → 设位置 → 组输出`）供出样式，视口应用链（`上下文 → 文本全等（本行身份）
→ 应用 → 组输出`）把它只贴到那一行。这样节点图里看到的就是拖动本身，不存在「只有视口知道」的
旁路状态。应用链的匹配键是行的 `ItemId` 而不是行序号——序号会在插入/删除/隐藏后移位，绑序号会
把这条链贴到别的行上。

**找回这对链靠结构，不靠名字也不靠额外标记**：应用链里 `expected == 行身份` 的那个 `文本全等`，
它的输出口门控着一个 `应用` 的条件输入口，那个 `应用` 的样式口来自一个 `样式链引用`，被引用的
样式链里第一个 `设位置` 就是要改的端口。用户给两条链改名、往里面再加节点都不影响找回
（`renaming_the_chains_does_not_lose_the_binding`）。

**只在松手时提交，提交的是位移量。** 移动过程中不产生任何模型写操作：`LineDragSession`
（`store/subtitle.rs`）只记两件事——按下时这一行已经压在 libass 算出的位置上的偏移 `pending`
（上一次提交还没落进脚本时非零），和本次按下的位移 `drag`。投影 `ui_sync_subtitle_boxes` 把
`pending + drag` 只加到被拖那一行的框上，画面于是跟着手走。松手时 `line-released` 派发
`SubtitleAction::ShiftLinePosition`，把 `drag` **加**到视口样式链那个 `设位置` 端口的现有值上
（端口原本没有值就从 0 起算），reducer 打的仍是节点图那两种 tag（端口值变化，必要时加链列表
变化），脚本重编照投影原路走，这里不另开通路。提交位移而不是提交落点：样式里的位置是**基线**，
动画行的基线已经折进 `\move` 的每个端点，落一个绝对坐标等于把按下那一瞬动画恰好走到的偏移
烤进基线里（详见 [animation-editor.md](animation-editor.md)）。脚本重编落在后台拟合上，所以会话
多带一个 `committed` 位：偏移一直画到新脚本真正落地的那一帧才清（`fit_in_flight` 与 `fit_pending`
都已落下时清除），清早了框会在旧脚本上闪回原位。同一行在落地前被再次按住时，新会话继承上一次的
`pending + drag`，而不是从 libass 的旧位置重新起算，框因此不会跳。提交失败时两个标记都不会置位，
会话在下一帧被丢弃、框回到原位，这条路径自身会纠偏。

**命中查的是画出来的那份模型，不查 libass。** `SubtitleState.boxes`
（`Rc<VecModel<SlintSubtitleBox>>`）既是 Slint 画框的数据源，也是命中判定扫的那一份列表，
两者按下标严格一致——包括一次提交后偏移仍在生效的那几帧。「倒序找第一个包含该点的矩形」在
Rust 的 `hit_subtitle_line` 里做：列表后面那些框画在前面那些之上，倒序即先命中最上面那条，
与重叠行的视觉上下关系一致。视口只问两件事——`line-picked`（这一点要不要选中一行字幕）、
`line-drag-started`（这一点要不要开始拖一行字幕，返回 `bool`）——命中判定留在 Rust 这一侧：
唯一还能拒的原因是这一点没落在任何框上（位置改成基线相加之后，每行都拖得动），拒的时候留一条
debug，免得「按下去没反应」在现场无从解释。视口拿到 `false` 才照旧去试移动非主视频。框本身不放
`TouchArea`，按下事件穿到上面的 `CanvasNavigation`；字幕排在画面之上，所以按住字幕时字幕优先。

**这份模型逐帧重算，所以出口只写变了的行。** 它走的是开头那条实时例外通路
（`before_rendering` → `render_subtitle_frame`，不经 reducer/depsgraph），每帧一份新几何。
`VecModel::set_row_data` 无条件发 `row_changed`，通知引发重绘，重绘又回到 `before_rendering`
——不先 diff 就把重绘循环自己转起来了。`ui_sync_subtitle_boxes` 因此先比旧值，只在值不同或
行新增/消失时写；行数不变且逐个全等时一个通知都不发。

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
  时间线画布仅以该活跃视频的彩虹色加深后半透明覆盖其时长范围之外的区域；不叠加
  其它视频的范围。媒体总时长在管线发布后写入同一时间条行，未发布时不显示遮罩。
  暂停态从 UI seek 到视频范围外时，动画时间轴保留用户请求值，视频会话与画面钳制
  到最近的可显示帧并由 preroll 刷新画面；越过结尾时从媒体尾边界执行反向精确
  segment seek，以首个 preroll 取得真正的最后一帧，避免正向 seek 在最后一帧 PTS
  之后直接进入 EOS。该路径固定先暂停；再次播放固定先建立正向 segment 再进入
  Playing，避免状态转换竞态导致不刷新或短暂倒放。播放态则始终由当前焦点视频自身的实际
  播放位置接管并传播回时间轴 UI，不受其它视频是否播放影响。
- **播放/暂停**：时间线 Header 中区的播放/暂停按钮是全局 transport；
  `VideoAction::Play/Pause/TogglePlayback` 作用于视口选择集合，无选择时只控制主视频，
  各目标从自身当前位置继续。`VideoSession::playing` 缓存 transport 播放意图，EOS 由 Bus
  写入原子缓存并触发一次播放按钮校正；逐帧渲染路径不查询管线 state/EOS，duration
  只在会话缓存尚未建立时随新帧尝试补齐，建立后不再查询。
  暂停态 seek 不临时切到 Playing，由 preroll 回调交付预览帧。范围外末帧 seek 不等待
  管线状态转换，并对连续相同尾端请求去重，避免拖动时在 UI 线程反复提交 segment。

实时播放时钟是 reducer/depsgraph 之外的明确例外：新视频帧到达时只查询实际收到该帧的
会话位置，并只同步对应时间条及 `AnimationState.current-time`；动作名、范围、通道和选择等
动画标量不随帧重复写入 Slint。波形专用的实际时间/范围外逻辑绑定位于条件实例化的
`AudioWaveformArea` 内，普通 Timeline、Viewport、Graph Area 不订阅这些逐帧属性。

## 音频波形

音频波形使用独立解码管线，不消费播放会话的 appsink。所有媒体共享一个后台
worker，任务串行解码，避免追加多个视频时同时全速占用 CPU/解码器。每个媒体项
持有取消令牌；替换媒体和 teardown 会取消对应任务，旧任务不能覆盖新模型。
波形管线禁止使用 `playbin`：它会把选流、playsink 和状态机封装在一起，不适合作为
可插入音频处理的基础。当前管线显式分层为 `urisourcebin → parsebin → elementary
stream-id 选择 → decodebin → queue → audioconvert → audioresample → capsfilter →
appsink`；其中 `queue` 后的 raw-audio 链是未来插入滤镜、分析器或效果器的稳定边界。
未选择的 elementary stream 接入异步 `fakesink`，确保解复用可以继续到 EOS，但不会启动
不必要的视频或其它音轨解码器。Discoverer 与 parsebin 共用 stream-id，禁止用容器内
序号或 playbin 的 current-audio 作为跨阶段选流依据。
每个 AudioWaveform Area 可独立选择要查看的视频音轨；在波形标尺定位非焦点音轨时
只更新该视频的时间条和画面，不改变全局动画时间。若所选音轨属于当前焦点视频，
同一次定位还会更新动画域当前时间及所有 Area 的全局播放头；暂停态保留用户请求的
时间，播放态继续由该焦点视频的实际管线位置接管。波形 Area 的逻辑播放头允许位于
媒体范围之外；非焦点音轨的范围外请求时间由该 Area 本地保存。视频画面仍钳制到
最近的有效帧，并以该视频的彩色竖线同时标出实际画面时间。

波形以固定 1ms 时间桶累计 min/max/rms 三值（Audacity 式包络），全文件保留作耐久层；横向主标尺与波形共享 1/2/5×10^k 的 nice 时间粒度，但波形密度独立于标尺：各 Area 按“每像素秒数”选桶屏宽 ≥ 1px 的最细列宽，经 `AudioCallbacks.waveform-view` 上报可视窗口（列宽、首列、列数）并取回持久列模型（`store::audio::ui_sync_waveform_columns` 直接聚合基础桶，不预建全文件金字塔；min/max 取极值，rms 取均方根，结果精确对应覆盖区间；数量变化走局部增删、行值走原地更新，槽位组件不重建），因此正常缩放下每像素约一列竖线（min→max 外线加显示 rms 的内层线条），极限放大时线加宽铺满桶区间；不同 Area 按 `slotmap` 键各自登记视图求值，互不干扰，关闭的 Area 由 pump 顺带裁剪。appsink 连续解码并将约四秒音频对应的基础桶合并为一个 UI 批次；批次经 Rust 无界通道发往 UI 线程的 pump 任务（`store::audio::ui_pump_waveform_batches`）写模型，不经过 Slint 回调，worker 不等待 Area 的可见组件完成求值，因此切换 Area 不会暂停后台解码。解码进度（`waveform.duration` 随批次变）驱动 Slint 列绑定重算，新批次到达即刷新可见列。UI 保留持久的
`VecModel<SlintWaveformBucket>` 基础桶，通过 `set_row_data`/`push` 原地更新，不按批次克隆完整桶数组或
重建嵌套模型。

渲染器 teardown 会停止全部管线并释放 sink 的 GPU 资源。
