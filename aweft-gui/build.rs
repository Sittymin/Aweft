fn main() {
    // `ComponentContainer` 与 `component-factory` 是实验类型：默认的内建类型表把它们
    // remove 掉了（typeregister.rs:626-627），只有 SLINT_ENABLE_EXPERIMENTAL_FEATURES
    // 存在时才用全集表（compiler/lib.rs:276 读该变量 → typeloader.rs:961 选表）。
    // 插件面板嵌进 Area 靠的就是这两样，所以宿主窗口必须带着它编译。
    // set_var 只喂给下面这行 slint_build::compile；cargo:rustc-env 到不了运行中的
    // 二进制——SLINT_LIVE_PREVIEW 下 .slint 是运行期当场编译的，那一刻读的是进程
    // 环境，这份由 flake.nix devShell 的 SLINT_ENABLE_EXPERIMENTAL_FEATURES 提供。
    unsafe { std::env::set_var("SLINT_ENABLE_EXPERIMENTAL_FEATURES", "1") };
    // release 与 Android 同一条静态分支：live-preview 代码嵌构建机绝对路径、建窗
    // 还依赖 shell 里的 SLINT_ENABLE_EXPERIMENTAL_FEATURES，都过不了分发。
    // cargo 给 build script 注入 PROFILE=debug|release，live preview 由此从环境
    // 开关变成 profile 开关——debug 全运行期编译，release 除插件（本就走
    // interpreter）外全静态，运行期不再读任何 SLINT_* 环境变量。
    if std::env::var("PROFILE").as_deref() == Ok("release") {
        unsafe { std::env::remove_var("SLINT_LIVE_PREVIEW") };
    }
    slint_build::compile("ui/app/window.slint").unwrap();
}
