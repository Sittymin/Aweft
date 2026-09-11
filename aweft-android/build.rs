//! Android 专用：把 GStreamer 通用包里的插件归档链进本包的 `cdylib`，
//! 并生成 `src/gst_plugins.rs` 要 include 的注册代码。
//!
//! 通用包（`sources/gstreamer-android/arm64/`）里没有 `.so`，核心与 247 个插件
//! 全是 `.a`，所以插件不会在运行时被扫描到：链接由这里给出，注册由生成的
//! `gst_plugin_<name>_register()` 声明给出。两边清单同源，改这里就够了。

/// 注册清单。取值依据是 `aweft-gui` 代码里实际出现的元素名，不是通用包的全集：
/// 每多一个解码器都直接进 APK 体积。
///
/// 硬约束：清单里只能有 C 编出来的插件归档。通用包 247 个插件归档里有 47 个是
/// gst-plugins-rs 的 Rust 静态库（`ar t` 里带 `.rcgu.o` 成员，实测：aws burn cdg
/// claxon dav1d demucs elevenlabs fallbackswitch ffv1 gif gopbuffer
/// hlsmultivariantsink hlssink3 hsv icecast isobmff json lewton livesync
/// mpegtslive ndi originalbuffer quinn raptorq rav1e regex reqwest rsanalytics
/// rsaudiofx rsaudioparsers rsclosedcaption rsinter rsonvif rspng rsrtp rsrtsp
/// rstracers rsvideofx rswebrtc speechmatics streamgrouper textahead textwrap
/// threadshare togglerecord uriplaylistbin webrtchttp）。它们各自带一份 std 的
/// 内部分配器符号 `__rustc::__rdl_alloc`/`__rdl_dealloc`，是 hidden 的、只由
/// 那个静态库自己的 std 定义，链进本 cdylib 会让 lld 报
/// `undefined hidden symbol`——所以 dav1d 已被移出清单，别加回来。
///
/// 注释里的元素归属必须查出来再写，不能凭印象：`llvm-nm --defined-only */libgst*.a |
/// grep ' <元素>_get_type$'` 落在哪个归档，插件名就是那个（去 libgst 前后缀）。两处
/// 错判都造成过装机后静默缺功能：缺 `volume` 让音频分支整个建不起来（无声），
/// `autoaudiosink` 其实属于 `autodetect` 而非 `playback`。
const PLUGINS: &[&str] = &[
    // 管线本身用的元素
    "coreelements", // capsfilter queue fakesink filesink
    "app",          // appsink
    "playback",     // uridecodebin3 decodebin3 parsebin urisourcebin
    "typefindfunctions", // 类型探测，demuxer 选型靠它
    "videoconvertscale", // videoconvert
    "autoconvert",       // autovideoconvert：无 vapostproc 时的 CPU 转换回退（装机实测缺它就建不了管线）
    "audioconvert",
    "audioresample",
    "volume",            // volume —— 音频分支的音量元素，缺它整条分支建不出来
    "opensles",          // openslessink —— Android 上唯一可用的音频输出元素，没它就出声不了
    // 容器与流
    "matroska", // matroskamux/demux
    "isomp4",   // qtmux/mov
    "videoparsersbad", // h264parse hevcparse，喂给 MediaCodec 前的必需一环
    "audioparsers", // aacparse ac3parse mp3parse...，parsebin 缺它就解不出音频流（装机实测无声的成因之一）
    "taglib",
    "id3demux",
    "apetag",
    "xingmux",
    // 解码器：AV1/VP9/H.264/HEVC 一律走 MediaCodec（插件 androidmedia，元素名
    // amcviddec-*，随设备编解码器清单变）；软件解码只留 C 实现的这几个。
    "androidmedia",
    "opus",
    "vorbis",
    "theora",
    "flac",
    "wavparse",
];

fn main() {
    println!("cargo:rerun-if-env-changed=GST_ANDROID_PKGCONFIG");
    println!("cargo:rerun-if-env-changed=ANDROID_NDK_ROOT");

    let target = std::env::var("TARGET").unwrap_or_default();
    let out_dir = std::env::var("OUT_DIR").unwrap();
    let generated = std::path::Path::new(&out_dir).join("gst_static_plugins.rs");

    if !target.ends_with("-linux-android") {
        // 桌面与其它目标不链任何插件，生成空清单让 src 侧的 cfg 分支照样能 include。
        std::fs::write(generated, "").unwrap();
        return;
    }

    // NDK 目标上 log 库必须显式给出，__android_log_write 由 src/lib.rs 直接声明调用。
    println!("cargo:rustc-link-lib=dylib=log");

    let Ok(pkgconfig) = std::env::var("GST_ANDROID_PKGCONFIG") else {
        panic!(
            "Android 构建需要 GStreamer 通用包的 .pc 目录。请在 nix develop 里构建，\
             flake.nix 的 shellHook 会导出 GST_ANDROID_PKGCONFIG；手动指定时它是 \
             <通用包>/<abi>/lib/pkgconfig。"
        );
    };
    // .../arm64/lib/pkgconfig → .../arm64/lib/gstreamer-1.0
    let plugins_dir = match std::path::Path::new(&pkgconfig).parent().map(|lib| lib.join("gstreamer-1.0")) {
        Some(dir) if dir.is_dir() => dir,
        other => panic!("插件归档目录不存在（由 GST_ANDROID_PKGCONFIG={pkgconfig} 推得）：{other:?}"),
    };
    for name in PLUGINS {
        let archive = plugins_dir.join(format!("libgst{name}.a"));
        if !archive.is_file() {
            panic!("缺少插件归档 {archive:?}");
        }
    }

    println!("cargo:rustc-link-search=native={}", plugins_dir.display());
    // 插件归档引用核心与第三方解码器的符号，核心又反过来被这里的插件引用，
    // 单一遍扫描的链接器按顺序会解不开，所以整体圈进一个 group 里迭代求解。
    // 只补 gstreamer-rs 的构建脚本没给的那批：插件独有的第三方解码/封装库。
    // 名字必须与 arm64/lib 下的归档严格对应（大小写敏感，FLAC 就是大写）。
    //
    // 后半截是“gstreamer-sys 那批 .pc 永远不会给出”的依赖，两种成因：
    // pkg-config 非静态模式不读 Requires.private 与 Libs.private，于是
    // glib→pcre2-8/iconv、gobject→libffi、gio→gmodule-2.0 全缺；
    // 以及插件链到但 .pc 里没有的 GStreamer 辅助库——androidmedia（也就是
    // amcvideodec 本身）引用 gstgl 与 photography，videoparsersbad 引用
    // codecparsers，id3demux 与 isomp4 引用 gsttag。
    // 这批的判据不是读出来的，是实测：产物里 191 个强未定义符号按
    // `llvm-nm --undefined-only` 反查 arm64/lib 归档，每个符号的提供者都算过，
    // 少一个就是一次 `dlopen failed: cannot locate symbol`。
    println!("cargo:rustc-link-arg=-Wl,--start-group");
    for name in PLUGINS {
        println!("cargo:rustc-link-arg=-lgst{name}");
    }
    for name in [
        "FLAC", "ogg", "opus", "orc-0.4", "tag", "theora", "theoradec", "theoraenc", "vorbis",
        "vorbisenc", "vorbisfile",
        // .pc 给不到的那批，见上
        "bz2", "ffi", "gmodule-2.0", "iconv", "pcre2-8", "z",
        // opensles 归档引用 slCreateEngine 与 SL_IID_*，由系统 libOpenSLES.so 提供
        // （NDK sysroot 里从 API 21 起就有，min_sdk 26 覆盖得到）。
        "OpenSLES",
        "gstcodecparsers-1.0", "gstgl-1.0", "gstphotography-1.0", "gstriff-1.0",
        "gstrtp-1.0", "gsttag-1.0",
    ] {
        println!("cargo:rustc-link-arg=-l{name}");
    }
    println!("cargo:rustc-link-arg=-Wl,--end-group");

    // libffi 的闭包跳板调 `__builtin___clear_cache`，clang 把它降成对外部函数
    // `__clear_cache` 的调用（不是内联指令）。这个符号三处都没有：
    // bionic 不导出它（实测设备 libc.so 的符号表里没有，`getrandom` 有）；
    // rustc 自带的 libcompiler_builtins 里也没有；平时靠 clang 驱动在链接末尾
    // 默认追加 builtins 才解得开，而 rustc 调驱动时带了 `-nodefaultlibs`
    //（实测捕获的 438 参数链接命令第 394 行），于是这份必须自己给。
    // 不加的后果不是编译失败而是装机后 `dlopen failed: cannot locate symbol`。
    println!(
        "cargo:rustc-link-arg={}",
        compiler_rt_builtins(&target).display()
    );

    // GST_PLUGIN_DEFINE 展开出的 gst_plugin_<name>_register 返回 void：
    // 它内部调 gst_plugin_register_static 并把结果丢掉，所以这里问不出成败，
    // 由 src/gst_plugins.rs 事后查 registry 验证。
    let mut out = String::from("// 由 aweft-android/build.rs 生成，勿手改。\n");
    out.push_str("pub(super) const PLUGIN_NAMES: &[&str] = &[\n");
    for name in PLUGINS {
        out.push_str(&format!("    \"{name}\",\n"));
    }
    out.push_str("];\n\nunsafe extern \"C\" {\n");
    for name in PLUGINS {
        out.push_str(&format!("    fn gst_plugin_{name}_register();\n"));
    }
    out.push_str("}\n\npub(super) fn register_all() {\n");
    for name in PLUGINS {
        out.push_str(&format!("    unsafe {{ gst_plugin_{name}_register() }};\n"));
    }
    out.push_str("}\n");
    std::fs::write(generated, out).unwrap();
}

/// 目标三元组对应的 NDK compiler-rt builtins 归档绝对路径。
///
/// clang 的版本号写进路径会随 NDK 升级失效，所以这里枚举 `lib/clang/*/` 取实存的那个。
fn compiler_rt_builtins(target: &str) -> std::path::PathBuf {
    let arch = match target {
        "aarch64-linux-android" => "aarch64",
        "armv7-linux-androideabi" => "arm",
        "x86_64-linux-android" => "x86_64",
        "i686-linux-android" => "i686",
        other => panic!("未预期的 Android 目标 {other}"),
    };
    let ndk = std::env::var("ANDROID_NDK_ROOT")
        .expect("Android 构建需要 ANDROID_NDK_ROOT，flake.nix 的 shellHook 已导出");
    let clang_dir = std::path::Path::new(&ndk).join("toolchains/llvm/prebuilt");
    let file = format!("libclang_rt.builtins-{arch}-android.a");
    for host in std::fs::read_dir(&clang_dir)
        .unwrap_or_else(|e| panic!("读取 {clang_dir:?} 失败：{e}")) {
        let vers = std::fs::read_dir(host.unwrap().path().join("lib/clang"));
        for v in vers.unwrap() {
            let candidate = v.unwrap().path().join("lib/linux").join(&file);
            if candidate.is_file() {
                return candidate;
            }
        }
    }
    panic!("在 {clang_dir:?} 下找不到 {file}");
}
