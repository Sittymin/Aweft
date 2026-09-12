# 应用设置（`aweft-settings`）

## 定位与依赖方向

下层成员，与 `aweft-model` / `aweft-project` 平级，是本工程唯二不引用 Slint 的成员之一。
它不依赖任何其它成员；反方向上 `aweft-gui`、根包的命令行入口、`aweft-android` 都依赖它。
以后做无头 CLI 时读的是同一份类型，不需要经过界面。

三个文件各管一段：

```
aweft-settings/src/
├── lib.rs       # Settings 类型、格式版本戳、进程内那一份（settings()）
├── location.rs  # 配置目录怎么定：显式覆盖 → 平台目录
└── storage.rs   # 读写落盘：原子写、版本门
```

## 字段分区

`Settings { format, paths }`，`paths.plugin_root` 是当前唯一字段。分区规矩：

- **只给界面用的字段将来单独开一个 `gui` 段**，CLI 只读非 `gui` 段。现在还没有这样的字段，
  所以没有空段——空段会让人以为可以往里塞任何东西。
- **插件自己的偏好将来开 `plugins` 段**（键为插件 id，值为不透明 JSON）。这样宿主的结构体
  永远不会因为某个插件加了个开关而改 schema，这条抄的是 Blender 的 addon 偏好（`bAddon.prop`
  是个不透明 IDProperty 袋）。
- 平台目录、插件根这类跨界面与命令行的字段留在顶层或非 `gui` 段里。

## 版本与迁移

`format` 是文件自身的格式版本，做法沿用 `aweft-project/src/document.rs` 的
`AWEFT_DOCUMENT_FORMAT` 先例。规则：

- **加字段不 bump**：`Settings` 上挂了容器级 `#[serde(default)]`，旧文件缺的字段自动取默认值。
- **只有"旧值必须改写才有意义"的改动才 bump**，并在 `storage::load` 里标明的位置按版本顺序
  补一步。不做 Blender 那套按字段名重建（SDNA），因为设置文件是我们的，不是第三方产的。
- 读到的 `format` 比本程序支持的新时**直接拒绝**（错误里点名两个版本号与文件路径），
  不猜测也不改写——否则旧程序会把新程序的设置悄悄抹平。

## 配置目录

优先级只有两级：**显式覆盖 → 调用方交进来的平台目录**。

- 显式覆盖是环境变量 `AWEFT_USER_CONFIG`。**它是给用户看的配置入口，不是测试开关**：
  设置文件自己的位置不能由设置文件里的字段决定，所以这一层必须在进程外留一个口；
  测试靠直接调 `storage::load(dir)` 传临时目录，不借这个变量。它只覆盖目录，不覆盖单个设置项。
- 平台目录由 `platform_config_dir()` 算：桌面走 `XDG_CONFIG_HOME`（规范要求自己定，
  所以相对路径与空值都按未设置处理）→ 回退 `$HOME/.config`，末尾接 `aweft`；
  **Android 上没有可猜的平台目录**，返回 `None`。
- 因此目录必须由入口在启动早期提交：桌面 `src/main.rs` 进 `main` 第一件事，
  Android `aweft-android/src/lib.rs` 在 `slint::android::init(app)` 移走 `app` 之前用
  `app.external_data_path()`（android-activity 0.6.1 `src/lib.rs:1277`，
  实测为 `/storage/emulated/0/Android/data/<包>/files`，应用自己可写，`adb push` 通）。

**时序约束（改动时先看这条）**：`settings()` 首次访问就按当时的配置目录读盘并缓存整个进程，
之后再 `set_config_dir` 不再生效。所有入口都必须"先定目录，后读设置"。

## 读盘失败怎么办

设置文件坏了不能挡住启动：目录未定、读不出、解析失败，一律退回出厂值并留一条 `warn`。
文件不存在按出厂默认处理（首次启动就是这个状态）。唯一的硬失败是上面那条版本门。

## 写盘

`storage::save` 是 temp + rename 原子写：写同目录临时文件 → `write_all` → `sync_all` →
`rename`，失败路径上删掉临时文件。两处细节都是必需的：

- **`sync_all` 必须在 `rename` 之前**：没刷盘的临时文件被 rename 上去之后，进程被杀或掉电
  会留下一个 0 字节的设置文件，下次启动就读不到设置了。
- **临时文件名带进程号**：命令行与界面可能同时在写同一份设置。

这条**没有仓内先例可抄**：`aweft-project/src/store.rs` 的 `save_to_file` 是
`to_string_pretty` + `fs::write` 直接覆盖，没有 .bak 也没有原子写，且在产品代码里零调用点。
设置文件损坏会直接影响启动，所以这里必须自带。

`save_to_dir` 目前在产品代码里还没有调用点——第一个写入口是设置界面（见
`docs/tmp/plugin-panel-android.md` §9 片 B 之后），在那之前手工编辑
`<配置目录>/aweft-settings.json` 是唯一的写法。

## 没有撤销

设置改动不做撤销栈。只有三件事：脏标记、显式保存、放弃改动＝重新读一次盘。
对照 Blender：它的偏好面板同样没有撤销（`WM_OT_read_userpref` 就是从盘重读），
`UserDef U_backup = dna::shallow_copy(U)` 只为算差集发通知，不用于回退；
Blender 真正的内存撤销是文档撤销（`MemFileUndoStep`，受 `U.undosteps` / `U.undomemory`
限制，超出即释放最老的步骤），跟设置是两条路。
