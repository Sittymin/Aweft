fn main() {
    println!("cargo:rerun-if-env-changed=SLINT_LIVE_PREVIEW");
    // `ComponentContainer` 与 `component-factory` 是实验类型：默认的内建类型表把它们
    // remove 掉了（typeregister.rs:626-627），只有 SLINT_ENABLE_EXPERIMENTAL_FEATURES
    // 存在时才用全集表（compiler/lib.rs:276 读该变量 → typeloader.rs:961 选表）。
    // 插件面板嵌进 Area 靠的就是这两样，所以宿主窗口必须带着它编译。
    // 运行期也要有这一份：SLINT_LIVE_PREVIEW 下 .slint 是在二进制里当场编译的。
    println!("cargo:rustc-env=SLINT_ENABLE_EXPERIMENTAL_FEATURES=1");
    unsafe { std::env::set_var("SLINT_ENABLE_EXPERIMENTAL_FEATURES", "1") };
    // SLINT_LIVE_PREVIEW=1（flake.nix 的 devShell 设的，桌面改 .slint 免重编）会让
    // slint-build 生成“运行期读 .slint 源码再编译”的窗口代码
    //（slint internal/compiler/generator/rust.rs:172），里面嵌的是构建机的绝对路径。
    // 手机上没有那份源码，建窗就报 `Could not compile …/ui/app/window.slint`，
    // 所以 Android 强制走普通的全量编译分支（顺带不把 slint 编译器链进 APK）。
    if std::env::var("CARGO_CFG_TARGET_OS").as_deref() == Ok("android") {
        unsafe { std::env::remove_var("SLINT_LIVE_PREVIEW") };
    }
    slint_build::compile("ui/app/window.slint").unwrap();
}
