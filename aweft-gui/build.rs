fn main() {
    println!("cargo:rerun-if-env-changed=SLINT_LIVE_PREVIEW");
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
