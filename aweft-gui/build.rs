fn main() {
    // `ComponentContainer` 与 `component-factory` 是实验类型：默认的内建类型表把它们
    // remove 掉了（typeregister.rs:626-627），只有 SLINT_ENABLE_EXPERIMENTAL_FEATURES
    // 存在时才用全集表（compiler/lib.rs:276 读该变量 → typeloader.rs:961 选表）。
    // 插件面板嵌进 Area 靠的就是这两样，所以宿主窗口必须带着它编译。
    // 运行期也要有这一份：SLINT_LIVE_PREVIEW 下 .slint 是在二进制里当场编译的。
    println!("cargo:rustc-env=SLINT_ENABLE_EXPERIMENTAL_FEATURES=1");
    unsafe { std::env::set_var("SLINT_ENABLE_EXPERIMENTAL_FEATURES", "1") };
    slint_build::compile("ui/app/window.slint").unwrap();
}
