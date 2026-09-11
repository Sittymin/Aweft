//! Android 文件选择：MediaStore（系统原生媒体索引）+ ContentResolver。
//!
//! 为什么不用 SAF 的 `ACTION_OPEN_DOCUMENT`：Slint 用的 android-activity 0.6
//! 没有回执通道（`PollEvent` 只有 Wake/Timeout/Main，registry 源码确认无
//! onActivityResult），startActivityForResult 发出去收不回来。本模块只用
//! 不需要回执的原生能力：读权限状态可轮询（`checkSelfPermission`），媒体列表
//! 可直接查，文件内容可直接拷。选择界面用 Slint 自家组件画（数据是系统的，
//! 皮是我们的）。
//!
//! 线程模型：所有 JNI 阻塞操作（权限轮询、查库、拷文件）跑在专用工作线程，
//! 经 oneshot 回到 `spawn_local` 的异步上下文，不卡 UI。

use std::cell::RefCell;
use std::ffi::c_void;
use std::path::PathBuf;
use std::ptr::null_mut;
use std::rc::Rc;
use std::sync::atomic::{AtomicPtr, Ordering};

use jni::objects::{JObject, JString, JValue};
use jni::sys::jlong;
use jni::JNIEnv;

/// 媒体库里的一条视频（跨线程传递，只含 Rust 自有数据）。
#[derive(Clone, Debug)]
pub struct VideoItem {
    pub name: String,
    pub duration_ms: i64,
    pub size_bytes: i64,
    /// `content://` 地址。GStreamer 读不了它，用 `cache_video` 落盘后再播。
    pub uri: String,
}

impl VideoItem {
    /// 列表行文案：名、时长、大小一行写完，Slint 侧直接显示不用再拼。
    pub fn display_line(&self) -> String {
        let secs = self.duration_ms.max(0) / 1000;
        let mb = self.size_bytes.max(0) as f64 / 1048576.0;
        format!("{} · {:02}:{:02} · {:.1}MB", self.name, secs / 60, secs % 60, mb)
    }
}

/// `requestPermissions` 的请求码。android-activity 0.6 没有回执通道
/// （`onRequestPermissionsResult` 收不到），这个码只是 Java 侧的必填参数，
/// 我们不读它——授权结果靠轮询 `checkSelfPermission` 拿。
const REQUEST_PERMISSIONS_CODE: i32 = 1001;

/// 等授权结果的轮询次数与间隔：合起来 120 秒，理由见 `wait_for_permission`。
const PERMISSION_POLLS: u32 = 240;
const PERMISSION_POLL_INTERVAL: std::time::Duration = std::time::Duration::from_millis(500);

/// 读媒体库要过的权限。API 33 起读写分离成 READ_MEDIA_*，之前是 READ_EXTERNAL_STORAGE。
fn required_permission(sdk: i32) -> &'static str {
    if sdk >= 33 {
        "android.permission.READ_MEDIA_VIDEO"
    } else {
        "android.permission.READ_EXTERNAL_STORAGE"
    }
}

/// 取 JVM 与应用上下文。注意 `ndk_context` 存的那份是 **Application**
/// （android-activity 0.6.1 的 `init.rs:285` 交进去的是 `app_global`），
/// 不是 Activity。`ContentResolver`、`checkSelfPermission` 这些 `Context`
/// 上的方法用它没问题；`requestPermissions` 只在 Activity 上有，
/// 见 [`request_video_permission`]。
fn android_ctx() -> Option<(jni::JavaVM, JObject<'static>)> {
    let ctx = ndk_context::android_context();
    // SAFETY：指针由 android-activity 在 android_main 入口提供，全程有效；
    // 'static 是对指针存续的断言，实际使用限于本次调用。
    let vm = unsafe { jni::JavaVM::from_raw(ctx.vm() as *mut _).ok()? };
    let context = unsafe { JObject::from_raw(ctx.context() as jni::sys::jobject) };
    Some((vm, context))
}

/// Activity 的 JNI global ref，由 `aweft-android` 的 `android_main` 交进来
/// （见 `set_activity`）。持有它是 `android_main` 那个 `AndroidApp`，被
/// `slint::android::init` 收进 platform 全局量，随进程存活。
static ACTIVITY: AtomicPtr<c_void> = AtomicPtr::new(null_mut());

/// 交入 Activity 句柄。`requestPermissions` 声明在 `Activity` 上，`Application`
/// 没有这个方法，拿 `ndk_context` 那份去调只会失败。
///
/// 真机第一次跑到这里只报出 `JavaException` 四个字，消息被 jni 的 Debug 实现吞了，
/// `describe_exception` 就是为补这一环加的；那一次的报错内容本身还没重读过。
///
/// 存续范围＝`android_main` 拿到的那个 Activity 实例。配置变化（旋转）导致
/// Activity 重建时这份会随旧实例失效，届时要在 `MainEvent::Create` 重新交入；
/// 当前重建路径本来就未接线，见 `docs/tmp/android-port.md`。
pub fn set_activity(activity: *mut c_void) {
    ACTIVITY.store(activity, Ordering::Relaxed);
}

/// 取 Activity 句柄。存的是 global ref，不随某个 JNI 栈帧失效，所以能在任意
/// 已附加的线程上用，类型给 `'static`。
fn activity_of() -> Option<JObject<'static>> {
    let ptr = ACTIVITY.load(Ordering::Relaxed);
    if ptr.is_null() {
        return None;
    }
    // SAFETY：`set_activity` 交进来的是 android-activity 全程持有的有效 global ref。
    Some(unsafe { JObject::from_raw(ptr as jni::sys::jobject) })
}

/// 把当前挂起的 Java 异常转成一行人话，顺手清掉它。`jni` 的 `Error::JavaException`
/// Debug 只出枚举名，看不到消息，真机排错全靠这里。取不到消息也返回占位文本。
fn describe_exception(env: &mut JNIEnv) -> String {
    let Ok(throwable) = env.exception_occurred() else {
        return "<无挂起异常>".to_string();
    };
    env.exception_clear().ok();
    let described = env
        .call_method(&throwable, "toString", "()Ljava/lang/String;", &[])
        .and_then(|v| v.l());
    let Ok(obj) = described else {
        return "<异常对象读不出>".to_string();
    };
    let text = JString::from(obj);
    match env.get_string(&text) {
        Ok(s) => s.into(),
        Err(_) => "<异常消息读不出>".to_string(),
    }
}

/// JNI 调用失败时的一行日志：错误本身 + 那一刻挂起的 Java 异常。
fn jni_fail(what: &str, e: jni::errors::Error, env: &mut JNIEnv) -> String {
    let pending = describe_exception(env);
    format!("{what}：{e:?}（Java 侧：{pending}）")
}

fn sdk_int(env: &mut JNIEnv) -> jni::errors::Result<i32> {
    let version = env.find_class("android/os/Build$VERSION")?;
    env.get_static_field(version, "SDK_INT", "I")?.i()
}

fn permission_granted(env: &mut JNIEnv, context: &JObject, perm: &str) -> jni::errors::Result<bool> {
    let perm = env.new_string(perm)?;
    let code = env
        .call_method(
            context,
            "checkSelfPermission",
            "(Ljava/lang/String;)I",
            &[JValue::from(&JObject::from(perm))],
        )?
        .i()?;
    Ok(code == 0) // PackageManager.PERMISSION_GRANTED
}

/// 缺权限时发起系统授权弹框；已有权限就什么都不做。
///
/// 放在 Slint 事件循环线程上调：这条线就是 `android_main` 所在的 Android UI 线程，
/// 弹框本身不阻塞（结果靠 `wait_for_permission` 在工作线程轮询），也没必要占用
/// 工作线程。`requestPermissions` 对调用线程有没有要求还没单独验证过——真机第一次
/// 失败的已证原因是收信对象是 Application，不是线程。
pub fn request_video_permission() -> Result<(), String> {
    let Some((vm, context)) = android_ctx() else {
        return Err("拿不到 JVM/应用上下文（ndk_context 未初始化）".to_string());
    };
    let mut guard = vm
        .attach_current_thread()
        .map_err(|e| format!("attach_current_thread 失败：{e:?}"))?;
    let env = &mut *guard;
    let sdk = sdk_int(env).map_err(|e| jni_fail("读 SDK_INT", e, env))?;
    let perm = required_permission(sdk);
    if permission_granted(env, &context, perm).map_err(|e| jni_fail("查权限", e, env))? {
        return Ok(());
    }
    let activity = activity_of().ok_or("未登记 Activity 句柄（android_main 没调 set_activity）")?;
    let perm_obj = JObject::from(env.new_string(perm).map_err(|e| jni_fail("建权限字符串", e, env))?);
    let str_class = env.find_class("java/lang/String").map_err(|e| jni_fail("找 String 类", e, env))?;
    let arr = env
        .new_object_array(1, str_class, JObject::null())
        .map_err(|e| jni_fail("建权限数组", e, env))?;
    env.set_object_array_element(&arr, 0, perm_obj)
        .map_err(|e| jni_fail("填权限数组", e, env))?;
    env.call_method(
        &activity,
        "requestPermissions",
        "([Ljava/lang/String;I)V",
        &[JValue::from(&JObject::from(arr)), JValue::from(REQUEST_PERMISSIONS_CODE)],
    )
    .map_err(|e| jni_fail("发起授权弹框", e, env))?
    .v()
    .map_err(|e| jni_fail("发起授权弹框", e, env))?;
    Ok(())
}

/// 等授权落地：轮询 `checkSelfPermission`，最多 `PERMISSION_POLLS × PERMISSION_POLL_INTERVAL`。
/// 用户拒绝或超时返回 false，调用方直接收手（不断言、不崩）。
///
/// 窗口给到 120 秒是实测定的：20 秒时人还没答完弹框就超时了（权限随后确实授予成功，
/// 但这边已经报"用户未授予媒体读取权限"）。拒绝这一侧收不了尾——没有回执通道
/// （见 `docs/tmp/android-port.md` "已验证"第 27 条），区分不出"弹框还开着"与"已拒绝"。
fn wait_for_permission(env: &mut JNIEnv, context: &JObject) -> jni::errors::Result<bool> {
    let perm = required_permission(sdk_int(env)?);
    for _ in 0..PERMISSION_POLLS {
        if permission_granted(env, context, perm)? {
            return Ok(true);
        }
        std::thread::sleep(PERMISSION_POLL_INTERVAL);
    }
    Ok(false)
}

fn column_index(env: &mut JNIEnv, cursor: &JObject, name: &str) -> jni::errors::Result<i32> {
    let col = JObject::from(env.new_string(name)?);
    env.call_method(cursor, "getColumnIndex", "(Ljava/lang/String;)I", &[JValue::from(&col)])?
        .i()
}

/// 查媒体库，返回按修改时间倒序的视频（上限 300 条）。
fn query_videos(env: &mut JNIEnv, context: &JObject) -> jni::errors::Result<Vec<VideoItem>> {
    let resolver = env
        .call_method(context, "getContentResolver", "()Landroid/content/ContentResolver;", &[])?
        .l()?;
    let media = env.find_class("android/provider/MediaStore$Video$Media")?;
    let base_uri = env
        .get_static_field(media, "EXTERNAL_CONTENT_URI", "Landroid/net/Uri;")?
        .l()?;

    let str_class = env.find_class("java/lang/String")?;
    let cols = ["_id", "_display_name", "duration", "_size"];
    let projection = env.new_object_array(cols.len() as i32, str_class, JObject::null())?;
    for (i, col) in cols.iter().enumerate() {
        let s = JObject::from(env.new_string(col)?);
        env.set_object_array_element(&projection, i as i32, s)?;
    }
    let sort = JObject::from(env.new_string("date_modified DESC")?);
    let null = JObject::null();
    let cursor = env
        .call_method(
            &resolver,
            "query",
            "(Landroid/net/Uri;[Ljava/lang/String;Ljava/lang/String;[Ljava/lang/String;Ljava/lang/String;)Landroid/database/Cursor;",
            &[
                JValue::from(&base_uri),
                JValue::from(&JObject::from(projection)),
                JValue::Object(&null),
                JValue::Object(&null),
                JValue::from(&sort),
            ],
        )?
        .l()?;
    if cursor.is_null() {
        return Ok(Vec::new());
    }

    let id_col = column_index(env, &cursor, "_id")?;
    let name_col = column_index(env, &cursor, "_display_name")?;
    let dur_col = column_index(env, &cursor, "duration")?;
    let size_col = column_index(env, &cursor, "_size")?;

    let mut items = Vec::new();
    for _ in 0..300 {
        // 每行包一层局部引用帧：列对象不断产生，不清会撑爆 JNI 局部引用表。
        let more: bool = env.with_local_frame(32, |env| -> jni::errors::Result<bool> {
            let has = env.call_method(&cursor, "moveToNext", "()Z", &[])?.z()?;
            if !has {
                return Ok(false);
            }
            let id: jlong = env.call_method(&cursor, "getLong", "(I)J", &[JValue::from(id_col)])?.j()?;
            let name_obj = env
                .call_method(&cursor, "getString", "(I)Ljava/lang/String;", &[JValue::from(name_col)])?
                .l()?;
            let name_js = JString::from(name_obj);
            let name: String = env.get_string(&name_js)?.into();
            let duration_ms: jlong =
                env.call_method(&cursor, "getLong", "(I)J", &[JValue::from(dur_col)])?.j()?;
            let size_bytes: jlong =
                env.call_method(&cursor, "getLong", "(I)J", &[JValue::from(size_col)])?.j()?;
            let uri_obj = env
                .call_static_method(
                    "android/content/ContentUris",
                    "withAppendedId",
                    "(Landroid/net/Uri;J)Landroid/net/Uri;",
                    &[JValue::from(&base_uri), JValue::from(id)],
                )?
                .l()?;
            let uri_str = env.call_method(&uri_obj, "toString", "()Ljava/lang/String;", &[])?.l()?;
            let uri_js = JString::from(uri_str);
            let uri: String = env.get_string(&uri_js)?.into();
            items.push(VideoItem { name, duration_ms, size_bytes, uri });
            Ok(true)
        })?;
        if !more {
            break;
        }
    }
    env.call_method(&cursor, "close", "()V", &[])?.v()?;
    Ok(items)
}

/// 把 `content://` 拷到应用私有缓存，返回本地路径（下游与桌面走同一条路）。
fn copy_to_cache(env: &mut JNIEnv, context: &JObject, item: &VideoItem) -> jni::errors::Result<PathBuf> {
    let resolver = env
        .call_method(context, "getContentResolver", "()Landroid/content/ContentResolver;", &[])?
        .l()?;
    let uri_str = JObject::from(env.new_string(&item.uri)?);
    let uri_obj = env
        .call_static_method(
            "android/net/Uri",
            "parse",
            "(Ljava/lang/String;)Landroid/net/Uri;",
            &[JValue::from(&uri_str)],
        )?
        .l()?;
    let stream = env
        .call_method(
            &resolver,
            "openInputStream",
            "(Landroid/net/Uri;)Ljava/io/InputStream;",
            &[JValue::from(&uri_obj)],
        )?
        .l()?;
    if stream.is_null() {
        return Err(jni::errors::Error::NullPtr("openInputStream 返回空"));
    }

    let cache_dir = env.call_method(context, "getCacheDir", "()Ljava/io/File;", &[])?.l()?;
    let sub = JObject::from(env.new_string("aweft-pick")?);
    let sub_dir = env.new_object(
        "java/io/File",
        "(Ljava/io/File;Ljava/lang/String;)V",
        &[JValue::from(&cache_dir), JValue::from(&sub)],
    )?;
    env.call_method(&sub_dir, "mkdirs", "()Z", &[])?.z()?;
    // 文件名取媒体库原名（非法字符由系统侧保证合法），避免二次编码。
    let file_name = JObject::from(env.new_string(&item.name)?);
    let file = env.new_object(
        "java/io/File",
        "(Ljava/io/File;Ljava/lang/String;)V",
        &[JValue::from(&sub_dir), JValue::from(&file_name)],
    )?;
    let fos = env.new_object("java/io/FileOutputStream", "(Ljava/io/File;)V", &[JValue::from(&file)])?;
    let path_obj = env.call_method(&file, "getAbsolutePath", "()Ljava/lang/String;", &[])?.l()?;
    let path_js = JString::from(path_obj);
    let path: String = env.get_string(&path_js)?.into();

    // 一个 Java 侧 byte[] 从头用到尾：每轮新建（读缓冲 + 再拷一份写缓冲）且不出
    // 局部引用帧，会让 GC 根一直攥着历轮数组，153MB 的文件实测把 256MB 的 Java 堆
    // 占满（OutOfMemoryError: Failed to allocate a 65552 byte allocation）。
    // 就地按读到的长度写，也不必要再往 Rust 侧整块拷一遍。
    let buf = env.new_byte_array(65536)?;
    let buf_obj = JObject::from(buf);
    loop {
        let n: i32 = env
            .call_method(&stream, "read", "([B)I", &[JValue::from(&buf_obj)])?
            .i()?;
        if n <= 0 {
            break;
        }
        env.call_method(
            &fos,
            "write",
            "([BII)V",
            &[JValue::from(&buf_obj), JValue::from(0), JValue::from(n)],
        )?
        .v()?;
    }
    env.call_method(&fos, "close", "()V", &[])?.v()?;
    env.call_method(&stream, "close", "()V", &[])?.v()?;
    Ok(PathBuf::from(path))
}

/// 阻塞版查库（跑在工作线程）：等授权落地 → 查库。出错一律转 `None`，只记日志。
/// 授权弹框不在这里发起，见 [`request_video_permission`]。
fn blocking_query() -> Option<Vec<VideoItem>> {
    let (vm, context) = android_ctx()?;
    let mut guard = vm.attach_current_thread().ok()?;
    let env: &mut JNIEnv = &mut guard;
    match wait_for_permission(env, &context) {
        Ok(true) => (),
        Ok(false) => {
            log::warn!("打开视频：用户未授予媒体读取权限");
            return None;
        }
        Err(e) => {
            log::warn!("{}", jni_fail("打开视频：等授权失败", e, env));
            return None;
        }
    }
    match query_videos(env, &context) {
        Ok(items) => Some(items),
        Err(e) => {
            log::warn!("{}", jni_fail("打开视频：查媒体库失败", e, env));
            None
        }
    }
}

/// 阻塞版拷文件（跑在工作线程）。
fn blocking_cache(item: &VideoItem) -> Option<PathBuf> {
    let (vm, context) = android_ctx()?;
    let mut guard = vm.attach_current_thread().ok()?;
    let env: &mut JNIEnv = &mut guard;
    match copy_to_cache(env, &context, item) {
        Ok(path) => Some(path),
        Err(e) => {
            log::warn!("{}", jni_fail("打开视频：拷缓存失败", e, env));
            None
        }
    }
}

/// 对话框的一次等待：条目（下标与 Slint 行一一对应）+ 挑选动作 + 回执通道。
/// 存在 `AppStore.pending_media_pick`，由选择回调取走后一次性完成。
pub struct PendingPick {
    pub items: Vec<VideoItem>,
    pub kind: PickKind,
    pub reply: futures::channel::oneshot::Sender<Option<usize>>,
}

/// 挑中后的动作（与桌面两处调用一一对应：追加 / 原地更换）。
#[derive(Clone, Copy)]
pub enum PickKind {
    Append,
    Replace(usize),
}

/// 查媒体库（异步外壳，工作线程跑阻塞调用）。
pub async fn query_recent_videos() -> Vec<VideoItem> {
    let (tx, rx) = futures::channel::oneshot::channel();
    std::thread::spawn(move || {
        let _ = tx.send(blocking_query().unwrap_or_default());
    });
    rx.await.unwrap_or_default()
}

/// 选中后拷缓存（异步外壳）。
pub async fn cache_video(item: &VideoItem) -> Option<PathBuf> {
    let (tx, rx) = futures::channel::oneshot::channel();
    let owned = item.clone();
    std::thread::spawn(move || {
        let _ = tx.send(blocking_cache(&owned));
    });
    rx.await.ok()?
}

/// 对话框编排（callbacks 的 Android 分支调它）：查库 → 弹层 → 等选择 → 拷盘 → dispatch。
///
/// 与桌面两处调用语义一致：`kind` 决定追加还是原地更换。空库、取消、拷盘失败
/// 都直接返回（上层无事可做）；弹层无论何种退出都要收起。
pub(crate) async fn run_android_picker(
    weak: slint::Weak<crate::bridge::types::MainWindow>,
    store: Rc<RefCell<crate::store::AppStore>>,
    kind: PickKind,
) {
    use crate::actions::media::{MediaAction, VideoAction};
    use crate::actions::Action;

    // 授权弹框必须在 UI 线程发（见 request_video_permission），本函数正是跑在
    // Slint 事件循环线程上；弹完立刻继续，等结果由工作线程轮询。
    if let Err(e) = request_video_permission() {
        log::warn!("打开视频：{e}");
        return;
    }
    let items = query_recent_videos().await;
    if items.is_empty() {
        log::info!("打开视频：媒体库为空或未授权");
        return;
    }
    let (tx, rx) = futures::channel::oneshot::channel();
    let lines: Vec<slint::SharedString> =
        items.iter().map(|item| item.display_line().into()).collect();
    store.borrow_mut().pending_media_pick = Some(PendingPick {
        items: items.clone(),
        kind,
        reply: tx,
    });
    let Some(instance) = weak.upgrade() else {
        store.borrow_mut().pending_media_pick = None;
        return;
    };
    {
        use slint::ComponentHandle;
        let picker = instance.global::<crate::bridge::types::MediaPickerState>();
        picker.set_items(slint::ModelRc::from(Rc::new(slint::VecModel::from(lines))));
        picker.set_visible(true);
    }
    let choice = rx.await.ok().flatten();
    if let Some(instance) = weak.upgrade() {
        use slint::ComponentHandle;
        instance
            .global::<crate::bridge::types::MediaPickerState>()
            .set_visible(false);
    }
    let Some(index) = choice else { return };
    let Some(item) = items.get(index) else { return };
    let Some(path) = cache_video(item).await else { return };
    let Ok(uri) = url::Url::from_file_path(&path).map(|u| u.to_string()) else {
        log::warn!("打开视频：缓存路径无法转换为 URI：{}", path.display());
        return;
    };
    let action = Action::Media(MediaAction::Video(match kind {
        PickKind::Append => VideoAction::Append { uri },
        PickKind::Replace(index) => VideoAction::Replace { index, uri },
    }));
    crate::bridge::callbacks::dispatch_and_render(&store, action, &weak);
}
