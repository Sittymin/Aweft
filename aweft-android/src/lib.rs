//! Android 入口：`android_main` 代替 `main`（要求见 slint 的 android 模块文档）。
//!
//! 只做 Android 特有的三件事：装 logcat 日志、交出 JVM/Activity 句柄并初始化
//! Slint 的 Android 后端、登记 GStreamer 静态插件；之后进与桌面同一套的
//! `aweft_gui::create_window`，面板、插件、播放链都不分叉。

#[cfg(target_os = "android")]
mod gst_plugins;
#[cfg(target_os = "android")]
mod logging;

/// android-activity 的应用句柄类型只在 Android 目标存在；
/// 非 Android 下本包不编译出任何东西（xbuild 只走 android target）。
#[cfg(target_os = "android")]
#[unsafe(no_mangle)]
fn android_main(app: slint::android::AndroidApp) {
    // 第一件事必须是日志：后面几步都可能失败，失败信息只能往 logcat 送。
    logging::install();
    // JavaVM 与 Activity 必须在 `app` 被移走之前取：下面 `slint::android::init(app)`
    // 会接管它。
    // 不用 ndk-context 的 `platform()` 拿，是因为那份全局量由 android-activity 自己写，
    // 我们从句柄上直接读更直白。
    let java_vm = app.vm_as_ptr();
    let activity = app.activity_as_ptr();
    // 授权弹框用的 `Activity.requestPermissions` 只在 Activity 上有，而 `ndk_context`
    // 里存的那份是 Application（android-activity 的 glue 交进去的就是它），所以这里
    // 必须把 Activity 单独递出去。句柄是 global ref，由 `app` 持有，`app` 随后进了
    // Slint 的 platform 全局量、进程内不释放，因此 gui 里存裸指针一直有效。
    aweft_gui::file_picker::android::set_activity(activity);
    // 这里不要再调 ndk_context::initialize_android_context：android-activity 的 glue
    // 在把控制权交给 android_main 之前已经初始化过一次（0.6.1 的 init.rs:285），
    // 而 ndk-context 那份初始化内部是 assert!(previous.is_none())，
    // 重复调用直接把进程 panic 掉。要用的句柄都在上面直接从 `app` 上取。
    if let Err(e) = slint::android::init(app) {
        log::error!("Slint Android 后端初始化失败：{e:?}");
        return;
    }
    // 插件登记失败不拦界面：还能浏览工程与面板，只是放不出媒体，
    // 缺了哪几个插件已经在 gst_plugins 里按名字报过。
    if let Err(e) = gst_plugins::init_and_register(java_vm, activity) {
        log::error!("GStreamer 启动失败：{e:?}");
    }
    if let Err(e) = aweft_gui::create_window() {
        log::error!("建窗失败：{e:?}");
    }
}
