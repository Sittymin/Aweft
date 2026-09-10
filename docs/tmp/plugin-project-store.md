# 临时文档：aweft-project + aweft-plugins（分支 feat/plugin-project-store）

> 目标：项目库与插件运行时都按"能长期演进的初版"对待，设计更好就大片重构。
> 端到端功能：字幕行时间被改时插件收到"哪一行变成了什么"；插件命令写回同一份真相；
> 每插件一个自带 Slint 片段的独立窗口。
> 2026-09-10 决定：只保留独立窗口，砍掉主程序内嵌的列渲染。
> 2026-09-11 决定：示例插件改用 Rust SDK + 提交预编译 `guest.wasm`；GUI 的数据真相改为持有 `ProjectStore`。
> 2026-09-11 决定（同日二改）：变更分发从"持有者泵写日志"下沉进 `ProjectStore`——写入即广播，
> 界面与插件一样注册成订阅者，谁都不再记得要 pump。

## 三个问题的结论

**Q1「`aweft-gui/build.rs` 里为什么有插件相关部分？」——不该有，已删。**
build.rs 现在只做 `slint_build::compile("ui/app/window.slint")`。插件在运行时装载：
`PluginBridge::load(store)` 在 `AppStore::new` 里扫描插件根目录（顺带把声明了订阅的插件注册成
存储的订阅者），`wat` 构建期依赖与 `OUT_DIR` 内嵌字节码全部撤掉。为了让示例插件在测试与运行里都可用，改为提交
预编译夹具 `plugins/test-plugin/guest.wasm`（`plugins/*/guest/target/` 已 gitignore）。

**Q2「为什么 GUI 里有 `plugin_notify_time_changed`，不该是 project 通知插件吗？」——方向已反转，且分发落在了存储里。**
`ProjectStore` 既持真相也持订阅者注册表：已建模的写入在 `record()` 里落成 `MutationReceipt`
后立刻广播（**写入即广播**），使用方不需要记得 pump。规则全在存储一处——
批量作用域（`begin_batch`/`end_batch` + `BatchToken`）把一次交互的多行写入合成一次通知；
广播期间 `hold` 非零，嵌套写入只排队，回到栈顶后按轮继续投；
轮数上限 `MAX_BROADCAST_ROUNDS = 8`，超限丢弃剩余回执并告警；
按 `Origin` 做回声过滤，写入者听不到自己那次写入。
订阅者是 `Rc<dyn MutationSubscriber>`（`&self` + 内部可变），广播时注册表被 `mem::take` 出来，
因此回调能拿到 `&mut ProjectStore` 写回。GUI 与插件平级：`store::project_watch` 把界面登记成
`Origin::LocalGui` 的订阅者，回调只把回执排进队列（广播发生在 `AppStore` 已被可变借用的中途，
此刻不能回头借同一个 `RefCell<AppStore>`），`dispatch` 收尾时取走队列换成刷新 `Tag`。
第一版的"持有者在一轮交互结束时 `take_pending()` 泵一次"已作废：那样每个接入方都要自己记得 pump，
插件写回的行还得再绕一轮才刷得出来。
删除的东西：`plugin_notify_time_changed`、`apply_plugin_receipts`、manager 内的工程数据镜像
（`sync_mirror`/`notify_on_project`/`invoke_on_project`）、按 tag 反推"哪些行变了"的扫描、
`ProjectStore::take_pending`、`PluginManager::{deliver_pending, deliver_time_changed, handle_map}`
与 `MAX_DELIVERY_ROUNDS`、GUI 的 `pump_plugins`/`receipt_tags`。

**Q3「`test-guest.wat` 真的是用户要写的东西吗？」——不是，已删。**
示例/测试插件现在是普通 Rust crate（`plugins/test-plugin/guest`）依赖 `aweft-plugin-sdk`，
`build.sh` 用 `cargo build --target wasm32-unknown-unknown` 产出 `guest.wasm`。
Python 路线本轮不做，准确的理由分两层：官方那条 wasm 产物路线走 `wasm-bindgen`，`.wasm` 的
import 指向 JS 侧胶水，纯 wasmi 宿主（没有 JS 引擎）在实例化阶段就满足不了这些 import；
可行的替代是自己做一个内嵌 RustPython 的 cdylib 壳、导出本 ABI、编到 `wasm32-wasip1`，
宿主侧再挂 wasmi 自带的 WASI preview1 支持（`wasmi_wasi`）——这条路未实测，
且每个 Python 插件要背上解释器体积与 VM 启动成本。示例插件的职责是把 ABI 讲清楚，故选 Rust SDK。

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
数字句柄 = `ProjectStore::ordered_event_ids()` 的下标（按开始时间排序，每次进入 Guest 前重建快照）。
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
分发也在这一侧：`subscribe(SubscriberRef)` 注册 `Rc<dyn MutationSubscriber>`，
`record()` 落成回执后立刻 `broadcast()`。广播把注册表 `mem::take` 出来再按轮投，
所以回调签名是 `on_mutations(&self, store: &mut ProjectStore, &[MutationReceipt])`；
`hold` 计数让广播期间与批量作用域内的嵌套写入只排队。来源标记 `Origin` 用于回声过滤：
`Origin::Plugin(id)` 的写入不再回投给该插件自己，`Origin::LocalGui` 的写入不回流界面刷新队列。

### 插件侧（aweft-plugins）
`PluginManager::load_dir(dir, store)` 读清单 → 装载 Guest → 校验描述声明的导出 →
编译面板 → 声明了 `time_changed` 的插件注册成存储订阅者。`GuestPlugin` 是成员内部类型
（`pub(crate)`），对外只有 `PluginManager`/`LoadedPlugin`/面板与 ABI 常量。
Guest 的读写一律先 `sync_from_store` 建行快照、调用结束后 `take_dirty_rows` 写回，
**对 `Rc<RefCell<GuestPlugin>>` 的借用绝不跨过对存储的写入**：写入会同步广播，
而广播可能绕回本插件形成嵌套借用（`RefCell` panic）。命令执行整条包在批量作用域里，
一条命令对其它订阅者就是一次通知。

### GUI 侧
`AppState.project` 现在是 `ProjectStore`（不再是裸 `Project`）。
`SequenceState` 的全部方法参数由 `&Project` 收窄为 `&Sequence`——行编辑器投影只依赖字幕结构。
reducer 的时间写入统一经存储并标 `Origin::LocalGui`（`adjust_time` 为唯一增减入口，
`InputTimecode` 走 `set_event_time_fuzzy`，`TimelineBlockDragCommit` 走 `shift_event_times`）；
预览与取消不写 Domain。行编辑器工具条的插件按钮由 `PluginState.plugin-titles` 重复生成
（启动期写一次），`open-plugin-panel(index)` 按下标路由，主程序不硬编码任何插件名。
`store::project_watch` 是界面自己的订阅者：回调只把回执排进 `RefreshQueue`，
`AppStore::dispatch` 用 `begin_batch`/`end_batch` 包住一轮交互、随后 `apply_refresh_queue()`
把队列换成刷新 tag 并回填插件窗口状态行。

## 要干什么（已完成打钩）

- [x] `aweft-project`：文档结构 + 唯一所有权存储 + 写日志回执 + 订阅者注册表与广播 + 工程文件读写（JSON，`format` 版本门）。
- [x] `aweft-plugins`：清单解析 + 目录装载 + wasmi 运行时 + 自描述 ABI + 装载时注册订阅者 + 通用面板窗口。
- [x] `aweft-plugin-sdk`：Guest 侧 `plugin! { commands, on_time_changed }` 声明宏与 `Api` 封装。
- [x] 示例插件 `plugins/test-plugin`（Rust Guest + panel.slint + 清单，`guest.wasm` 提交为夹具）。
- [x] GUI 接线：持有 `ProjectStore`、界面自己注册成订阅者、`dispatch` 用批量作用域包住一轮交互、
      命令路由、按装载结果生成按钮。
- [x] 集成测试：`aweft-project` 存储内测试 11 项（广播时机、回声过滤、批量合并、写回触达第三方、
      轮数上限）、`aweft-plugins/tests/plugin_flow.rs` 8 项（真实装载 `plugins/test-plugin`，
      含"一次命令只通知一次"）、`aweft-gui/tests/plugin_window.rs` 6 项
      （含"插件写入刷进 UI 行模型"的投影断言）。
- [ ] CLI 子命令（本轮砍掉：CLI 尚无工程文件开/存流程，无落点）。
- [ ] 后续（不在本轮）：结构/层级/文本写入尚无 `MutationKind`（因此不进广播，插件与界面都看不到）；
      撤销栈策略；`dirty`/保存提示接 UI；远端合流（`aweft-collab`）；多语言 SDK（C/Python）再评估。

## 本轮删除的死代码（可点名否决）

`PluginManager`：`CommandResult`/`StatusTarget`/`PluginCommand`/`NotifyRecord`/`CommandOutcome`、
`format_ms`/`format_outcome`、工程数据镜像与 `sync_mirror`、`visible_panels`/`set_hidden`/`last_notify`、
`deliver_pending`/`deliver_time_changed`/`handle_map`/`MAX_DELIVERY_ROUNDS`；
`GuestPlugin`：`id`/`origin`/`invoke_command`/`notify_time_changed`/`flush_to_store`（快照 + 写回拆开后可由
`sync_from_store`/`call_*`/`take_dirty_rows` 组合出来）；`ProjectStore::take_pending`（队列只由广播排空）；
`ui.rs`：`PanelHooks`/`PanelGuard`/`open_panel` 与内嵌的 `TEST_PANEL_SOURCE`；
`aweft-plugins/assets/`（wat/清单/面板三份测试资产）；`aweft-gui`：`apply_plugin_receipts`、
`run_plugin_command_text`、`plugin_notify_time_changed`、`Option<PluginBridge>` 降级分支、
`ProjectStore::shift_all_events_ms`（SDK 用成对写入即可，无调用方）、`receipt::Subscriber`、
`pump_plugins`、`receipt_tags`。

## 验证结果（2026-09-11，广播下沉后复测）

- `nix develop --command cargo test -p aweft-project -p aweft-plugins -p aweft-gui`：
  `aweft-project` 11、`aweft-plugins` 1+8、`aweft-plugin-sdk` 2、
  `aweft-gui` 197（lib）+ 依赖图/行编辑器/时间线/NLA/DopeSheet/节点图/插件窗口共 60 项集成，全绿。
- `cargo check --workspace --all-targets`：本轮改动的三个 crate 零告警
  （`aweft-ass`、`src/cli.rs` 的告警是既有噪声）。clippy 仍被 `aweft-libass` 的构建挡住，故用 check+test 验证。
- 真机点按未验证：插件按钮与独立窗口需要跑起来确认（无头后端只证明不 panic）。
  `plugin_window.rs` 的"插件写入刷进 UI 行模型"断言覆盖到了投影这一层，但仍是无头路径。

## 教训

- 存储写入即广播 ⇒ **任何跨过写入的外部借用都会撞嵌套借用**。插件侧固定写法：
  借出 Guest → 只做纯 Guest 调用 → `take_dirty_rows` → 释放借用 → 才写 store；
  界面侧则根本不在回调里动 `depsgraph`，只排进 `RefreshQueue`，回合末由 `AppStore` 自己收。
  广播里 `mem::take` 注册表是同一件事的另一面：回调要拿 `&mut ProjectStore`，注册表不能同时被借用。
- `begin_batch`/`end_batch` 用令牌而非 RAII（`dispatch` 期间要以 `&mut self` 调子分发，守卫借不住），
  于是**出错路径必须自己收口**：`end_batch` 放在 `?` 之前，配成一个内层函数 + 外层收口。
  漏收口的表现是 `hold` 永不归零、之后所有写入静默不再通知——不会有编译错误或 panic。
- 勿跑任何整树格式化命令（`cargo fmt -- <files>` 与按文件列表的 `rustfmt` 都曾引发全工作区
  churn，两次共还原 40+ 无关文件；仓库本身非当前工具链 rustfmt 默认风格）。之后只手写格式；
  本轮实测：先 `rustfmt --check` 逐文件看 diff 是否只落在自己改过的行，再只对那几个文件跑，
  并用 `git diff --stat` 复核——`reducers/sequence.rs` 因既有漂移（非本次改动行）被跳过。
- `slint::ComponentHandle` 提供 `global()`；`bridge/types.rs` 用 `include_modules!()`，
  因此新 global 必须在 `ui/app/window.slint` 里 import + 重新导出，Rust 侧才看得见。
- `slint-interpreter` 不重导出 `ModelRc`/`VecModel`，需要额外依赖同 rev 的 `i-slint-core`；
  `ModelRc<T>` 没有 `From<Vec<T>>`，用 `ModelRc::from(slice)` 或 `ModelRc::new(VecModel::from(..))`。
- wasmi 2.0 API 以本地克隆（`sources/wasmi`）为准：`Linker::func_wrap` + `instantiate_and_start`
  + `get_typed_func` + `get_memory`/`Memory::read`；`Caller::get_export("memory")` 取宿主函数里的内存。
- 分发接口不要写成 `deliver(&mut store, store.take_pending())`——同一次表达式里两次可变借用，
  编译不过且是个脚枪。中途曾改成"由被调方自己排空"的 `deliver_pending(&mut store)`，
  最终结论是这一步不该存在：队列与注册表都不外露，存储自己在 `record()` 里广播。
