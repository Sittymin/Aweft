# 应用设置（`aweft-settings`）

## 定位与依赖方向

下层成员，与 `aweft-model` / `aweft-project` 平级，是本工程唯二不引用 Slint 的成员之一。
它只依赖 `aweft-storage`；反方向上 `aweft-gui` 与根包的命令行入口依赖它
（`aweft-android` 只提交配置目录，因此依赖的是 `aweft-storage` 而不是本成员）。
以后做无头 CLI 时读的是同一份类型，不需要经过界面。

落盘的公共规矩不在本成员：配置目录怎么定、原子写、版本门都在 `aweft-storage`，
全工程只有那一份。本成员只有一个文件：

```
aweft-settings/src/lib.rs   # Settings 类型、格式版本戳、进程内那一份（settings()）
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

`format` 是文件自身的格式版本。`Settings` 实现 `aweft_storage::json::JsonFile`，
读写走 `read_json_or_default` 与 `write_json`，版本门在那一处统一实施。规则：

- **加字段不 bump**：`Settings` 上挂了容器级 `#[serde(default)]`，旧文件缺的字段自动取默认值。
- **只有"旧值必须改写才有意义"的改动才 bump**，并在 `Settings` 的 `JsonFile::migrate` 里
  按版本顺序补一步。不做 Blender 那套按字段名重建（SDNA），因为设置文件是我们的，不是第三方产的。
- 读到的 `format` 比本程序支持的新时**直接拒绝**（错误里点名两个版本号与文件路径），
  不猜测也不改写——否则旧程序会把新程序的设置悄悄抹平。

## 配置目录

目录怎么定是 `aweft-storage` 的 `location` 模块管的，全工程共用一份：显式覆盖（环境变量
`AWEFT_USER_CONFIG`）优先，其次 `platform_config_dir()` 算出来的平台目录，Android 上返回
`None`。设置这一侧只关心两件事：

- **谁来提交**：入口在启动早期调一次 `aweft_storage::set_config_dir`。桌面 `src/main.rs`
  进 `main` 第一件事，Android `aweft-android/src/lib.rs` 在 `slint::android::init(app)`
  移走 `app` 之前用 `app.external_data_path()`（android-activity 0.6.1 `src/lib.rs:1277`，
  实测为 `/storage/emulated/0/Android/data/<包>/files`，应用自己可写，`adb push` 通）。
- **时序约束（改动时先看这条）**：`settings()` 首次访问就按当时的配置目录读盘并缓存整个进程，
  之后再 `set_config_dir` 不再生效。所有入口都必须"先定目录，后读设置"。

设置文件自己的位置不能由设置文件里的字段决定，所以覆盖只能留在进程外的那个环境变量上；
它是给用户看的配置入口，不是测试开关——测试直接把目录交给 `save_to_dir` 与 `read_json_or_default`。

## 读盘失败怎么办

设置文件坏了不能挡住启动：目录未定、读不出、解析失败，一律退回出厂值并留一条 `warn`。
文件不存在按出厂默认处理（首次启动就是这个状态）。唯一的硬失败是上面那条版本门。

## 写盘

原子写也在 `aweft-storage`：临时文件 → `write_all` → `sync_all` → `rename`，失败路径上删掉
临时文件，临时名带进程号与进程内序号（命令行与界面可能同时在写同一份文件）。工程文件、
协作记录与名册用的是同一条路径，规矩只在这一处写。

`save_to_dir` 目前在产品代码里还没有调用点——第一个写入口是设置界面（见
`docs/tmp/plugin-panel-android.md` §9 片 B 之后），在那之前手工编辑
`<配置目录>/aweft-settings.json` 是唯一的写法。

## 没有撤销

设置改动不做撤销栈。只有三件事：脏标记、显式保存、放弃改动＝重新读一次盘。
对照 Blender：它的偏好面板同样没有撤销（`WM_OT_read_userpref` 就是从盘重读），
`UserDef U_backup = dna::shallow_copy(U)` 只为算差集发通知，不用于回退；
Blender 真正的内存撤销是文档撤销（`MemFileUndoStep`，受 `U.undosteps` / `U.undomemory`
限制，超出即释放最老的步骤），跟设置是两条路。
