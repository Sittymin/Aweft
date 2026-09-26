//! GStreamer 静态插件注册（仅 Android）。
//!
//! 通用包 `sources/gstreamer-android/arm64/` 里核心与 247 个插件全是 `.a`，没有
//! `.so`：运行时没有可扫描的插件目录，`gst_registry_scan_path` 一条也找不到。
//! 所以插件要先由 `build.rs` 链进本 `cdylib`（清单在 build.rs，两边同源），
//! 再由这里在 `gst_init` 之后逐个登记进默认 registry。
//!
//! 顺序是硬要求：`gst_plugin_register_static` 里有
//! `g_return_val_if_fail (_gst_plugin_inited != FALSE, FALSE)`，早于 `gst_init`
//! 调用会整批静默失败，所以本模块自己先把 init 做掉。

use anyhow::Context;
use std::ffi::c_void;
use std::ptr::null_mut;
use std::sync::atomic::{AtomicPtr, Ordering};
use std::sync::Once;

include!(concat!(env!("OUT_DIR"), "/gst_static_plugins.rs"));

// `androidmedia` 的 plugin_init 第一件事就是取 JavaVM，取不到直接返回 FALSE，
// 于是 amcviddec-* 一个都注册不上（实测装机日志：登记缺失 ["androidmedia"]）。
// 它给的三条路里只有第一条在纯 NativeActivity 进程里走得通——应用自己把 VM 交出来
//（gstjniutils.c:626 读 java_vm，631 找 `gst_android_get_java_vm` 符号，
// 644 之后要 dlopen libdvm/libjvm，现代 Android 上都不成立）。
// 公开接口是 gst_amc_jni_set_java_vm（gstjniutils.h:110）。
unsafe extern "C" {
    fn gst_amc_jni_set_java_vm(java_vm: *mut c_void);
}

/// `AndroidApp::vm_as_ptr()` 与 `activity_as_ptr()`。两个指针的有效期都由进程
/// 保证：JavaVM 由 ART 持有，Activity 那份是 android-activity 建的 global ref。
/// 之所以放进原子量而不是当参数传下去，是因为下面那个符号是 C 侧按名字调的，
/// 签名里没有地方收参数。
static JAVA_VM: AtomicPtr<c_void> = AtomicPtr::new(null_mut());
static ACTIVITY: AtomicPtr<c_void> = AtomicPtr::new(null_mut());

/// `Activity.getClassLoader()` 的 global ref，只解析一次。
///
/// 用 `Once` + `AtomicPtr` 而不是 `OnceLock<*mut c_void>`：裸指针不是 `Sync`，
/// 而 static 要 `Sync`，这样配就不用为它手写 `unsafe impl`。
static CLASS_LOADER: AtomicPtr<c_void> = AtomicPtr::new(null_mut());
static CLASS_LOADER_ONCE: Once = Once::new();

/// 取 Application ClassLoader。
///
/// 必须我们自己给：`androidmedia` 的 `initialize_classes()` 唯一做的事就是
/// `g_module_open(NULL)` + 按名字查这个符号（gstjniutils.c:592 的
/// `check_application_class_loader`），查不到就 `gst_amc_jni_initialize()` 返回
/// FALSE，`plugin_init` 整块失败。core 里那份同名实现（gstreamer
/// subprojects/gstreamer/gst/gstandroid.c:411）在官方 Android 包里靠 Java 侧
/// `GSTAndroid.init(context)` 喂 `_class_loader`，本工程是纯 NativeActivity、
/// APK 里没有 `.dex`，那个入口永远不会被调，而且 `gstandroid.c.o` 压根没参与
/// 链接（实测 `llvm-nm --dynamic` 里查无此符号，装机日志里 `dlsym` 拿到 0x0）。
/// 返回的 global ref 与 core 那份语义一致：调用方 `gst_amc_jni_get_application_class`
/// 用完不 unref（gstjniutils.c:853 的 `done:` 只 unref 局部量），所以这里故意
/// 不回收，进程活着它就一直活着。
#[unsafe(no_mangle)]
pub extern "C" fn gst_android_get_application_class_loader() -> *mut c_void {
    CLASS_LOADER_ONCE.call_once(|| {
        CLASS_LOADER.store(resolve_application_class_loader(), Ordering::Relaxed);
    });
    CLASS_LOADER.load(Ordering::Relaxed)
}

fn resolve_application_class_loader() -> *mut c_void {
    let (vm_ptr, activity_ptr) = (
        JAVA_VM.load(Ordering::Relaxed),
        ACTIVITY.load(Ordering::Relaxed),
    );
    if vm_ptr.is_null() || activity_ptr.is_null() {
        log::error!("取 Application ClassLoader 失败：JavaVM/Activity 还没交进来");
        return null_mut();
    }
    let vm = unsafe { jni::JavaVM::from_raw(vm_ptr.cast()) };
    let resolved = vm.attach_current_thread(|env| -> jni::errors::Result<*mut c_void> {
        // SAFETY：activity_ptr 是 android-activity 持有且不交给我们的 global ref，
        // 这里只读它、不包成 Global/Auto，因此不会去删它。
        let activity = unsafe { jni::objects::JObject::from_raw(env, activity_ptr.cast()) };
        let loader = env
            .call_method(
                &activity,
                jni::jni_str!("getClassLoader"),
                jni::jni_sig!("()Ljava/lang/ClassLoader;"),
                &[],
            )?
            .l()?;
        // into_raw 而不是留着 Global：Global 一 Drop 就 DeleteGlobalRef，
        // 这个引用要活得比本次调用长。
        Ok(env.new_global_ref(&loader)?.into_raw().cast())
    });
    match resolved {
        Ok(ptr) => {
            log::info!("Application ClassLoader 已备好");
            ptr
        }
        Err(e) => {
            log::error!("取 Application ClassLoader 失败：{e:?}");
            null_mut()
        }
    }
}

/// 初始化 GStreamer 并登记全部静态插件。`java_vm` 与 `activity` 取自
/// `AndroidApp::vm_as_ptr()` / `activity_as_ptr()`。
///
/// `aweft-gui` 那边建管线时还会再 `gst::init()` 一次：`gst_init` 内部有
/// `gst_initialized` 守卫，重复调用是空操作，不会重置 registry。
pub(crate) fn init_and_register(java_vm: *mut c_void, activity: *mut c_void) -> anyhow::Result<()> {
    gstreamer::init().context("GStreamer 初始化失败")?;
    JAVA_VM.store(java_vm, Ordering::Relaxed);
    ACTIVITY.store(activity, Ordering::Relaxed);
    // 必须在 register_all 之前：plugin_init 就在这批注册里跑。
    // SAFETY：指针由 android-activity 保证是有效的 JavaVM，且本函数只读存它。
    unsafe { gst_amc_jni_set_java_vm(java_vm) };
    // 提前把 ClassLoader 解析好：解析只做一次，成败能在启动日志里看到，
    // 不必等到某个元素在流线程里查类时才暴露。
    gst_android_get_application_class_loader();
    register_all();
    report_missing_plugins();
    Ok(())
}

/// 注册函数本身返回 `void`（`GST_PLUGIN_DEFINE` 把
/// `gst_plugin_register_static` 的 `gboolean` 丢掉了），问不出单个插件的成败，
/// 只能事后从默认 registry 里按名字回查。少了哪个就报哪个——版本号不匹配、
/// license 字段不合法都会走到这里。
///
/// 注意别用 feature 列表查插件名：`gst_registry_add_plugin` 只把插件挂进
/// `registry->plugins`，插件对象本身不是 feature，按插件名 `lookup_feature`
/// 恒为 None。
fn report_missing_plugins() {
    let registry = gstreamer::Registry::get();
    let missing: Vec<&str> = PLUGIN_NAMES
        .iter()
        .copied()
        .filter(|name| registry.features_by_plugin(name).is_empty())
        .collect();
    if missing.is_empty() {
        log::info!(
            "GStreamer 静态插件全部登记成功（{} 个）",
            PLUGIN_NAMES.len()
        );
    } else {
        log::warn!("GStreamer 静态插件登记缺失：{missing:?}");
    }
}
