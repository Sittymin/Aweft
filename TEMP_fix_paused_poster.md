# 临时文档：导入即暂停视频的首帧（海报）黑屏 + 参考框错位修复

> 状态：未提交、未 Review。本文档用于 Review 与回滚讨论。
> 分支工作树本来就有一大批未提交改动（多视频会话重构等），本次只涉及其中 3 处靶点。
> 已知副作用：**播放性能劣化**（见末尾「待查」一节），需 Review 时一并定夺。

---

## 1. 问题

- 视频「导入（尚未播放，处于 PAUSED 态）」时，画面是黑的，没有用第一帧作为海报。
- 因为第一帧没取到，`video-widths/heights` 也就没有分辨率 → 主视频的参考延伸框尺寸错乱（表现为右边/下边距离不对）。
- 之前尝试过若干方案（开播放改暂停、渲染通知器里 request_redraw、新_preroll 回调、事件循环定时器持续重绘）都没解决，属于**间歇性/顽固黑屏**。

## 2. 根因（真正的原因）

`SlintWGPUSink::fetch_next_frame`（`aweft-gui/src/render/video_sink/sink.rs:192`）原来在**暂停态**先调用 `try_pull_sample`，再调用 `try_pull_preroll`：

```rust
let sample = if let Some(s) = self.appsink.try_pull_sample(gstreamer::ClockTime::ZERO) { s }
else if self.pending_refresh { self.appsink.try_pull_preroll(ZERO)? }
else { self.appsink.try_pull_preroll(ZERO)? };
```

GStreamer 的 **pull-mode appsink** 里，preroll（PAUSED 时的首帧）和 sample 共用同一份内部缓冲；
**只要先 pull 过一次 sample，preroll 缓冲就被一并消费掉**，随后 `try_pull_preroll` 永远返回 `None`。
于是暂停态的首帧/seek 预览帧永远取不到 → 黑屏 + 无分辨率。

### 复现（写成了回归测试 `test_paused_preroll_with_callbacks`）

我加了一个对照实验测试，完整复刻 App 路径：`connect()`（带 `new_preroll/new_sample` 回调 + bus 同步处理器丢弃消息 + 构建 wgpu 转换管线）→ `set_state(Paused)` → 反复调用 `fetch_next_frame()`。
- 旧顺序（`try_pull_sample` 在前）：`frames_pulled=0`，`try_pull_sample=false` 且 `try_pull_preroll=false`，**永远取不到**。
- 单独只调 `try_pull_preroll`（不经过 `fetch_next_frame`）：能取到 → 证明管线/preroll 本身没问题，锅在 `fetch_next_frame` 的拉取顺序。
- 改成 preroll 优先后：第二次迭代 `preroll=true`，测试通过。

## 3. 修复 1：fetch_next_frame 预滚动优先

`aweft-gui/src/render/video_sink/sink.rs:206` 起：

```rust
// 关键：pull 模式下若先调用 try_pull_sample，会把 preroll 一并消费掉，
// 导致随后的 try_pull_preroll 永远取不到首帧/seek 预览帧（黑屏）。故暂停态
// 必须预滚动优先；播放态普通 sample 优先。两者都尝试以保证不丢帧。
let sample = if self.pending_refresh {
    self.appsink
        .try_pull_preroll(gstreamer::ClockTime::ZERO)
        .or_else(|| self.appsink.try_pull_sample(gstreamer::ClockTime::ZERO))?
} else {
    self.appsink
        .try_pull_sample(gstreamer::ClockTime::ZERO)
        .or_else(|| self.appsink.try_pull_preroll(gstreamer::ClockTime::ZERO))?
};
```

要点：
- `pending_refresh == true`（暂停态：开场首帧 / 拖拽预览）走 **preroll 优先**。
- 播放态走 **sample 优先**（与原来行为一致），preroll 作为兜底（此时 preroll 早已被取走，返回 None 无副作用）。

## 4. 修复 2：主视频归位到世界原点（参考框错位）

`aweft-gui/src/reducers/media.rs:300` `cascade_anchor`：

```rust
fn cascade_anchor(store: &AppStore) -> (f64, f64) {
    let state = store.state();
    let anchor = state.selected_video.or(Some(state.main_video))
        .and_then(|i| state.video_sessions.get(i));
    // 首个（主）视频固定在原点，不应用级联偏移；偏移仅用于错开叠加的后续视频。
    // 否则主视频会偏离参考框中心（右下错位）。
    let Some(anchor) = anchor else {
        return (0.0, 0.0);
    };
    let (ax, ay) = (anchor.position_x, anchor.position_y);
    let height = anchor.wgpu_sink.last_video_size.map(|(_, h)| h).unwrap_or(1080) as f64;
    let offset = (height * 0.08).clamp(32.0, 200.0);
    (ax + offset, ay + offset)
}
```

原来对「第一个（主）视频」也施加了 `height*0.08` 的级联偏移 `(86.4, 86.4)`，但 UI 里的 `main-rect`/参考框是以世界原点 `(0,0)` 为中心绘制的，于是主视频被推到右下，参考框左右/上下不对称。
现改为：首个视频返回 `(0,0)`，与 `view.slint` 里「主视频固定在原点」的注释一致；级联偏移仍只用于后续叠加视频。

## 5. 配套机制（三者一起才让「暂停态首帧」可靠显示）

仅修复 2 的拉取顺序还不够——暂停态没有 `new_sample` 回调驱动渲染循环，渲染循环不跑就不会调 `fetch_next_frame`。
因此保留/新增了以下驱动：

- **`pending_refresh` 标志**（`sink.rs:61`）：打开视频/暂停态 seek 后置 `true`，取到一帧后清零。
  - 设置在 `reducers/media.rs:209`（append）、`:250`（replace）、`:435`（seek）。
- **`kick_refresh_until_settled` 定时器**（`callbacks.rs:524`）：用 `slint::Timer`（事件循环线程，可靠）每 30ms `request_redraw()`，直到没有任何会话 `pending_refresh` 才自停。从 `on_open_video`（`callbacks.rs:96`）与 `on_time_bar_dragged` 触发。
- **`new_preroll` 回调**（`sink.rs:171`）：appsink 的 preroll 通知也请求重绘，作为补充入口。注意 `on_frame_arrived` 类型因此改成 `Arc<dyn Fn() + Send + Sync>`（`AppSinkCallbacks` 要求 `Sync`）。
- **`handle_before_rendering` 的 `need_more`**（`callbacks.rs:586`）：渲染通知器里若仍有会话 `pending_refresh`，派发 `request_redraw` 再保险。

> 说明：渲染通知器跑在**渲染线程**，`request_redraw` 直接调不可靠，所以统一走事件循环（`upgrade_in_event_loop` / `slint::Timer`）。

## 6. 播放性能劣化（待查，未定位）

你提到播放性能劣化了。本人在这三处修改里**没有做明显的逐帧重活**，但以下点需要 Review/取样确认，不能排除是我的改动引入：

1. **`kick_refresh_until_settled` 是否在播放期误持续**：逻辑上 `pending_refresh` 取到首帧即清，播放态应为 `false`，定时器应停。但若有会话 `pending_refresh` 一直不清除（例如刷新被 `refresh_attempts>300` 兜底清零后、又有新会话追加），定时器会持续 30ms 请求重绘 → 空转。需确认 playing 时 `pending_refresh` 确实为 `false`、定时器已停。
2. **每渲染帧都 `fetch_next_frame` 全量遍历**（`handle_before_rendering`）：playing 时 `try_pull_sample` 每帧调用（本身廉价），但仍有 `borrow_mut` + 遍历所有会话的开销。
3. **背压设计 `max_buffers(3)` + `drop(false)`**（`sink.rs:78`）：原注释说这是为了避免「导入后播几帧就停」，但 playing 时解码被节流到渲染拉取速率——若渲染拉取跟不上会卡顿。这比本次修复更早存在，但值得作为性能嫌疑点一并看。
4. **与本次 3 处无关的整体回归**：工作树里还有大批未提交改动（多视频会话重构等），性能劣化可能来自那批，而非本次修复。建议用 `perf`/火焰图对 `handle_before_rendering` + `fetch_next_frame` 取样，定位到底是哪个改动。

建议 Review 时：先确认「播放态 `pending_refresh` 是否确实清零、定时器是否停止」，这是最便宜也最可能踩中的点。

## 7. 测试

新增回归测试 `aweft-gui/src/render/video_sink/tests.rs::test_paused_preroll_with_callbacks`：
完整复刻 `connect()` + `Paused` + `fetch_next_frame`，断言能取回首帧且携带分辨率变化。

`cargo test -p aweft-gui` 全绿（lib 176 + 集成测试均通过）。

## 8. 涉及文件与行号速查

| 文件 | 位置 | 内容 |
|---|---|---|
| `aweft-gui/src/render/video_sink/sink.rs` | `fetch_next_frame` :192，修复 :206-217 | 预滚动优先拉取 |
| `aweft-gui/src/render/video_sink/sink.rs` | `:61` `:93` `:244` | `pending_refresh` 字段/初始化/清零 |
| `aweft-gui/src/render/video_sink/sink.rs` | `:171` | `new_preroll` 回调 |
| `aweft-gui/src/reducers/media.rs` | `cascade_anchor` :300，修复 :306-310 | 主视频归位原点 |
| `aweft-gui/src/reducers/media.rs` | `:209` `:250` `:435` | 打开/替换/seek 置 `pending_refresh` |
| `aweft-gui/src/bridge/callbacks.rs` | `kick_refresh_until_settled` :524 | 事件循环定时器驱动重绘 |
| `aweft-gui/src/bridge/callbacks.rs` | `need_more` :586 | 渲染通知器内再保险重绘 |
| `aweft-gui/src/bridge/callbacks.rs` | `:96` `:173` | 定时器触发点（打开视频 / 拖拽时间条） |
| `aweft-gui/src/render/video_sink/tests.rs` | `test_paused_preroll_with_callbacks` | 回归测试 |

## 9. Review 待确认清单

- [ ] preroll 优先的拉取顺序是否还有其他调用点需要同步：`reducers/media.rs:656`（`pump_appsink`）、`:1093`（seek 死锁测试 drain）也有 `try_pull_sample` 调用，但二者都是**测试用的排空/放背压辅助函数**，不是生产 `fetch_next_frame` 路径；生产路径只有 `sink.rs::fetch_next_frame` 一处，已修复。确认这两处测试辅助不影响结论即可。
- [ ] 播放态 `pending_refresh` 是否真正清零（性能关键）。
- [ ] `refresh_attempts > 300` 兜底清零：是否会让某会话「彻底放弃」首帧而黑屏（极端情况）。
- [ ] 性能劣化根因定位（见第 6 节）。
- [ ] 是否要回滚 `kick_refresh_until_settled` 改用更轻量的触发（仅在打开/seek 时 request_redraw 一次 + 依赖 new_preroll/new_sample 回调）。
