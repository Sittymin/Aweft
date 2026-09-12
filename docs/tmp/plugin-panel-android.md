# Android 上插件面板的嵌入路线（进度记录）

> 本文件记录 2026-09-12 这一轮对「Android 上怎么打开插件运行时 UI」的核实结果，
> 供上下文丢失后接手。文中「pin rev」指本工程 pin 的
> `5e097e2c267d7752bfe1bf27f4e9a094e00e335d`。当日核对完成后，
> `/home/Sittymin/Development/sources/slint` 已就地 checkout 到该 rev，
> 所以全文所有 slint 行号现在都能在这份检出里按行号直接打开
> （本轮已逐条复核 `typeregister.rs:626-627`、`api.rs:1196`、
> `component_container.rs:91-121`、`langtype.rs:214-221`、
> `android-activity/lib.rs:106`、`typeloader.rs:961`、`lib.rs:276`、
> `builtins.slint:487`、LSP 四处先例）。「pin rev」的标注保留，
> 用于说明该行号当初是在哪个版本上读到的。
> 未验证的条目在标题里写明「未验证」，不要当结论引用。
> 相关既有记录：`docs/tmp/android-port.md`、
> `docs/tmp/plugin-project-store.md` 的「装载路线」段。

## 结论（第一句先答所问）

Android 上插件面板必须嵌进编译期 Slint 树（`ComponentContainer` 元素 +
`ComponentFactory`），开第二个窗口这条路不通。嵌入本身只差两个开关：给
`slint-interpreter` 开 `internal` 特性，以及编译宿主窗口时开实验类型。本工程 rev
已 pin 死，风险不漂移。第二个开关是本轮新查出的硬门槛：`ComponentContainer` 与
`component-factory` 类型属于「实验类型」，默认的内建类型表里被删掉了，所以**必须在
`aweft-gui/build.rs` 的 `slint_build::compile` 之前设
`SLINT_ENABLE_EXPERIMENTAL_FEATURES=1`**，否则 `ui/app/window.slint` 里写
`ComponentContainer` 会直接报未知类型。

另外插件文件目前在 APK 里根本不存在（§6）。同日定案：**不靠打包解决**——
运行时用户安装的插件不可能编译进软件。改成插件根可配置，Android 默认落
`external_data_path()/plugins`（§8 已实测可写可读），覆盖口放进设置。
形态上按「Area Switch 菜单动态增加插件条目 + 空白宿主 Area 由插件 UI 填」做，
不硬编码任何给插件用的控件，桌面与 Android 统一走嵌入。
前置的不依赖 GUI 的设置层已经落：新成员 `aweft-settings`（`docs/architecture/settings.md`），
未来 CLI 读同一份非 GUI 字段；片 A 同步落完，插件根两项新候选排在开发期兜底之前。
剩下的只有片 B 的嵌入与设置界面本身。

## 1. 桌面现状（实现）

| 对象 | 出处 | 作用 |
|---|---|---|
| 面板实例化 | `aweft-plugins/src/ui.rs:74` | `definition.create()`，走 `WindowOptions::CreateNewWindow` |
| 面板显示 | `aweft-plugins/src/ui.rs:94` | `instance.show()`，开真窗口 |
| 窗口守卫 | `aweft-plugins/src/ui.rs:61-63` | `PluginWindow` 持 `Rc<ComponentInstance>`，丢弃即关窗 |
| 四个约定名 | `aweft-plugins/src/ui.rs:14-17` | `command-labels`、`status-lines`、`command-clicked`、`command-busy` |
| 面板编译 | `aweft-plugins/src/manager.rs:180-196` | `read_panel` + `ui::compile_panel`，失败只丢窗口 |
| 每插件一个窗口 | `aweft-gui/src/plugin_bridge.rs:24-32`、`:237-282` | `windows: HashMap<String, PluginWindow>` |
| 按钮 | `aweft-gui/ui/areas/line_editor/view.slint:142-144` | 按 `PluginState.plugin-titles` 下标发 `open-plugin-panel` |
| 回调注册 | `aweft-gui/src/bridge/callbacks.rs:1409-1420` | `setup_plugin_callbacks` → `open_plugin_window_at` |
| 标题写入 | `aweft-gui/src/store/app_store.rs:56-63` | 装载结果写 `PluginState.plugin-titles` |

## 2. Android 阻断点（实现，pin rev 核实）

- `internal/backends/android-activity/lib.rs:106-108`（pin rev）：
  `create_window_adapter` 恒返回 `Ok(self.window.clone())`。
- 后果：每个 `slint::Window` 都是那块唯一的 NativeActivity 表面。
  `definition.create()` + `.show()` 拿不到第二个窗口，面板只会和主界面抢同一块。

## 3. 嵌入路线：逐项公开性核实（pin rev = `5e097e2c267d7752bfe1bf27f4e9a094e00e335d`）

| 名称 | 出处（pin rev） | 可见性与约束 | 够不够 |
|---|---|---|---|
| `ComponentContainer` | `internal/compiler/builtins.slint:487` 起 | 内建元素，带 `in property <component-factory> component-factory` | 够 |
| `component-factory` 类型 | 同上 | Slint 语言类型 | 够 |
| `FactoryContext` | `internal/core/component_factory.rs:14`（`:16` `parent_item_tree`、`:19` `parent_item_tree_index`） | 结构体与其字段全部 `pub` | 够 |
| `ComponentFactory::new` | `internal/core/component_factory.rs:53`，约束在 `:54-55` | `pub`，要求 `X: HasStaticVTable<ItemTreeVTable>`、`T: ComponentHandle<WeakInner = VWeak<ItemTreeVTable, X>>` | 够 |
| `ComponentFactory` 重导出 | `api/rs/slint/lib.rs:221` | `pub use i_slint_core::component_factory::ComponentFactory;` | 够 |
| `ComponentDefinition::create_embedded` | `internal/interpreter/api.rs:1196` | `#[doc(hidden)]` + `#[cfg(feature = "internal")]`（`:1194-1195`） | 开 `internal` 特性即可用 |
| `create_with_options` | `internal/interpreter/api.rs:1216` | `pub(crate)`，解释器内部实现，调用方不需要碰 | — |
| `internal` 特性声明 | `internal/interpreter/Cargo.toml:130` | `internal = []` | 够 |
| `ComponentInstance` 满足 `ComponentHandle` | `internal/interpreter/api.rs:1688` | `type WeakInner = vtable::VWeak<ItemTreeVTable, crate::instance::Instance>` | 够 |
| `Instance` 满足 `HasStaticVTable` | `internal/interpreter/item_tree_vtable.rs:27` | `i_slint_core::ItemTreeVTable_static!(static INTERPRETER_INSTANCE_VT for Instance);` | 够 |

最后两行是决定性的：`ComponentFactory::new` 的两个 trait 约束，
`ComponentInstance` 与它背后的 `Instance` 都满足。所以

```rust
let factory = ComponentFactory::new(move |ctx: FactoryContext| {
    let instance = definition.create_embedded(ctx).ok()?;
    // 在这里填属性、接回调
    Some(instance)
});
```

这样的类型是成立的。`definition` 是 `aweft-plugins/src/ui.rs:37-43`
`compile_panel` 产出的 `ComponentDefinition`，它是 `Clone`。

## 4. 此前没有记到的硬门槛：`ComponentContainer` 是实验类型

- pin rev `internal/compiler/typeregister.rs:614` `builtin_experimental` 返回全集；
  `:622` `builtin` 在 `:626` 执行 `register.elements.remove("ComponentContainer").unwrap()`、
  在 `:627` 执行 `register.types.remove("component-factory").unwrap()`。
- 两者之间的选择在 pin rev `internal/compiler/typeloader.rs:961`：
  `enable_experimental` 为真时用 `builtin_experimental`，否则用 `builtin`。
- `enable_experimental` 的唯一来源是环境变量，pin rev `internal/compiler/lib.rs:276`：
  `std::env::var_os("SLINT_ENABLE_EXPERIMENTAL_FEATURES").is_some()`，写进字段
  `:172` `pub enable_experimental: bool`。
- 上游先例：`tools/lsp/build.rs:16-18` 在 `slint_build::compile` 之前
  `unsafe { std::env::set_var("SLINT_ENABLE_EXPERIMENTAL_FEATURES", "1") }`，
  注释写明「Make the compiler handle ComponentContainer」。
- 对本工程的后果：`aweft-gui/build.rs:11` 现在是
  `slint_build::compile("ui/app/window.slint").unwrap()`，必须在它之前设该变量。
- 副作用（**未验证**）：pin rev `internal/compiler/lib.rs:442` 有
  `if !diagnostics.enable_experimental && !type_register.expose_internal_types`
  这道门控，开了 experimental 会连带把 `builtin_internal` 里的其它实验元素
  （例如 `FlexboxLayout`）暴露给宿主窗口。需要跑一次宿主编译确认没有别的变化。

## 5. 上游可运行的先例（实现）

- `tools/lsp/preview.rs:1683-1689`：`ComponentFactory::new(move |ctx: FactoryContext| {`
  → `compiled.create_embedded(ctx).unwrap()` → `callback(instance.clone_strong())`
  → `Some(instance)`；随后 `:1643` `api.set_preview_area(factory)`。
- `tools/lsp/Cargo.toml:113`：对 `slint-interpreter` 开
  `["compat-1-2", "internal", "internal-highlight", "internal-json", "image-default-formats"]`。
- `tools/lsp/ui/views/preview-view.slint:318-321`：`preview-area-container := ComponentContainer {`
  配 `component-factory: root.preview-area;`；全局属性在 `tools/lsp/ui/api.slint:344`
  `in-out property <component-factory> preview-area;`。
- 代码生成侧：pin rev `internal/compiler/generator/rust.rs:95` 把
  `Type::ComponentFactory` 映射成 `slint::ComponentFactory`，所以容器的 setter
  收的是 `slint::ComponentFactory`。
- 先例还说明：被嵌入的根组件可以是 `Window` 根的组件（LSP 嵌入的就是整个应用组件）。
  本工程的 `plugins/test-plugin/panel.slint` 是
  `export component PluginPanel inherits Window`，可以原样嵌入；容器内部对
  `WindowItem` 根的子树会自己接管宽高绑定（`internal/core/items/component_container.rs`
  的 `ensure_updated`）。

## 6. Android 上插件文件当前根本不存在（实现）

- 候选目录探测只认文件系统：`aweft-gui/src/plugin_bridge.rs:146-165`
  `plugin_roots()` 取 cwd、cwd 父目录、可执行文件同级目录下的 `plugins`。
- 读取全走 `std::fs`：`aweft-plugins/src/manifest.rs:45-51` `load_dir`、
  `:62-65` `read_guest`、`:68-72` `read_panel`；`load_root` 在
  `aweft-plugins/src/manager.rs:224-248` 用 `std::fs::read_dir`。
- APK 里这些目录都不存在，装载结果为空 → `PluginState.plugin-titles` 为空 →
  行编辑器上连插件按钮都不出现（`aweft-gui/src/store/app_store.rs:56-63`）。
- 需要装载的载荷（仓库内现状）：
  `plugins/test-plugin/aweft-plugin.json`、`guest.wasm`、`panel.slint`；
  `plugins/test-plugin/guest/` 是 Guest 源码（184K），其 `target/` 被
  `.gitignore` 的 `/plugins/*/guest/target/` 排除。

## 7. APK assets 的能力（仅记录，插件不走这条路）

**2026-09-12 定案**：插件不作 assets 打包。用户原话「插件不应该是打包进去的吧？
毕竟是运行时用户安装的插件肯定不可能是编译进软件的」。assets 只适合编译期就确定的
载荷，以后字幕字体可以走这条路，本节那几条 xbuild 行号留着备用。

检出在 `/home/Sittymin/Development/sources/xbuild`（分支 `pin-e67b501`，
从 `~/.cargo/git/db/xbuild-7ada1d01b20e4efd` 离线取的该 commit）。
本节行号已在该检出上逐条复核。

- 配置项：`xbuild/src/config.rs:440` `pub assets: Vec<AssetPath>`。
- `AssetPath` 两种写法，`xbuild/src/config.rs:353-362`：直接给 `PathBuf`，
  或 `{ path, optional, alignment }`；默认 alignment 是
  `UnalignedCompressed(Compressed)`（`config.rs:336-340`）。
- 消费点：`xbuild/src/command/build.rs:210-215`，路径以
  `env.cargo().package_root()` 为基准（本工程即 `aweft-android/`），
  `optional` 为真且路径不存在就跳过。
- 打包实现：`apk/src/lib.rs:80-93` `add_asset`，dest = `assets/<所给路径的文件名>`；
  目录走 `xcommon/src/lib.rs:299-307` `add_directory` → `:347` `add_recursive`，
  递归并保留目录结构。
- 与本项无关的门槛：`docs/tmp/android-port.md` 已验证第 9 条说明 GStreamer 全是
  静态库，没有 `.so` 要收集。

## 8. Android 上的目录与用户可写性（真机实测，2026-09-12）

替代原先「方案 (a) 解 assets / 方案 (b) 来源抽象」那条未定案的取舍，两条都作废。
测量环境：小米 nuwa（2210132C）、API 33、MIUI、包 `org.aweft`、`userId=10256`。

- 两个目录访问器，都在 `android-activity-0.6.1/src/lib.rs`，都返回
  `Option<PathBuf>`：`:1272 internal_data_path()` = `/data/data/org.aweft`、
  `:1277 external_data_path()` = `/storage/emulated/0/Android/data/org.aweft/files`。
  `AndroidApp` 由 `api/rs/slint/android.rs:103` 重导出，`:751` 是 `Clone`。
- `internal_data_path` 用户侧写不进去：实测 `adb shell run-as org.aweft id` 报
  `run-as: package not debuggable: org.aweft`，非 root 只有这一条路。排除。
- `external_data_path` 两条都成立：
  - 应用自己读写**不需要任何权限**——实测 `ls -la` 显示 `files` 目录属主 `u0_a256`、
    模式 `drwxrws---`，而 `dumpsys package org.aweft` 给的 `userId=10256` 就是它，
    即属主是本应用。
  - `adb` 侧可写——实测 `mkdir -p .../files/plugins` 返回 0，
    `adb push` 一个 probe 文件进去后 `cat` 读回内容一致（已清理，目录留空）。
- 「用户读写不了」这条只对**第三方文件管理器**成立（Android 11+ 的 `Android/data`
  限制）。该说法在这台机器上**没有实测**（设备上没有装带 `MANAGE_EXTERNAL_STORAGE`
  的文件管理器），只作为已知平台限制记录，不当结论用。
- 依赖：`aweft-gui/Cargo.toml:82` 现在只有 `ndk-context`。走 `AndroidApp` 的
  访问器不需要加 `ndk`；只有要 `AssetManager`（`asset_manager()` 在同文件 `:1030`）
  时才需要，而 assets 这条路已废。

## 9. 改动清单（三片，按顺序做）

架构参照 Blender 的用户偏好体系，取舍依据见本节末「借鉴来源」。
片 0 与片 A（1-13 步）不含任何 UI 代码，已于 2026-09-12 落完；
片 B 从第 14 步的前置验证开始，验不过就走它写明的退路。

### 片 0：新增 workspace 成员 `aweft-settings`（不依赖 GUI）—— 已落（2026-09-12）

1. 落点：与 `aweft-model`/`aweft-project` 平级的新成员，已注册进根
   `Cargo.toml:7-10` 的 members，并且根包 `[dependencies]` 也加了它。
   本工程没有 `[workspace.dependencies]` 表，依赖版本各成员自己写。
   它是唯二不引用 Slint 的下层（`aweft-model/Cargo.toml:6-16` 零内部依赖、
   `aweft-project/Cargo.toml:7,11` 只有 model + serde_json）；不能挂在
   `aweft-plugins` 侧，那里已经直接依赖 `slint-interpreter`
   （`aweft-plugins/Cargo.toml:19`）。
   三个模块：`src/lib.rs`（类型 + 版本戳 + 进程内那一份）、
   `src/location.rs`（目录怎么定）、`src/storage.rs`（读写）。
2. 形状：落成 `Settings { format: u32, paths: Paths }`，`Paths` 里只有
   `plugin_root: Option<PathBuf>`。
   **计划里的 `gui` 段与 `plugins` 映射都没有建空壳**：空段会让人以为可以往里
   随便塞，等真有字段时再开。分区规矩写进了 crate 文档注释与
   `docs/architecture/settings.md`——只给界面用的字段将来进 `gui` 段、
   插件偏好将来进 `plugins` 段（键为插件 id、值为不透明 JSON，宿主 schema
   不为插件增长，这条学 Blender 的 `bAddon.prop`，`DNA_userdef_types.h:586`）。
   `format` 版本戳的做法照 `aweft-project/src/document.rs:5`
   `AWEFT_DOCUMENT_FORMAT` 的既有先例。
3. 默认值就是 `Default`，「恢复出厂」= 跳过读盘，不做第二套值。
4. 迁移**没有做成 `fn migrate(&mut Settings, from: u32)`**：实际是一个版本门
   （`storage.rs:30-37`，读到的 `format` 比本程序支持的新就 `bail!` 点名两个版本
   与文件路径）+ 容器级 `#[serde(default)]`（加字段不 bump 也能读旧文件），
   补写旧值的位置在 `storage.rs:38-41` 的注释里标出。不做 Blender 那套按字段名
   重建（SDNA，`dna_genfile.cc:706/1246`），因为设置文件是我们自己产的。
5. 写盘用 temp + rename 原子写（`storage.rs:46-78`）。**这条没有先例可抄**：
   `aweft-project/src/store.rs:344-350` 是 `to_string_pretty` + `fs::write` 直接覆盖，
   没有 .bak 也没有原子写，而且 `save_to_file` 在生产里零调用点
   （只有 `store.rs:677,679,693` 的测试用）。设置文件损坏会挡启动，所以这里必须自带。
   两处必需细节都留了注释：`sync_all` 必须早于 `rename`（否则掉电留 0 字节文件）、
   临时文件名带进程号（命令行与界面可能同时写）。
6. 没有撤销栈：只有 `is_dirty` 标记、显式保存、revert = 再 `load()` 一次。
   对照 Blender 核实过：它的偏好面板同样没有撤销，`WM_OT_read_userpref`
   （`wm_files.cc:2773-2778`）就是从盘重读，`UserDef U_backup = dna::shallow_copy(U)`
   （`:2724`）只为算差集发通知。
7. 读设置走进程内那一份：`aweft_settings::settings()`（`lib.rs`）首次访问按当时的
   配置目录读盘并缓存，`Settings::load_default` 是它的私有实现。
   **时序约束**：入口必须"先 `set_config_dir`，后任何一次 `settings()`"，
   否则缓存的是错目录的结果——桌面在 `src/main.rs:8-11` 进 `main` 第一件事，
   Android 在 `aweft-android/src/lib.rs:31-39` 于 `slint::android::init(app)`
   （`:44`）移走 `app` 之前用 `app.external_data_path()`
   （android-activity 0.6.1 `src/lib.rs:1277`）。
8. 配置目录优先级落成**两级**：`AWEFT_USER_CONFIG` 显式覆盖 → 平台目录
   （`location.rs:23-28`），计划里的第三级"内置回退"没有做——定不出目录时
   `config_dir()` 返回 `None`，读设置退回出厂值并留警告，不猜一个可能写不进去的路径。
   平台目录这一项**确实算在本成员里**（`platform_config_dir()`，
   桌面 `XDG_CONFIG_HOME` → `$HOME/.config`，再接 `aweft`；相对或空的 XDG 按未设置处理），
   与初版计划"本成员不算平台路径"不同：Android 侧仍然必须由入口交，
   该函数在 Android 目标上返回 `None`（`location.rs:51-54`）。
   覆盖口是必需的，因为 `plugin_root` 是设置文件里的字段，而该文件的位置不能由它自己定。
   **这个环境变量是给用户看的配置入口，不是测试开关**：测试直接给
   `storage::load(dir)` 传临时目录。
9. 文档：`docs/architecture/settings.md`（新增，记上面的分层与界线），
   新成员已补进 `docs/architecture/file-layout.md`（`## aweft-settings/src/` 一节）
   与 `docs/architecture/overview.md` 的 crate 列表。
   验证：`nix develop --command cargo test -p aweft-settings` 11 条全过，
   `cargo check --workspace` 与 `cargo check -p aweft-android --target
   aarch64-linux-android` 均为 0 退出码、新成员零告警。

### 片 A：插件根改为可配置，Android 默认落外部数据目录 —— 已落（2026-09-12）

10. `aweft-gui/src/plugin_bridge.rs:146-175` `plugin_roots()` 的候选列表前面插了两项：
    `aweft_settings::settings().paths.plugin_root`、`config_dir()/plugins`。
    第二项在 Android 上就是 `external_data_path()/plugins`
    （`/storage/emulated/0/Android/data/<包>/files/plugins`），在桌面上是
    `~/.config/aweft/plugins`。原有 cwd / cwd 父目录 / exe 同级三项保留作开发期兜底，
    不存在的目录照旧由消费点 `:43-51` 的 `is_dir()` 跳过。
    `aweft-gui/Cargo.toml:31` 加了 `aweft-settings` 依赖，
    `aweft-android/Cargo.toml:14` 同理。
11. `aweft-plugins` 的装载路径一行不动：`std::fs` 那三处
    （`manifest.rs:45-51`、`:62-65`、`:68-72`）与 `manager.rs:224-248` 的
    `load_root` 全部照旧，因为它们吃的就是目录路径。
12. 装机验证通道：`adb push <插件目录> /storage/emulated/0/Android/data/org.aweft/files/plugins/`
    （§8 已实测该路径可写可读）。
13. `plugin_root` 现在只能靠手工写 `<配置目录>/aweft-settings.json` 来设
    （`save_to_dir` 在产品代码里还没有调用点）；设置 UI **不在本片做**，
    等片 B 的 4-5 步把 Area 菜单改成模型之后，"设置"只是菜单里多一条数据。
    **本片的目录与候选顺序尚未在真机上跑过**——要等下一次 `x build` 装机才能验到，
    验收点是日志里那条 `从 … 装载了 N 个插件` 打的是哪个目录。

### 片 B：插件 UI 嵌进编译期 Area

14. 前置验证（先编一次再往下写）：`component-factory` 能不能当**结构体字段**。
    类型门控这一关是过的——`internal/compiler/langtype.rs:214-242` 的
    `is_property_type()` 列了 `Self::ComponentFactory`，同一列表还含
    `Struct{..}` 与 `Array(_)`；但 `grep -rn component-factory tests/cases/`
    全是 property 声明，Slint 自己的语料里没有 struct 字段用例
    （最接近的是 `tests/cases/elements/component_container_component.slint:18`
    `in property<component-factory> c1;`）。
    **退路**：`ScrAreaData` 只带 `plugin-id: string`，factory 由 Area 按 id 现取。
15. `aweft-gui/ui/core/area_type_metadata.slint`：`items`（`:5-15`）从编译期字面量数组
    改成宿主模型属性，值由 Rust 算（9 个内置项 + 每个有面板的插件一项）；
    两条硬编码 0..8 下标链（`:18-40`、`:43-65`）整个删掉。
    消费点是 `ui/widgets/area_type_switcher.slint:33` 与 `:71`，
    唯一写点是 `ui/areas/area_header.slint:38-46` →
    `AreaCallbacks.switch-area-type`（`ui/bridge/callbacks.slint:25-27`）。
16. `aweft-gui/ui/core/types.slint`：`SlintAreaType`（`:8-25`）加 case `plugin`；
    `ScrAreaData`（`:85-93`）加 `plugin-id` 与 `plugin-factory`。
    Rust 侧 `aweft-gui/src/screen/area.rs:47` 的 `AreaType` 加带值变体，
    `aweft-gui/src/bridge/types.rs:9-37` 两支转换各补一支。
    Area 本身已是模型驱动的（`ui/app/window.slint:84` `for scr-area in scr-areas`），
    不需要新机制。
17. `aweft-gui/ui/screen/area_container.slint`：在 `:49-91` 标题栏组加一支
    `if area-type == SlintAreaType.plugin: AreaHeader {...}`，在 `:93-175` 内容组加一支
    `ComponentContainer { component-factory: root.plugin-factory; }`。
    不预留任何供插件使用的控件。
18. `aweft-gui/build.rs`：在 `:11` 的 `slint_build::compile` 之前设
    `SLINT_ENABLE_EXPERIMENTAL_FEATURES=1`。不开的话
    `internal/compiler/typeregister.rs:626-627` 已把 `ComponentContainer` 与
    `component-factory` 从内建表里 remove 掉，宿主 `.slint` 写它就是未知类型。
    选择点在 `internal/compiler/typeloader.rs:961`，变量读取在
    `internal/compiler/lib.rs:276`。
19. `aweft-plugins/Cargo.toml:19`：给 `slint-interpreter` 加 `features = ["internal"]`
    （特性声明在 `internal/interpreter/Cargo.toml:130`；`i-slint-core` 在 `:23`
    是同 rev，不用动）。只为这一个开关：`create_embedded` 在
    `internal/interpreter/api.rs:1194-1196` 是 `#[doc(hidden)] #[cfg(feature="internal")]`，
    它背后的 `create_with_options`（`:1216`）是 `pub(crate)`。
20. `aweft-plugins/src/ui.rs`：把开窗那条换成嵌入——不再 `definition.create()`（`:74`）
    + `instance.show()`（`:94`），改为 `ComponentFactory::new` 闭包内
    `create_embedded(ctx)`，四项约定（`:14-17`）的属性写入与回调接线在闭包里做完再返回。
    两个 trait 约束已核实满足：`api.rs:1688`、`item_tree_vtable.rs:27`
    对得上 `internal/core/component_factory.rs:53-55`。
21. 语义后果，三条都要处理：
    - 闭包只能现取现填。`if area-type ==` 是条件元素，来回切 Area 会销毁重建容器
      连带插件实例（`area_container.slint:183` 的 `changed area-type` 就在应对这事），
      捕获快照会让切回来的 Area 显示旧状态。
    - 同一插件可在两个 Area 同时打开，`aweft-gui/src/plugin_bridge.rs:24-32` 那个
      以 plugin_id 为键的 `HashMap<String, PluginWindow>` 要改成按 Area 持实例，
      同步时对同插件所有实例扇出。
    - 桌面也走嵌入，开窗分支整体删除（Android 上开第二窗本就断，
      `internal/backends/android-activity/lib.rs:106-108` 恒返回同一个 window）。
      删掉的就是 `ui.rs:61-63` 的窗口守卫与 `ui/areas/line_editor/view.slint:140-146`
      那排插件按钮（入口由 Area 菜单承担）。
    - 插件根组件是 `Window` 也能原样嵌：`internal/core/items/component_container.rs:91-121`
      检测到根是 `WindowItem` 时单向绑定容器宽高。`plugins/test-plugin/panel.slint:8`
      的 `PluginPanel inherits Window` 不用改，其中 `title`/`min-width`/`min-height`
      变成无意义。换 factory 即换内容：同文件 `:55-127` 的 `ensure_updated`
      用 `component_tracker` 跟踪 `component_factory`。

### 借鉴来源（Blender `v5.2.1`，rev `9e2066aef`，本地检出 `sources/blender`）

- 抄的：字段初始化器即出厂默认 + factory 模式跳过读盘
  （`creator_args.cc:3348`、`wm_files.cc:1325-1361`）；版本戳 + 单条顺序迁移函数
  （`versioning_userdef.cc:817`，且迁移里禁止碰全局偏好，`:67` 的 static assert）；
  目录发现三级优先且平台实现抽到接口后（`appdir.cc:496/510/517`、
  `BKE_appdir_folder_id_ex:649` 的 `BLENDER_USER_CONFIG:695`、
  `GHOST_SystemPathsUnix.cc:99-109`）；无撤销栈、revert = 重读、
  dirty + 退出按开关自动存（`wm_files.cc:2773-2778`、`wm_init_exit.cc:527`）；
  插件偏好走类型无关的袋子、宿主结构不为插件增长
  （`DNA_userdef_types.h:1040` 的 addons、`:586` 的 `bAddon.prop`）。
- 不抄的：设置塞进 blend 二进制容器；1200 行平铺结构 + 手写 `_pad`；
  为同一结构在 RNA 里维护 11 个视图类导致字段说明双写
  （`rna_userdef.cc:5078/6136/7299/7799` 全 `RNA_def_struct_sdna(..., "UserDef")`）；
  版本号进目录名再用 Python `copytree` 整目录继承（`userpref.py:98/172`）。

## 10. 未验证项与下一步命令

未验证两条半，动手时按顺序收：

1. §4 的副作用：开 `SLINT_ENABLE_EXPERIMENTAL_FEATURES` 会连带把
   `builtin_internal` 里的其它实验元素暴露给宿主窗口（门控在
   `internal/compiler/lib.rs:442`）。跑一次宿主编译即可确认没有别的变化。
2. §9 第 14 步：`component-factory` 当结构体字段能不能编过。类型门控过了
   （`langtype.rs:214-242`），但 Slint 语料里没有用例。失败就走退路：
   `ScrAreaData` 只带 `plugin-id`。
3. （半条）§9 片 A 的候选顺序没有在真机上跑过：下一次装机看日志
   `从 … 装载了 N 个插件` 打的是哪个目录。原来这里的"原子写没有先例"一条已收：
   `aweft-settings/src/storage.rs:46-78` 落成 temp + `sync_all` + rename，
   由 `round_trip_keeps_plugin_root` 与 `save_leaves_no_temp_file` 两条测试覆盖。

2026-09-12 跑过并全绿的：`cargo test -p aweft-settings`（11 条）、
`cargo check --workspace`、`cargo check -p aweft-android --target aarch64-linux-android`。
**没有跑过 `x build`，也没有装机**，所以 Android 侧只有编译面结论。

```sh
# 设置层
nix develop --command cargo test -p aweft-settings
# 桌面回归（确认开 experimental 没有破坏宿主窗口编译）
nix develop --command cargo check -p aweft-gui
# Android 编译面
nix develop --command bash -c 'cargo check -p aweft-gui --target aarch64-linux-android'
# 插件与界面的测试
nix develop --command cargo test -p aweft-plugins -p aweft-gui
# 出包与装机：见 docs/tmp/android-port.md 的「验证命令备忘」
```

## 11. 环境事实

- slint 源码只有一份在场：`/home/Sittymin/Development/sources/slint`，
  2026-09-12 已就地 checkout 到本工程 pin 的 rev
  `5e097e2c267d7752bfe1bf27f4e9a094e00e335d`（切之前是 1.17.1 `cf62c975c`）。
  `sources/` 是随手可弃的阅读目录，所以 **pin 变了就在这份检出里直接
  `git checkout <新 rev>`**，不要再引 cargo 缓存，也不要另建 worktree 或第二份检出。
  核对方式：与 `~/.cargo/git/checkouts/slint-774a37e469fae6bf/5e097e2` 抽样 6 个文件
  md5 逐字节相同。
- 本工程的 pin 点：`aweft-plugins/Cargo.toml:19`（`slint-interpreter`）、`:23`
  （`i-slint-core`）；`aweft-gui/Cargo.toml` 的桌面段与 Android 段各自 pin 同一 rev。
- Android 后端渲染器是 Skia（`docs/tmp/android-port.md` 已验证第 2 条），
  与嵌入路线无冲突：`ComponentContainer` 走的是 item tree，不依赖具体渲染器。
