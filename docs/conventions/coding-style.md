# 编码规范

**局部更新**：`set_row_data` > `set_vec` > `clear`；`rebuild_all` 仅初始化/重置用

**清空 VecModel**：用 `clear()`，不用 `set_vec(Vec::new())`

**Slint ↔ Rust**：直接用 Slint 生成枚举，不做 `i32` 中转（本次会话已全局替换动画编辑器部分）

**错误处理**：`log::warn!` + 提前 return

**Rust 函数命名与分层**：函数名必须能够直接表明所属层和职责，避免在不同层使用含义模糊的通用名称。媒体 reducer 的会话编排使用 `media_*` 前缀；GStreamer 管线、状态、seek、EOS、时长和音频操作使用 `gst_*` 前缀；渲染回调及帧驱动同步使用 `render_*` 前缀；Slint/UI 模型、标量和时间条同步使用 `ui_*` 前缀。跨模块调用也必须保留该层级前缀，测试和注释中的函数引用同步更新。

示例：`media_apply_video_playback`（媒体 reducer 编排）、`gst_seek_pipeline_accurate`（GStreamer 操作）、`render_sync_video_playback_time`（渲染回调）、`ui_sync_video_time_bars`（UI 模型同步）。

**分支风格**：优先 **卫语句（guard）+ 提前返回** 减少嵌套与认知负担；`if` 嵌套超过 2 层或 `else` 分支仅为 `return`/`continue` 时必须改为 `if !cond { return }`；与已有 `store/animation/graph/marquee.rs`、`store/sequence.rs`、`store/animation/nla/` 的早期返回风格保持一致

**函数长度**：像 `strip_dragged` 这类长函数尽量拆分为通用小函数（非强制，但超过 50 行或含多级嵌套时应拆分）；通用逻辑抽至 `store/utils/{marquee,selection,drag}.rs` 单一文件夹，供曲线/摄影表/时间线/NLA 复用，避免重复实现框选与拖动

**通用逻辑**：通用函数集中于 `store/utils/` 单一文件夹（`marquee`/`selection`/`drag`），新增功能前先在此查找是否已有通用函数（如 `normalize_time`/`clamp_rows`/`is_replacing`/`is_valid_delta`），避免重复造轮子；跨编辑器复用逻辑不得散落在各 `store/animation/*/marquee.rs` 等局部文件中

**回调命名**：`kebab-case`（`keyframe-pressed`）

**导入**：跨编辑器共享 → `core/types.slint`；编辑器专用 → `core/animation_types.slint`

**克隆审慎**：`clone` 大数据（`editing_keyframes`/`editing_strips`/`VecModel` 等）必须经过深思熟虑、确认无增量/最终校验等更轻量方案后才使用；优先增量回退（`prev_delta`）或最终偏移提交校验（同字幕行 `DragTimecodePreview/Commit` 与 Area 拆分合并）

**注释必须有可测量的目标**：注释只允许陈述**有东西守着的事实**——类型或编译器强制的不变式、
测试名点明的行为、能数出来的操作次数或复杂度。凡「代价随 X 走」「驻留上限是 Y」「这种情况不会
发生」「这两处必须一起改」这类主张，都要同时写清是谁守住它（`tests::xxx`、`#[must_use]`、泛型
约束、计数断言）；守不住就删掉那句，或把它降级成 `TODO(未实测)`，**不许自说自话**。理由不是
修辞：新写的代码旁边最容易长出一段听起来合理但没人验证过的解释，解释越顺越危险。
性能类主张同样受这条约束：提交信息里写了「更快/更省」却没实测的，必须显式标注「按复杂度推的，
未实测」。

**写文档**：只写全局验证过的内容，不猜测；不写会话日志（会膨胀）；保持简短
