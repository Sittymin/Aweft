# Android 移植工作台账（临时）

> 目标：`aweft-gui` 跑在 Android，真机可装可播，功能不损失。
> 硬解视频保留，文件对话框、剪贴板、字幕库做 Android 兼容，
> WGPU 继续用但从"与渲染器共享纹理"退为"离屏计算"。
> 本文件是工作记录：已验证的结论带证据出处，未验证的标验证方法。
> 更新规矩：完成一项就把对应行的状态改掉，不要另起文件。

## 已验证的结论（不要再反复查）

1. Slint 官方支持 Android。证据：`sources/slint` 按本工程锁的 rev `5e097e2`
   取出的 `api/rs/slint/Cargo.toml:218` 有 `backend-android-activity-06`，
   `:256` 的 `unstable-wgpu-30` 转发给 Android 后端；
   同目录 `docs/astro/.../guide/platforms/mobile/android.mdx:12` 起是 Rust 步骤，
   `:106` 写最低 API 26，`:279` 起是 `slint_main` 入口写法，`:321` 起是 `xbuild` 打包。
2. Android 后端的渲染器是 Skia，不是 femtovg-wgpu。证据：同一 rev
   `internal/backends/android-activity/Cargo.toml:28` 的目标依赖只有
   `i-slint-renderer-skia`。后果：凡是从 Slint 渲染器手里拿 `wgpu::Device`
   的代码，在手机上拿不到。
3. WGPU 能在 Android 上跑，但只能离屏用。证据：`sources/wgpu`（v29.0.4）
   的 `wgpu-hal/src/vulkan/instance.rs:254` 有 `VK_KHR_android_surface` 分支，
   `wgpu-hal/Cargo.toml:302` 有 android 专属依赖；全树搜
   `external_memory_android` / `ahardwarebuffer` 零命中，即没有
   AHardwareBuffer 导入纹理的 API，MediaCodec 输出零拷贝进 wgpu 此路不通。
4. 视频帧到屏幕的交接必须换。现状 `aweft-gui/src/render/video_sink/mod.rs:6`
   写的是 `slint::Image` 零拷贝，入口是 `Image::try_from(wgpu::Texture)`
   （见 `video_sink/tests.rs:2402` 的断言备注），这条只认同源 wgpu 纹理。
   替换件存在：同一 Slint rev `internal/core/graphics/image.rs:831`
   有 `Image::from_rgba8`，走 CPU 像素，任何渲染器（含 Skia）都吃。
5. 硬解码元素存在，但元素名不是固定的。证据：`sources/gstreamer`（1.29.2.1）
   `subprojects/gst-plugins-bad/sys/androidmedia/` 下有 `gstamcvideodec.c`
   （MediaCodec 视频解码）和 `gstamcaudiodec.c`。元素名由
   `sys/androidmedia/gstamc.c:1934` 的 `create_element_name` 逐个编出来：
   前缀是 `amcviddec-`/`amcauddec-`/`amcvidenc-`/`amcaudenc-` 四种（长度 10），
   后面接编解码器名里所有字母数字转小写（非字母数字字符全丢），
   例如 `c2.android.avc.decoder` → `amcviddec-c2androidavcdecoder`；
   注册点在 `:2037` 与 `:2082`，rank 规则在 `:2040`-`:2050`
   （硬件视频解码给 `GST_RANK_PRIMARY + 1`）。
   结论：元素集合随设备编解码器清单变，代码里不该写死任何 `amc*` 名字，
   让 `uridecodebin3`/`decodebin3` 按 rank 自己选。
6. `rfd` 与 `arboard` 没有 Android 支持。证据：registry 里 `rfd-0.15.4`
   的 `src/backend.rs` 只有桌面和 wasm 分支；`arboard-3.6.1` 的
   `src/platform/mod.rs` 在 Android 上是空模块，而 `src/lib.rs:71` 无条件
   用了 `platform::Clipboard`。所以 `bridge/callbacks.rs:67,257` 和
   `clipboard/backend/system.rs` 在手机目标上编不过，门控是必须的。
7. 插件执行器不用动。证据：`Cargo.lock` 里 `wasmi-2.0.0` 的依赖全是纯 Rust 包。
   `slint-interpreter`、`spin_on` 同理。
8. 字幕库离不开字体提供器。现状 `aweft-libass/src/lib.rs:131` 用
   `DefaultFontProvider::AutoDetect`，桌面等于 fontconfig；
   好消息是内嵌字体提取已经在了（`track.rs:132` 经 `ass_add_font`），
   所以 Android 路线大概率是自带 fonts.conf 指到 `/system/fonts`，
   不是重写字幕栈。
9. GStreamer Android 通用包 1.28.7 里没有任何共享库，全部是静态库。证据：
   `sources/gstreamer-android/arm64/` 下 `find . -name '*.so*'` 计数为 0；
   `arm64/lib/` 有 144 个 `.a`（核心与依赖，1330 MiB），
   `arm64/lib/gstreamer-1.0/` 有 247 个插件 `.a`（668 MiB）。
   后果：APK 打包不是"把 GStreamer 的 `.so` 塞进 `jniLibs`"，
   而是把这些 `.a` 链进 `aweft-android` 自己那一个 `libaweft_android.so`，
   xbuild 收集 native library 的能力与本项无关。
   包内四个 ABI 目录（arm64/armv7/x86/x86_64）各约 2.0 GiB，解压后合计 6.7 GiB，
   本机磁盘余量不支持全解，只解了 `arm64`。
10. 静态插件不会被 `gst::init()` 自动装载，必须在 Rust 侧显式注册，且只能注册在
    `gst::init()` **之后**。证据：
    `nm -u arm64/lib/libgstreamer-1.0.a | grep static_plugin` 无输出，
    即 `libgstreamer-1.0.a` 里没有对 `gst_init_static_plugins` 的未定义引用；
    每个插件 `.a` 自带导出符号，`nm -g --defined-only
    arm64/lib/gstreamer-1.0/libgstandroidmedia.a` 显示
    `T gst_plugin_androidmedia_get_desc` 与 `T gst_plugin_androidmedia_register`。
    顺序是硬要求：`subprojects/gstreamer/gst/gstplugin.c:231` 有
    `g_return_val_if_fail (_gst_plugin_inited != FALSE, FALSE)`，
    函数注释（`:204`）也写明"必须先 gst_init"，早于 init 调用会整批静默失败。
    上游那条自动路子在这类工程里不能白捡：`gst/gst.c:622` 的
    `init_static_plugins()` 在非 `GST_FULL_STATIC_COMPILATION` 分支里靠
    `g_module_open (NULL)` + `g_module_symbol` 自查 `gst_init_static_plugins`，
    而 APK 里我们这个 `cdylib` 由 `System.loadLibrary` 以 `RTLD_LOCAL` 载入，
    `dlopen(NULL)` 拿到的全局命名空间里没有我们的定义，查必落空。
    声明细节：`GST_PLUGIN_DEFINE`（`gst/gstplugin.h:274`）展开出的
    `gst_plugin_<name>_register` **返回 void**（`:300`），内部调
    `gst_plugin_register_static` 并把它的 `gboolean` 丢掉
    （`extern` 声明见 `:277`），所以 Rust 侧声明成 `-> i32` 是按一个
    不存在的返回值判成败，必须写成无返回。
    做法：对每个要用的插件写 `extern "C" { fn gst_plugin_<name>_register(); }`，
    在 `gst::init()` 之后逐个调用；Rust 引用到的符号会把归档里对应的
    目标文件拉进来，因此不需要 `-Wl,--whole-archive`。
11. `androidmedia` 插件里有三处 JNI 找类的代码（本轮用不上，限定见第 14 条）。证据：
   `grep -ao 'org/freedesktop/gstreamer[a-zA-Z0-9/_]*'
   arm64/lib/gstreamer-1.0/libgstandroidmedia.a` 得到三个类名
   `org.freedesktop.gstreamer.androidmedia.GstAhcCallback`、
   `GstAhsCallback`、`GstAmcOnFrameAvailableListener`，
   源码就在 `arm64/share/gst-android/ndk-build/androidmedia/*.java`。
   这三段要编成 `.dex` 放进 APK，否则 `libgstandroidmedia.a` 在运行时
   找类会失败。`uridecodebin3` 归属也一并确认：它只出现在
   `arm64/lib/gstreamer-1.0/libgstplayback.a`（该归档 9 次命中，
   `libgstcoreelements.a` 0 次），所以注册清单里必须带 `playback`。
12. 构建期解析用得上包内的 pkg-config 元数据，插件目录那批用不上。证据：
    `arm64/lib/pkgconfig/` 有 138 个 `.pc`，`aweft-gui/Cargo.toml:19-23` 列的
    五个绑定所需的 `gstreamer-1.0.pc`、`gstreamer-app-1.0.pc`、
    `gstreamer-video-1.0.pc`、`gstreamer-pbutils-1.0.pc`、
    `gstreamer-allocators-1.0.pc` 全在，`glib-2.0.pc`、`gobject-2.0.pc`、
    `gio-2.0.pc`、`gmodule-no-export-2.0.pc` 也在同一目录。
    `gstreamer-1.0.pc` 自述 `Requires: glib-2.0 >= 2.64.0, gobject-2.0`、
    `Requires.private: gmodule-no-export-2.0`、
    `Libs.private: -lm -latomic -llog`，闭合且不牵外部组件。
    反例：`arm64/lib/gstreamer-1.0/pkgconfig/gstandroidmedia.pc` 的
    `Requires.private` 含 `gstreamer-gl-1.0`、`gstreamer-photography-1.0`、
    `orc-0.4`，所以插件那批 `.pc` 不进 `PKG_CONFIG_PATH`，
    插件归档的链接参数由本工程自己写。
13. 通用包自带的 Android `fonts.conf` 不指向 `/system/fonts`。证据：
    `arm64/share/gst-android/ndk-build/fontconfig/fonts.conf` 的目录声明只有
    `<dir prefix="xdg">fontconfig/fonts</dir>` 与
    `<cachedir prefix="xdg">fontconfig</cachedir>`，
    同目录随包附 `fonts/Ubuntu-R.ttf`（345 KiB），全文无 `/system` 字样。
    即上游走的是"字体文件随 APK 进 assets、运行时解到应用私有目录、
    把该目录交给 fontconfig"。这条给"已验证"第 8 条的字幕字体路线提供先例：
    `aweft-libass` 在 Android 上照同样的形状自带字体目录，
    比猜 `/system/fonts` 里有什么更可控。
14. 本轮不需要往 APK 放 `.dex`。证据在本地 `sources/gstreamer`
    （检出 commit `4cdf9d9`，1.29.2.1，比通用包 1.28.7 新一个minor，
    结论按源码路径与函数名对读）：三个类各自只服务于一条与自己无关的路径，
    `GstAhcCallback` 仅出现在 `sys/androidmedia/gst-android-hardware-camera.c:1771`
    （硬件相机元素），`GstAhsCallback` 仅出现在
    `sys/androidmedia/gst-android-hardware-sensor.c:579`（传感器元素），
    `GstAmcOnFrameAvailableListener` 仅出现在
    `sys/androidmedia/jni/gstamcsurfacetexture-jni.c:267`，
    而 `GstAmcSurfaceTexture` 在 `gstamcvideodec.c` 里的调用点
    （`:186`、`:204`、`:262`、`:1275`、`:3185`）全部落在
    `video/x-raw(memory:GLMemory)` 分支内
    （该分支的 caps 由 `gstamcvideodec.c:389` 与 `:393` 构造）。
    本工程只协商 `video/x-raw(memory:SystemMemory)`，不进那条分支。
    仍需真机佐证：logcat 里若出现 `Failed to register native methods for
    GstAmcOnFrameAvailableListener` 或 `ClassNotFoundException`，本条作废，
    按第 15 条另找出路。
15. xbuild commit `e67b501` 没有把自定义 Java 类或 `.dex` 放进 APK 的配置项。
    证据：`apk` 包的 `ApkBuilder::add_dex`（`apk/src/lib.rs:95`）在
    `xbuild/src` 全树零调用；`android.gradle = true` 时 xbuild 生成 gradle 工程，
    源码目录只处理 `env.cargo().package_root().join("kotlin")`
    （`xbuild/src/gradle/mod.rs:181`），且是 `std::fs::copy(entry.path(),
    kotlin.join(entry.file_name()))` 平铺拷贝（`:190`），
    保留不了 `org/freedesktop/gstreamer/androidmedia/` 这种带包路径的目录。
    第 14 条说明本轮用不上；但剪贴板 JNI、`ACTION_OPEN_DOCUMENT`
    之类需要应用自己的 Java/Kotlin 类时会撞上，届时要么改 xbuild 要么换打包路径。
16. 交叉构建的 `.pc` 只从 `TARGET_PKG_CONFIG_PATH` 找，命令行前置
    `PKG_CONFIG_PATH` 无效。证据：`PKG_CONFIG_PATH=.../arm64/lib/pkgconfig
    nix develop --command sh -c 'echo $PKG_CONFIG_PATH'` 打印出来的值是
    33 个 `/nix/store/*-dev/lib/pkgconfig`，里面没有 arm64 目录
    （进壳时 nix 的 `pkg-config-wrapper-0.29.2` 按 buildInputs 重算了它）；
    紧接着的 `cargo build -p aweft-android --target aarch64-linux-android`
    链接行里带的是 `-L /nix/store/...-gstreamer-1.28.5/lib` 这类主机目录，
    报错为 `ld.lld: error: .../libgstreamer-1.0.so is incompatible with
    aarch64linux`（退出码 101）。
    规则：`gstreamer-rs` 的构建脚本经 Rust 的 `pkg-config` crate 取值，
    该 crate 在 `TARGET` 存在时读 `TARGET_PKG_CONFIG_PATH`，
    且这个变量一旦非空就整体替换 `PKG_CONFIG_PATH`，
    所以主机 `.pc` 目录必须拼在末尾兜底（`cargo check` 阶段的版本探测靠它们）。
    顺序坑：`arm64/lib/pkgconfig/` 自带 `freetype2.pc`、`fribidi.pc`、
    `harfbuzz.pc`、`harfbuzz-subset.pc`，与
    `aweft-libass/third_party/android-deps/lib/pkgconfig` 同名，
    谁在前谁中选，所以通用包目录只能排在 android-deps 之后。
    本轮已把该顺序写进 `flake.nix` 的 shellHook，路径可用 `GST_ANDROID_PKGCONFIG` 覆盖。
17. 静态链接路线已跑通。实测（2026-09-11，`flake.nix` 改完进壳重跑）：
    `cargo build -p aweft-android --target aarch64-linux-android` 退出码 0，
    产物 `target/aarch64-linux-android/debug/deps/libaweft_android.so`。
    `readelf -d` 的 `NEEDED` 只有 `libm.so`、`libandroid.so`、`liblog.so`、
    `libdl.so`、`libc.so` 五项，即没有任何外部 GStreamer/GLib 共享库依赖；
    `nm --defined-only` 在该产物符号表里数到 1744 个 `gst_` 定义，
    含 `t gst_init`、`t gst_element_factory_make`、`t g_object_ref_sink`，
    说明 `arm64/lib/` 那批 `.a` 已被吸收进这一个 `cdylib`。
    后果：APK 只需要带 `libaweft_android.so` 一个 native 库，
    `generic.runtime_libs` 用不上 GStreamer。
    动态导出同时含 `T android_main` 与 `T ANativeActivity_onCreate`，
    即清单用默认的 `android.app.NativeActivity` 就能起来。
    该条两点已收：`x build --release` 的产物是 strip 过的库；插件归档与
    注册清单见第 18 条。
18. 插件清单已链入并注册成功。`aweft-android/build.rs` 按 `PLUGINS`（20 个）
    补 `-L .../gstreamer-1.0` 与逐个 `-lgst<name>`，`src/gst_plugins.rs` 声明并
    调用 `gst_plugin_<name>_register()`。装机日志：
    `GStreamer 静态插件全部登记成功（20 个）`。
    体积代价：`libaweft_android.so` 75 856 664 字节（约 72 MiB），
    即整个通用包只进了这 20 个插件之后的结果。
    清单只收 C 编出来的归档：通用包 247 个归档里 47 个是 gst-plugins-rs 的
    Rust 静态库，各自带一份 hidden 的 `__rustc::__rdl_alloc`，
    链进来会让 lld 报 `undefined hidden symbol`（实测 dav1d）。
19. `androidmedia` 有两道门槛，缺一道就整块注册失败，`plugin_init` 不报错、
    只能事后从 registry 回查（见 `src/gst_plugins.rs` 的 `report_missing_plugins`）：
    - JavaVM：`gst_amc_jni_initialize_java_vm`（gstjniutils.c:621）先读
      应用设进去的 `java_vm`，所以要在 `register_all()` 之前调
      `gst_amc_jni_set_java_vm(app.vm_as_ptr())`。
    - Application ClassLoader：紧接着的 `initialize_classes()` 只做一件事——
      按名字 `dlsym` 找 `gst_android_get_application_class_loader`
      （gstjniutils.c:592）。查不到即返回 FALSE。core 里那份同名实现
      （`subprojects/gstreamer/gst/gstandroid.c:411`）靠 Java 侧
      `GSTAndroid.init(context)` 喂数据，纯 NativeActivity 进程里没人调，
      而且 `gstandroid.c.o` 根本没被拉进链接。
      解法是自己实现这个符号，见 `src/gst_plugins.rs` 的
      `gst_android_get_application_class_loader`。
    实测（2026-09-12，装机日志）：补上之前 `gst_amc_jni_initialize()` 返回 0，
    补上之后返回 1，`registry.features_by_plugin("androidmedia")` 不再为空，
    名字同时含 `amcviddec-*`、`amcauddec-*`、`amcvidenc-*` 三类，视频侧有
    `amcviddec-c2androidavcdecoder`、`-c2androidhevcdecoder`、
    `-c2androidvp9decoder`、`-c2androidav1decoder` 与高通
    `-c2qtiavcdecoder`/`-omxqcomvideodecoderhevc` 系列。
20. rustc 编 `cdylib` 时只导出 Rust 侧 `#[no_mangle]` 的符号，静态归档里的 C
    符号一律不可见。实测 `llvm-nm --defined-only --dynamic` 只数到 6 个 `T`
    （`android_main`、`ANativeActivity_onCreate` 等）。所以「按符号名在运行时
    反查实现」这条路（第 19 条那道门槛、以及 gstjniutils.c:631 找
    `gst_android_get_java_vm`）只有 Rust 侧自己给出符号才走得通，
    指望链进来的 core 帮忙导出是不可能的。
21. 别用 `dlopen(NULL)` + `dlsym` 去预判 `g_module_symbol` 的结果，两者不等价。
    实测同一个符号：Rust 侧 `dlopen(NULL, RTLD_NOW|RTLD_LOCAL)` + `dlsym`
    拿到 `0x0`，而 GLib 那条路（`check_application_class_loader`）拿到了，
    并且 `plugin_init` 随之成功。判据只能看 `plugin_init` 之后的 registry 状态。
22. 链接必须自己补 NDK 的 compiler-rt builtins。libffi 闭包跳板调
    `__builtin___clear_cache`，clang 把它降成对外部符号 `__clear_cache` 的调用，
    而 rustc 调链接驱动时带 `-nodefaultlibs`（实测捕获的 438 参数命令行第 394 行），
    驱动默认追加 builtins 那一步不会发生。bionic 不导出该符号（对照设备
    `libc.so`：`getrandom` 有、`__clear_cache` 无），rustc 自带的
    `libcompiler_builtins` 里也没有。失败形态不是编译错，而是装机后
    `dlopen failed: cannot locate symbol "__clear_cache"`。
    现在由 `build.rs` 的 `compiler_rt_builtins()` 给出归档路径。
23. `xbuild 0.2.0`（commit `e67b501`）与 nix 闭包里的 NDK r29、JDK 17 配合可用，
    真机包已由它产出。它不读 `ANDROID_HOME`/`ANDROID_NDK_ROOT`，
    所以本机 `~/Android/Sdk` 全程不参与。打包要记的四条：
    - `x build` 必须在 `aweft-android/` 目录里跑：它按 cwd 的包有没有
      `src/lib.rs` 判定是不是库工程（`command/build.rs:30`），
      在仓库根跑会报 `Android APKs/AABs require a library`。
    - 它只认死路径 `~/.cache/x/{Android.sdk,Android.ndk}`，而 `flake.nix:190`
      的 shellHook 每次进壳都把这两个软链重指到 nix 闭包。任何替换都要在
      进壳之后、同一只 shell 里做，先跑 shellHook 再覆盖。
    - `target_sdk_version` 默认 33（`config.rs:136`，`get_or_insert` 语义），
      `manifest.yaml` 里的值优先。
    - 用 nix 闭包里的真 API-35 `android.jar` 时，`Create apk` 步报
      `unrecognized chunk ResChunkHeader { ty: 15076, header_size: 0, size: 514 }`。
      报错里 `header_size: 0` 就是自证：任何合法 `ResChunkHeader` 的 header_size 不小于 8，
      所以那根本不是 chunk，是 xbuild 自己跳偏了之后从表中段读出来的字节。
      出错的文件是 **`android.jar` 自带的 `resources.arsc`**（框架资源表），
      不是 build-tools 产物——xbuild 全程没有调用 aapt/aapt2（`grep -rn aapt` 零命中）。
      机制：`apk/src/res.rs:1025` 读 `ResTableType` 的 entry 偏移时一律
      `read_u32::<LittleEndian>()`，随后 `:1030` 起顺序读 entry 且再也不 seek 回
      `entries_start`，于是遇到 API 35 的 16 位偏移编码（entry 偏移表在
      `ResTableType::flags` 置 `FlagOffset16` 时按 u16 存）就整体前移 2×entry_count 字节。
      `ResTableTypeHeader`（`res.rs:474`）结构体里没有 `flags` 字段，
      `ResTableConfig`（`res.rs:521`）把 `version` 之后的字节全塞进 `unknown: Vec<u8>`，
      所以它结构上就没法支持这个编码（`grep -rni offset16` 在 apk  crate 里零命中）。
      实测四代 jar 里触发该路径的 chunk 数：API 33 与 34 为 0，35 为 129，36 为 137，37 为 140，
      且全部满足 `entries_start = align4(20 + config_size + 2 × entry_count)`。
      绕法就是把 API-34 的 `android.jar` 放到 `platforms/android-35/` 路径下。
      上游未修：`apk/src/res.rs` 最近一次提交 `58eb125` 只动了 clippy。
      另记一处同源隐患：`apk/src/compiler/table.rs:168` 对未知 `attribute_type` 直接 `panic!`。
24. 三条与设备/生成器相关的坑：
    - `SLINT_LIVE_PREVIEW=1`（`flake.nix` 的 devShell 为桌面热预览设的）会让
      `slint-build` 换成「运行期读 `.slint` 源码再编译」的生成器
      （slint `internal/compiler/generator/rust.rs:172`），生成代码里嵌的是
      构建机绝对路径，手机上必然失败。`aweft-gui/build.rs` 已对 android 目标
      移除该变量。注意库侧 `live-preview` 特性仍然必须开（生成代码硬引用
      `sp::live_preview`）。
    - 同一份 `.slint` 在普通生成器与热预览生成器下借用检查结论不同：普通
      生成器把结构体字段读进属性时是移动，热预览那份走
      `LiveReloadingComponent` 不产生该绑定，所以只有编译真机包时才暴露
      `E0382`（已修，`ui/areas/timeline/timeline_editor.slint:137` 起）。
    - MIUI 上 `adb install -g` 被禁（`SecurityException: You need the
      android.permission.INSTALL_GRANT_RUNTIME_PERMISSIONS`），
      用普通 `adb install -r`，权限在运行时申请。
25. 真机日志里每次启动都出现的 `插件事件通道关闭，pump 退出` 是预期行为，不是缺陷。
    成因：`PluginBridge::load`（`aweft-gui/src/plugin_bridge.rs:40`）里 `event_tx`
    是局部变量，只按克隆交给每个装载成功的 guest；APK 里没有 `plugins/` 目录，
    `plugin_roots()`（`:146`）三项探测全落空，所以没有任何人持有它，`load` 一返回
    就被丢弃，`start_event_pump`（`:124`）的流立刻 `None`，打出这条 `info`。
    桌面端在没有插件的情况下跑也是同样一条。要做掉它得等"插件 wasm 打包"那行落地。
26. `backend-winit` 与 `backend-android-activity-06` 不存在共存问题。两组 slint 特性
    分别写在 `aweft-gui/Cargo.toml:41`（段头 `:40` 带 `cfg(not(target_os = "android"))`）
    与 `:63`（段头 `:62` 带 `cfg(target_os = "android")`），按目标求值，
    同一时刻只有一组参与解析。手机目标不能留着 `backend-winit` 不开渲染器：
    选择器自己把 `i-slint-backend-winit` 放在 `cfg(not(target_os = "android"))` 段
    （`internal/backends/selector/Cargo.toml:97-98`），Android 目标上它不在依赖图里，
    而它的 `lib.rs:118` 在没有渲染器特性时是 `compile_error!`。
    这条的最初观察记在 `aweft-gui/Cargo.toml:58-60` 的注释里。
    依据：`cargo build -p aweft-android --target aarch64-linux-android` 与桌面
    `cargo check -p aweft-gui` 两侧都退出码 0（第 17 条）。
27. SAF（`ACTION_OPEN_DOCUMENT`）在本工程的入口形态下走不通：`android-activity 0.6.1`
    没有 activity result 回执通道。证据：`PollEvent` 只有 `Wake`/`Timeout`/`Main`
    三个变体（`src/lib.rs:548`），`MainEvent`（`:456` 起）里没有任何 result 变体，
    全 `src` 目录 grep `onActivityResult`/`activity_result`/`start_activity` 零命中，
    即 `startActivityForResult` 发出去收不回来。
    所以文件选择只能用不需要回执的原生能力，现状已按这条路实现：MediaStore 查库 +
    `ContentResolver` 拷到应用私有缓存 + Slint 自绘弹层，见
    `aweft-gui/src/file_picker/android.rs`（编排入口 `run_android_picker:433`；
    授权分两处发起与等待，`request_video_permission:150` 在事件循环线程弹框、
    `wait_for_permission:185` 在工作线程轮询 40×500 ms，理由见第 29 条）。
    未验证的是真机上这一整条选择流程（见"未验证项"第 3 条）。
28. Android 上 `window().scale_factor()` 在建窗那一刻还是 `1.0`，真值要到
    `RenderingState::RenderingSetup` 才拿得到（读源码：Slint 的
    `androidwindowadapter.rs:211` 起，密度是在 `InitWindow` 事件里才
    `ScaleFactorChanged` 给窗口的）。真机实测到的后果：设备 1440x3200、density 3.5，
    而 `AppStore::new`（`aweft-gui/src/store/app_store.rs:39`）在建窗时经
    `ThemeSettings::from_main_window` 读到 1.0，整套布局按 1.0 算，画面大 3.5 倍。
    修法：`ScreenAction::InitializeLayout`（`aweft-gui/src/actions/screen.rs:13`）
    带上 `scale_factor`，在 `handle_rendering_setup`
    （`aweft-gui/src/bridge/callbacks.rs:770`，`:779` 现读一份 theme）里读真值再
    dispatch，reducer 在建布局之前先 `update_scale_factor`
    （`aweft-gui/src/reducers/screen.rs:100` 起）。修后同一行日志是
    `窗口 1440x3200@3.50`。
29. `ndk_context::android_context().context()` 给出的是 **Application**，不是
    Activity（读源码：android-activity 0.6.1 `init.rs:285` 交进去的是 `app_global`）。
    `Context` 上的方法用它都对：真机上已验到的是 `checkSelfPermission`（那次失败就
    发生在这一步之后），`getContentResolver` 与 `getCacheDir` 所在的查库、拷缓存路径
    还没走到，属于"读 Java API 判断可用"。而 `requestPermissions` 声明在 `Activity`
    上，拿 Application 调只会失败。所以 Activity 句柄由 `aweft-android` 的
    `android_main` 显式交出（`aweft-gui/src/file_picker/android.rs:85`
    `set_activity`），持有者是 `AndroidApp`，它被 `slint::android::init` 收进
    platform 全局量，随进程存活；Activity 因配置变化重建时这份会失效，
    重建路径本身还未接线。
30. 全屏窗口不扣系统栏那圈，顶栏就画在状态栏底下，而那一圈的触摸归系统，
    真机上表现为按钮点不到。数据来自 Slint 的 `Window.safe-area-insets`
    （`builtins.slint:1613`，Android 侧由 `androidwindowadapter.rs:536`
    `update_window_insets` 与 `:507` `resize` 写入）。接线：`ui/app/window.slint`
    导出四个 `safe-inset-*`，`handle_rendering_setup` 换算成物理像素存进
    `AppState::safe_inset`（`store/state.rs:126`），可用区一律走
    `Insets::usable_rect`（`screen/primitives.rs:138`），
    `current_window_rect()`（`store/state.rs:191`）是它的唯一出口。
    真机实测：修前日志 `上35 下16`（把逻辑像素当物理像素用了，成因同第 28 条），
    修后 `上123 下56`，123/3.5≈35.1 与状态栏厚度对上。
31. JNI 的方法名与签名在 Rust 侧编译期一点都不校验：写错方法名照样编译过，
    只在真机上抛 `NoSuchMethodError`（`jni` 的 Debug 还不带消息，见第 29 条那次）。
    本机有免上机的核对办法：拿打包用的那份 `android.jar` 直接 `javap`。
    ```sh
    javap -p -classpath /tmp/xsdk/platforms/android-35/android.jar android.content.ContentUris
    ```
    这条已经抓到一处：`ContentUris.withId(Uri,long)` 不存在，同类里只有
    `withAppendedId(Uri,long)`、`appendId`、`removeId`、`parseId`，签名同为
    `(Landroid/net/Uri;J)Landroid/net/Uri;`。`file_picker/android.rs` 里其余
    十个 Java 符号（`Uri.parse`、`ContentResolver.query` 五参重载、`Cursor` 四个
    取列方法、`File`/`FileOutputStream` 构造、`Context` 三个、
    `Activity.requestPermissions`、`MediaStore$Video$Media.EXTERNAL_CONTENT_URI`、
    `Build$VERSION.SDK_INT`）已逐个用 javap 对过，全部存在。
    设备实测环境：`ro.build.version.sdk = 33`（Android 13，小米 nuwa/2210132C）。

32. JNI 局部引用是 GC 根。`copy_to_cache` 原先每轮 `new_byte_array(65536)` 读一次、
    再 `byte_array_from_slice` 拷一份写一次，两个数组都不出本次调用的作用域，
    于是历轮数组全被攥住不放：153MB 的片源实测把 256MB 的 Java 堆占满，
    `OutOfMemoryError: Failed to allocate a 65552 byte allocation with 29048 free
    bytes and 28KB until OOM, target footprint 268435456`。
    改法是一个 `byte[]` 从头用到尾，写侧换 `FileOutputStream.write("([BII)V")`
    按本轮读到的长度就地写，既不再新建数组也不必把整块绕回 Rust 侧。
    装机复测：6.3MB 与 153MB 两个片源都拷完起播。
33. 元素名到插件名的归属不能凭印象。`pipeline.rs` 回退分支要的
    `autovideoconvert` 被清单注释记在 `playback` 名下，实际它在
    gst-plugins-bad 的 `autoconvert`（`subprojects/gst-plugins-bad/gst/autoconvert/plugin.c:37`），
    于是真机上 `Failed to find element factory with name 'autovideoconvert'`。
    核对办法是在本地 GStreamer 检出里反查注册宏，比记名字可靠：
    ```sh
    grep -rn "GST_ELEMENT_REGISTER_DEFINE (autovideoconvert" subprojects/
    ```
    `autoconvert` 已补进 `build.rs` 的 `PLUGINS`（21 个全部登记成功）。
    它的未定义符号只有 glib/gstreamer/libc 那几个，不引入新的第三方归档。
34. 授权轮询窗口给到 120 秒（`PERMISSION_POLLS` 240 × 500ms）是实测定的：
    原先 20 秒时人还没在 MIUI 弹框上答完就超时，日志报"用户未授予媒体读取权限"，
    而事后 `checkSelfPermission` 显示 `granted=true`——超时被误读成了拒绝。
    已知限制仍在：android-activity 0.6.1 没有 `onRequestPermissionsResult` 通道，
    轮询分不出"弹框还开着"与"已拒绝"两种状态，所以只能靠窗口长度把后者逼出来，
    代价是拒绝路径要等满 120 秒才反馈。
35. 硬解路径已由出帧间接证明：`build.rs` 的清单里没有任何软件 H.264/HEVC 解码器
    （解码器只有 `androidmedia` 的 `amcviddec-*` 与 opus/vorbis/theora/flac 四个音频的，
    `dav1d` 因 Rust 归档的 `__rdl_alloc` 冲突被排除，通用包里也没有 libav），
    `videoparsersbad` 只解析不解码。两个片源（bilibili 的 H.264、小米相机的 DOLBY）
    都能出帧，只可能是 `amcviddec-*` 在解。
    顺带确认 `amcvideodec` 无 Surface 时确实输出系统内存：下游 caps 由
    `video_caps_with(false, ten_bit)` 约束成纯 sysmem，协商通过并持续出帧，
    "未验证项"第 1 条就此收口。
36. Android 无声的唯一根因是清单少一个插件：`volume` 元素由 `libgstvolume.a`
    （插件名 `volume`）提供，而 `build.rs` 的注释把它记在 `coreelements` 名下，
    于是 `make("volume")` 恒失败。`build_audio_branch` 里四个 `let-else` 又全部
    静默 `return (None, None)`，所以日志里一点痕迹都没有——画面分支照常跑，
    表现就是"完全无声"。修法是清单补 `"volume"`（登记数 23→24），
    并把四个失败落点各加一条点名元素的 `warn`、`link_pad_by_media_type` 里
    被丢弃的 `pad.link()` 结果改为失败即 `warn`。
    归属判定方法已写进 `build.rs` 上方：`llvm-nm --defined-only <插件目录>/libgst*.a`
    找 ` <元素>_get_type`，落在哪个归档插件名就是哪个（去 `libgst` 前后缀）。
    同一次核查纠正了 `autoaudiosink`/`autovideosink` 的归属：它们在
    `libgstautodetect.a`（插件 `autodetect`），不在 `playback`。而且 Android 上
    链进来的 24 个插件没有一个注册 `DeviceProviderFactory`，`autoaudiosink` 即便
    链上也枚举不出设备，所以 `AUDIO_SINK_FACTORY` 在 android 目标取 `openslessink`，
    `has_audio_output()` 也改成探测该元素在不在注册表里。
    `openslessink` 的 sink caps 是离散集（从 `ar p libgstopensles.a openslessink.c.o |
    strings` 读出）：`format={S16LE,U8LE}`、`rate ∈ {8000,11025,12000,16000,22050,
    24000,32000,44100,48000}`、`channels=[1,2]`、`layout=interleaved`。
    度量坑（这台 MIUI 上别再踩）：`dumpsys media.audio_flinger` 的 track 行与
    `dumpsys audio` 的 `AudioPlaybackConfiguration state` 在**正在播**时也显示
    `Active no`/`G db -inf`/`Server 0`/`state:idle`，据此判定过的"没在播"是错的，
    最终结论由人耳给出。可用的旁证只有 `dumpsys audio` 事件行
    `new player piid:… type:OpenSL ES AudioPlayer (Buffer Queue) u/pid:<uid>/<pid>`、
    `ps -T` 里出现的 `AudioTrack` 线程，以及 logcat 里 `c2.android.aac.decoder` 在解码。
37. 包名已改 `org.aweft`（`aweft-android/manifest.yaml:7`，与域名 aweft.org 对齐）
    并装机核实：`am start -n org.aweft/android.app.NativeActivity` 起界面、
    选片、`✅ 视频系统连接成功，开始播放`、`音频分支已建：sink = openslessink` 全部复现。
    之前那次 `INSTALL_FAILED_USER_RESTRICTED: Install canceled by user` 只拦
    **设备上一个都没装过的新包名**，同一包名的 `-r` 重装不受影响；
    开 MIUI 开发者选项的「通过 USB 安装」后新包名即可安装（本次即在此之后装成）。
    换包名等于换应用身份：MediaStore 授权与 `cache/aweft-pick/` 里的拷贝都重来，
    验证时可用 `adb shell pm grant org.aweft android.permission.READ_MEDIA_VIDEO` 跳过弹框。

## 环境现状（`nix develop` 提供，2026-09-11 进壳实测通过）

`flake.nix` 的 devShell 已收编全部 Android 工具链，不再依赖本机 `~/Android/Sdk`。
进壳实测输出：`ANDROID_HOME` 指向 nix 闭包里的 androidsdk，`ANDROID_NDK_ROOT`
解析到 `ndk/29.0.14206865/`，`aarch64-linux-android26-clang` 存在，
`rustc --print target-list` 含两个 android 目标，adb 1.0.41、JDK 17 就绪。
版本：platform-tools 37.0.1、platform 35、build-tools 35.0.0、
NDK r29（`29.0.14206865`）、JDK 17 headless、Rust 1.97.1（带两套 android std）。
链接器与 `CC_<target>`、`PKG_CONFIG_ALLOW_CROSS` 由 shellHook 导出，无需手配。

本机 `~/Android/Sdk`（NDK r30 等）退为备用；模拟器沿用本机的，nix 闭包里没收
（体积大）；`/home/Sittymin/Android/Key/release-key.jks` 仍是发布签名密钥。

手动件已到位（2026-09-11）：`cargo install` 路径的 xbuild 已装（`~/.cargo/bin/x`，
git commit `e67b501c`）；GStreamer Android 通用二进制包落在
`sources/gstreamer-android/gstreamer-1.0-android-universal-1.28.7.tar.xz`
（946.9 MiB），`sha256sum -c 1.28.7.sha256sum` 输出 OK，
满足 `aweft-gui/Cargo.toml` 的 `v1_24` 下限。
包内解压只做 `arm64/`（2.0 GiB），另外三个 ABI 未解压：
`/home/Sittymin/Development` 挂载点余量 15 GiB，全解 6.7 GiB 之后
不足以支撑 aarch64 的 `target/` 构建产物。
需要 x86_64 模拟器时再补解 `x86_64/`。

版本决策：`minSdk 26`（Slint 硬性要求，见上文 android.mdx:106）；
`targetSdk`/`compileSdk` 取 35（与 nix 闭包里的 platform 一致），
NDK 用闭包里的 r29。

## 模块状态表

| 模块 | 文件 | 现状与改法 | 状态 |
|---|---|---|---|
| Slint 特性 | `aweft-gui/Cargo.toml:40,62` | 已按目标拆两组：非 Android 段（`:41`）留 `backend-winit`+`renderer-femtovg-wgpu`+`unstable-winit-030`，Android 段（`:63`）只给 `backend-android-activity-06`+`unstable-wgpu-30`+`live-preview`，两组合存问题因按目标求值而消解 | 完成 |
| 入口 | `aweft-android/src/lib.rs:16` | 没有改 `aweft-gui/src/main.rs`，另起 `aweft-android` cdylib 包写 `android_main`：先装 logcat 日志，再在 `app` 被移走之前取 `vm_as_ptr()`/`activity_as_ptr()`，然后 `slint::android::init`，把两个句柄交给 `gst_plugins::init_and_register`，最后进与桌面共用的 `aweft_gui::create_window`。注意不要再自己调 `ndk_context::initialize_android_context`：android-activity 0.6.1 的 glue（`init.rs:285`）已经初始化过，那份实现内部是 `assert!(previous.is_none())`，重复调用直接把进程 panic 掉（实测崩溃点 `ndk-context-0.1.1/src/lib.rs:87`） | 完成 |
| 字幕 wgpu 管线 | `aweft-gui/src/render/subtitle.rs:3` | 加 `not(target_os="android")` 门控；Android 用自建 wgpu 离屏设备跑同一 WGSL，读回 RGBA（见视频行） | 待办 |
| 视频 dmabuf 导入 | `render/video_sink/dmabuf.rs` | 整模块门控掉（Vulkan 外部内存导入，手机无此物） | 待办 |
| 视频 caps 协商 | `render/video_sink/pipeline.rs:34` | Android 分支固定只报 sysmem，不报 dmabuf；`try_vapostproc`（`:104`）失败回退已写好，不用动 | 待办 |
| 视频帧交接 | `render/video_sink/frame.rs:137` | 产出改走 `Image::from_rgba8`；1080p 一帧约 8MB 内存拷贝，可接受 | 待办 |
| 音频输出 | `render/video_sink/pipeline.rs:300` `build_audio_branch` | 分支形状与桌面同（`queue ! audioconvert ! volume`），只有末端按平台取 `AUDIO_SINK_FACTORY`：桌面 `autoaudiosink`，Android `openslessink`（前者属 `autodetect` 插件且依赖设备提供方枚举，手机上一个提供方都没链）。`has_audio_output()` 的 android 版改成探测该元素在不在注册表里，探测不到才退 `fakesink` | 完成：真机出声已由人耳确认（"已验证"第 36 条），旁证是 `dumpsys audio` 的 `new player … OpenSL ES AudioPlayer` 与 `c2.android.aac.decoder` 在解码。注意同条记录里那条度量坑：`dumpsys` 的 track 状态读数在这台 MIUI 上不可用作在播判据 |
| 视频解码 | `pipeline.rs:186` | `uridecodebin3` 不动，Android 通用包里自动选 `amcvideodec`；运行时日志确认协商到 `video/x-raw(memory:SystemMemory)`。前提：通用包里没有可自动扫描的插件目录，`uridecodebin3` 所在的 `playback` 与 `amcvideodec` 所在的 `androidmedia` 都要按"已验证"第 10 条显式注册才存在 | 完成：真机出帧即间接证明走的是 `amcviddec-*`（清单里没有任何软件 H.264/HEVC 解码器），无 Surface 时输出系统内存也已确认，见"已验证"第 35 条 |
| GStreamer 静态链接与插件注册 | `aweft-android/build.rs` + `src/gst_plugins.rs` | 核心由 `flake.nix` 的 `.pc` 顺序解决（"已验证"第 17 条）；插件由 `build.rs` 的 `PLUGINS` 清单给出链接行（`--start-group` 里逐个 `-lgst<name>`，另补 `.pc` 给不到的 `ffi`/`pcre2-8`/`iconv`/`gmodule-2.0`/`z`/`gstgl`/`gstcodecparsers`/`gsttag`/`gstphotography` 等），Rust 侧在 `gst_init` 之后逐个 `gst_plugin_<name>_register()`，24 个全部登记成功（"已验证"第 18、33、36 条；清单从 21 涨到 24 是补了 `autoconvert`、`opensles`+`audioparsers`、`volume`，每补一个都对应一次真机缺功能的实测，判定元素归属的方法记在 `build.rs` 上方与第 36 条）。`androidmedia` 另有两道门槛：`gst_amc_jni_set_java_vm(vm_as_ptr())` 与我们自己导出的 `gst_android_get_application_class_loader`（第 19、20 条），少一个就整块注册失败且不报错 | 完成 |
| 离屏 wgpu 设备 | 新增小模块 | 自建 `Instance`+`Device`（Vulkan/GLES 由 wgpu 按手机情况选），跑现有 `convert.rs` 着色器做 YUV→RGB，`map` 读回；与 Slint 渲染器无共享 | 待办 |
| 文件对话框 | `aweft-gui/src/file_picker/android.rs`、`bridge/callbacks.rs:104,279` | SAF 已排除（android-activity 0.6.1 无 activity result 回执通道，"已验证"第 27 条）。实现走 MediaStore 查库 + `ContentResolver` 拷进应用私有缓存再返回本地路径（下游 URI 组装与桌面同路），选择界面是 Slint 自绘弹层 `ui/areas/media_picker.slint`，回执经 oneshot 送回 `run_android_picker`；两处入口分别是 `Append`（`:104`）与 `Replace`（`:279`）。权限用 `requestPermissions` 发出后轮询 | 完成：真机整条流程跑通（授权 → 列表 → 选中 → 拷缓存 → 起播，见"已验证"第 32、34 条）。两处已知代价：无回执通道，拒绝路径要等满 120 秒轮询窗口才反馈；选片是把整份文件复制进 `cache/aweft-pick/`，与媒体库原件各占一份，没有剩余空间门限检查 |
| 剪贴板 | `clipboard/backend/mod.rs:14,35` | 门控已完成：`mod system` 只在非 Android 编译，Android 由 `default_backend()` 退到 `memory::InMemoryBackend`。真实现是 JNI 调 `ClipboardManager`（要应用侧类的话会撞上"已验证"第 15 条：xbuild 0.2.0 没有把自定义 Java/Kotlin 类进包的配置项），工作量小，放 P2 | 门控完成，JNI 待办 |
| 字幕库构建 | `aweft-libass/build.rs:31,48` | autotools 交叉（TARGET→配置参数是否自动带，未验证）+ 四个库改走 NDK 预编、`pkg-config` 改走 `PKG_CONFIG_PATH` 指向预编产物；`bindgen` 跑在主机无碍 | 待办 |
| 字幕字体 | `aweft-libass/src/renderer.rs:237` | 路线按"已验证"第 13 条定为照上游做法：字体文件随 APK 进 assets，运行时解到应用私有目录，把该目录交给字体提供器（上游形状见 `arm64/share/gst-android/ndk-build/fontconfig/`）；备选是查 libass 有无不经 fontconfig 的提供器 | 待办 |
| 插件 wasm 打包 | 新增 | `plugins/test-plugin/guest.wasm` 与面板 `.slint` 打进 APK assets（`slint-interpreter` 加载时编译，需要源文件在场）；`wasmi` 从内存字节建模块，不用落盘可执行 | 待办 |
| flake | `flake.nix` | 完成（2026-09-11）：`androidSdk` 合成件（platform-tools 37.0.1、platform 35、build-tools 35.0.0、NDK r29）、`jdk17_headless`、两套 android target、shellHook 导出 NDK/linker/`CC_<target>`；`accept_license` 与 unfree 只开在本工程 pkgs 实例 | 完成 |
| 打包 | `aweft-android/manifest.yaml` | 配置落点就是这个文件（不是新增 `android/` 目录），已有 `package: org.aweft`（反向域名对齐 aweft.org，改法与装机核实见"已验证"第 37 条）、`min_sdk_version: 26`、`target_sdk_version: 35`、三项读媒体权限。`x build --platform android --offline --release --arch arm64` 已出包装机（产物 `target/x/release/android/aweft-android.apk`），但**还依赖两处未固化的手工件**：`/tmp/xb-shim` 暴露裸 `clang`/`clang++`（xbuild 把 `CARGO_TARGET_*_LINKER` 设成 `clang`，而 nixpkgs 的 `androidndkPkgs.clang` 只有带三元组前缀的名字，且它设的环境变量优先级高于 `.cargo/config.toml`，只能靠 PATH）；`/tmp/xsdk` 把 API-34 的 `android.jar` 顶到 `platforms/android-35/`（原因见"已验证"第 23 条）。xbuild 能承接的输入只有两处：`generic.runtime_libs`（`xbuild/src/config.rs:410`）与 `android.assets`（`:440`，可给目录、带 `optional` 与 zip 对齐选项）。按"已验证"第 9 条，没有 GStreamer 的 `.so` 要收集，进包的只有字幕字体、插件 `guest.wasm` 与面板 `.slint`；签名指 `Key/release-key.jks` | 部分完成：出包装机跑通，两处手工件与 assets/发布签名待办 |
| 日志 | `aweft-android/src/logging.rs` | 已完成：`android_main` 第一件事就是 `logging::install()`，把 `log` 门面接到 `__android_log_write`（tag `aweft`），后端初始化、建窗、插件登记这几步的失败信息都留在 logcat。看日志用 `adb -s <serial> logcat -d -s aweft` | 完成 |

## 分阶段推进

- P0 编过：**完成**（2026-09-11）。验收命令 `cargo check -p aweft-android
  --target aarch64-linux-android` 与 `cargo build -p aweft-android
  --target aarch64-linux-android` 退出码均为 0，
  剩余告警 7 条，全部是 Android 用不到的 DMA-BUF 代码。
  原计划"先不接 GStreamer"的规避不需要了：核心库已经链进产物（见"已验证"第 17 条）。
- P1 点亮 UI 壳：**完成**（2026-09-12）。真机截图能看到时间轴标尺、滚动条与
  「打开视频」按钮，日志走到 `窗口初始化完成`，无 panic。
  原计划里"662 MB 未 strip 调试库"的收敛由 `x build --release` 顺带解决
  （产物 `libaweft_android.so` 约 72 MiB，含 20 个插件归档）。
- P1.5 打包可复现：`/tmp/xb-shim`（裸 `clang`/`clang++`）与 `/tmp/xsdk`
  （API-34 jar 顶到 android-35 路径）两处手工件固化进 `flake.nix`。
  候选做法：clang 用 nix 侧 linkFarm 暴露裸名（`androidndkPkgs_29.clang` 本身
  不行，它的 `bin/` 只有带三元组前缀的名字）；jar 用
  `platformVersions = [ "34" "35" ]` 配 `manifest.yaml` 里
  `target_sdk_version: 34`，把替换需求整个消掉。
- P2 功能补齐：剪贴板 JNI、libass NDK 构建 + fonts.conf、插件面板在 Android 上换装载路线
  （`create()`+`show()` 在这台后端上得不到第二个窗口，要改走 `ComponentContainer`
  + `ComponentFactory::new` 的嵌入路线并开 slint-interpreter 的 `internal` 特性，
  依据见 `docs/tmp/plugin-project-store.md` 的"装载路线"段）。文件选择已完成（第 32、34 条）。
- P3 视频硬解与音频：**解码、出帧、出声三项均已跑通**（2026-09-12）。
  `androidmedia` 注册成功，`amcviddec-*` 在真机上解 H.264/HEVC 并输出系统内存，
  经 `autovideoconvert` 的 CPU 转换到 `appsink`，画面渲染出来，见"已验证"第 33、35 条；
  音频由 `openslessink` 出，真机人耳确认，无声那次的根因与度量坑见第 36 条。
  剩下的是把转换与交接换成零拷贝：dmabuf 导入模块门控、离屏 wgpu 跑 `convert.rs`
  做 YUV→RGB、`Image::from_rgba8` 交接（`frame.rs` 那 7 条 dead-code 告警由这条路消化）。
- P4 发布：`release-key.jks` 签名，targetSdk 定版，回归桌面构建（`cargo test` 全量仍绿）。

## 验证命令备忘

```sh
nix develop --command bash -c 'cargo check -p aweft-gui --target aarch64-linux-android'
# 环境变量（ANDROID_HOME、ANDROID_NDK_ROOT、CARGO_TARGET_*_LINKER、
# CC_aarch64_linux_android 等）全部由 flake 的 shellHook 导出，不用手配。

# check 不做链接，验证 GStreamer 是否真的接上要用 build：
nix develop --command cargo build -p aweft-android --target aarch64-linux-android
# 核对产物是否只依赖 Android 系统库、GStreamer 是否被静态吸收：
readelf -d target/aarch64-linux-android/debug/deps/libaweft_android.so | grep NEEDED
nm --defined-only target/aarch64-linux-android/debug/deps/libaweft_android.so | grep -c ' [Tt] gst_'
```

禁忌：不要在 `nix develop` 之前用命令行前置 `PKG_CONFIG_PATH` 去指通用包的
`.pc` 目录。进壳时 nix 的 pkg-config 包装器会按 buildInputs 重算这个变量，
外面传的值整体丢失（实测见"已验证"第 16 条）。交叉构建的搜索路径只有一条通路：
`flake.nix` 的 shellHook 导出的 `TARGET_PKG_CONFIG_PATH`。

出包到真机（本轮跑通用的一条链，两处 `/tmp` 手工件待按 P1.5 固化）：

```sh
cd aweft-android && nix develop .. --command bash -c \
  'export PATH=/tmp/xb-shim:$PATH;
   ln -sfn /tmp/xsdk "$HOME/.cache/x/Android.sdk";   # 必须在 shellHook 之后：进壳时它已被重指到 nix 闭包
   x build --platform android --offline --release --arch arm64'
# 装机前的符号门：每个强未定义符号（含 NOTYPE，只要 GLOBAL）要么在自身 .dynsym，
# 要么在 DT_NEEDED 列出的库里；只看 NDK 全部 stub 的并集会误判通过。
nix develop --command bash /tmp/und-check.sh \
  "$PWD/../target/x/release/android/arm64/cargo/aarch64-linux-android/release/libaweft_android.so" 26
cd ..   # APK 在仓库根的 target/ 下，不在 aweft-android/ 里（照抄上一行的目录会报 No such file）
adb -s <serial> install -r target/x/release/android/aweft-android.apk   # MIUI 上不要用 -g
adb -s <serial> shell am start -n org.aweft/android.app.NativeActivity
# 换包名后 MediaStore 授权要重答；联调时先预授可跳过 120 秒轮询弹框
adb -s <serial> shell pm grant org.aweft android.permission.READ_MEDIA_VIDEO
adb -s <serial> logcat -d -s aweft
```

## 未验证项（逐项消灭，确认后移到"已验证"）

1. `autotools` 包是否自动处理交叉 `--host`：P2 看报错。
2. libass 有没有不经 fontconfig 的字体提供器：spike 后定，没有就走自带 fonts.conf。

原先挂在这里的四条已收口，不再重复：后端特性共存（第 26 条）、
`system-deps` 的链接参数与插件归档清单（第 16、17、18 条）、
xbuild 与 nix 闭包里 NDK r29/JDK 17 的配合及 `live-preview` 在手机目标的可用性
（第 23、24 条）、`amcvideodec` 无 Surface 时的输出内存与真机文件选择整条流程
（第 32、33、35 条）。
