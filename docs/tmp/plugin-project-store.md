# 临时文档：aweft-project + aweft-plugins（分支 feat/plugin-project-store）

> 目标：项目库与插件运行时都按"能长期演进的初版"对待，设计更好就大片重构。
> 端到端功能：字幕行时间被改时插件收到"哪一行变成了什么"；插件命令写回同一份真相；
> 每插件一个自带 Slint 片段的独立窗口。
> 2026-09-10 决定：只保留独立窗口，砍掉主程序内嵌的列渲染。
> 2026-09-11 决定：示例插件改用 Rust SDK + 提交预编译 `guest.wasm`；GUI 的数据真相改为持有 `ProjectStore`。

## 三个问题的结论

**Q1「`aweft-gui/build.rs` 里为什么有插件相关部分？」——不该有，已删。**
build.rs 现在只做 `slint_build::compile("ui/app/window.slint")`。插件在运行时装载：
`PluginBridge::load()` 在 `AppStore::new` 里扫描插件根目录，`wat` 构建期依赖与
`OUT_DIR` 内嵌字节码全部撤掉。为了让示例插件在测试与运行里都可用，改为提交
预编译夹具 `plugins/test-plugin/guest.wasm`（`plugins/*/guest/target/` 已 gitignore）。

**Q2「为什么 GUI 里有 `plugin_notify_time_changed`，不该是 project 通知插件吗？」——方向已反转。**
真相是 `ProjectStore`：每次已建模的写入在 `pending` 里留一条 `MutationReceipt`（写日志），
持有者在一轮交互结束时 `take_pending()` 分发。GUI 只在 `AppStore::dispatch` 末尾调一次
`pump_plugins()` → `PluginManager::deliver_pending(store)`，把日志投给订阅的 Guest；
Guest 写回的是同一个存储，因此继续投递直到日志为空（上限 `MAX_DELIVERY_ROUNDS = 8`，
超限只告警）。GUI 侧不再"应用"插件结果，只把回执映射成刷新 `Tag`。
写入瞬间不做同步回调：插件在收到通知后会继续写同一个存储，同步回调等于在可变借用期间再写一次。
删除的东西：`plugin_notify_time_changed`、`apply_plugin_receipts`、manager 内的工程数据镜像
（`sync_mirror`/`notify_on_project`/`invoke_on_project`）、按 tag 反推"哪些行变了"的扫描。

**Q3「`test-guest.wat` 真的是用户要写的东西吗？」——不是，已删。**
示例/测试插件现在是普通 Rust crate（`plugins/test-plugin/guest`）依赖 `aweft-plugin-sdk`，
`build.sh` 用 `cargo build --target wasm32-unknown-unknown` 产出 `guest.wasm`。
Python 路线查过并否决：rustpython-wasm 依赖 `getrandom`/JS 侧运行时，宿主是纯 wasmi
而没有 JS 引擎，且 CPython 级 Guest 的体积与启动成本对一个"改两行时间"的示例是本末倒置。

## 现在的形态

### 目录与装载
```
plugins/<id>/
  aweft-plugin.json   # 身份 + 构件位置 + 权限申请（不声明功能）
  guest.wasm          # 逻辑（提交预编译夹具）
  panel.slint         # 自带窗口片段（单根组件）
```
装载根按存在与否探测：`$PWD/plugins` → `$PWD/../plugins` → 可执行文件同级 `plugins`，
覆盖"仓库根运行""crate 目录跑测试""打包安装"三种布局。无 env 开关。
`load_root` 只扫一层子目录并排序（投递顺序稳定），单个插件失败只记日志；
面板缺失或编译失败只丢窗口，命令与订阅照常工作。

### Guest ABI v1
导出：`aweft_describe_ptr`/`aweft_describe_len`（描述 JSON
`{"abi":1,"commands":["计算最早时间","全部后移 1s"],"events":["time_changed"]}`）、
`aweft_on_command(i32) -> i32`、可选 `aweft_on_event_time(i32, i64, i64)`。
导入 `aweft` 模块：`event_count`、`event_start_ms`、`event_end_ms`、`set_event_start_ms`、
`set_event_end_ms`、`set_status_line(line,ptr,len)`、`log_text(ptr,len)`。
数字句柄 = `ProjectStore::ordered_event_ids()` 的下标（按开始时间排序，每轮投递前重建）。
**显示文本一律由插件自己组装**：旧的 `command_result_kind`/`command_format` 模板机制删除，
宿主不做格式化与模板替换。命令编号未声明即拒绝执行。

### 面板通用契约（宿主与片段之间只有三项）
`in property <[string]> command-labels`、`in property <[string]> status-lines`、
`callback command-clicked(int)`。片段自己决定这些值渲染成什么控件。
解释器组件无法嵌进编译期 Slint 树（`create_embedded` 是 internal 特性，`FactoryContext`
外部造不出；复查 master 9933161 结论不变），因此只能独立开窗；Overlay 盖窗方案亦已否决
（Wayland 下应用无法自定位窗口）。

### 真相侧（aweft-project）
`ProjectStore` 是文档唯一所有者，时间写入的单一入口：`set_event_start_ms`、
`set_event_end_ms`、`set_event_times_ms`（成对写，先结束后开始以避开钳制截断）、
`set_event_time_fuzzy`、`adjust_event_time`（含钳制）、`shift_event_times`
（整体平移，不钳制，允许开始时间为负）。`record_time_write` 比对写入前后的成对值，
**钳制后值未变就不记回执**，插件不会为一次无效写入收到通知。
来源标记 `Origin` 用于回声过滤：`Origin::Plugin(id)` 的写入不再回投给该插件自己。

### GUI 侧
`AppState.project` 现在是 `ProjectStore`（不再是裸 `Project`）。
`SequenceState` 的全部方法参数由 `&Project` 收窄为 `&Sequence`——行编辑器投影只依赖字幕结构。
reducer 的时间写入统一经存储并标 `Origin::LocalGui`（`adjust_time` 为唯一增减入口，
`InputTimecode` 走 `set_event_time_fuzzy`，`TimelineBlockDragCommit` 走 `shift_event_times`）；
预览与取消不写 Domain。行编辑器工具条的插件按钮由 `PluginState.plugin-titles` 重复生成
（启动期写一次），`open-plugin-panel(index)` 按下标路由，主程序不硬编码任何插件名。

## 要干什么（已完成打钩）

- [x] `aweft-project`：文档结构 + 唯一所有权存储 + 写日志回执 + 工程文件读写（JSON，`format` 版本门）。
- [x] `aweft-plugins`：清单解析 + 目录装载 + wasmi 运行时 + 自描述 ABI + 写日志投递 + 通用面板窗口。
- [x] `aweft-plugin-sdk`：Guest 侧 `plugin! { commands, on_time_changed }` 声明宏与 `Api` 封装。
- [x] 示例插件 `plugins/test-plugin`（Rust Guest + panel.slint + 清单，`guest.wasm` 提交为夹具）。
- [x] GUI 接线：持有 `ProjectStore`、`dispatch` 末尾泵写日志、命令路由、按装载结果生成按钮。
- [x] 集成测试：`aweft-plugins/tests/plugin_flow.rs`（7 项，真实装载 `plugins/test-plugin`）、
      `aweft-gui/tests/plugin_window.rs`（6 项）。
- [ ] CLI 子命令（本轮砍掉：CLI 尚无工程文件开/存流程，无落点）。
- [ ] 后续（不在本轮）：结构/层级/文本写入尚无 `MutationKind`（因此不进写日志，插件看不到）；
      撤销栈策略；`dirty`/保存提示接 UI；远端合流（`aweft-collab`）；多语言 SDK（C/Python）再评估。

## 本轮删除的死代码（可点名否决）

`PluginManager`：`CommandResult`/`StatusTarget`/`PluginCommand`/`NotifyRecord`/`CommandOutcome`、
`format_ms`/`format_outcome`、工程数据镜像与 `sync_mirror`、`visible_panels`/`set_hidden`/`last_notify`；
`ui.rs`：`PanelHooks`/`PanelGuard`/`open_panel` 与内嵌的 `TEST_PANEL_SOURCE`；
`aweft-plugins/assets/`（wat/清单/面板三份测试资产）；`aweft-gui`：`apply_plugin_receipts`、
`run_plugin_command_text`、`plugin_notify_time_changed`、`Option<PluginBridge>` 降级分支、
`ProjectStore::shift_all_events_ms`（SDK 用成对写入即可，无调用方）、`receipt::Subscriber`。

## 验证结果（2026-09-11）

- `nix develop --command cargo test`：`aweft-project` 8、`aweft-plugins` 1+7、`aweft-plugin-sdk` 2、
  `aweft-gui` 197（lib）+ 依赖图/行编辑器/时间线/NLA/DopeSheet/节点图/插件窗口共 60 项集成，全绿。
- `cargo check --workspace --all-targets`：本轮改动的三个 crate 零告警
  （`aweft-ass`、`src/cli.rs` 的告警是既有噪声）。clippy 仍被 `aweft-libass` 的构建挡住，故用 check+test 验证。
- 真机点按未验证：插件按钮与独立窗口需要跑起来确认（无头后端只证明不 panic）。

## 教训

- 勿跑任何整树格式化命令（`cargo fmt -- <files>` 与按文件列表的 `rustfmt` 都曾引发全工作区
  churn，两次共还原 40+ 无关文件；仓库本身非当前工具链 rustfmt 默认风格）。之后只手写格式。
- `slint::ComponentHandle` 提供 `global()`；`bridge/types.rs` 用 `include_modules!()`，
  因此新 global 必须在 `ui/app/window.slint` 里 import + 重新导出，Rust 侧才看得见。
- `slint-interpreter` 不重导出 `ModelRc`/`VecModel`，需要额外依赖同 rev 的 `i-slint-core`；
  `ModelRc<T>` 没有 `From<Vec<T>>`，用 `ModelRc::from(slice)` 或 `ModelRc::new(VecModel::from(..))`。
- wasmi 2.0 API 以本地克隆（`sources/wasmi`）为准：`Linker::func_wrap` + `instantiate_and_start`
  + `get_typed_func` + `get_memory`/`Memory::read`；`Caller::get_export("memory")` 取宿主函数里的内存。
- 投递接口不要写成 `deliver(&mut store, store.take_pending())`——同一次表达式里两次可变借用，
  编译不过且是个脚枪；改成 `deliver_pending(&mut store)` 由被调方自己排空。
