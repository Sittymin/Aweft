//! Android 系统剪贴板（JNI 调 ClipboardManager）。
//!
//! 桌面走 arboard，Android 上 arboard 的 platform 为空（见 mod.rs），
//! 这里直接调 `android.content.ClipboardManager`。

use anyhow::{anyhow, Result};
use jni::objects::{JObject, JString, JValue};
use jni::JNIEnv;

use crate::clipboard::backend::ClipboardBackend;

pub struct AndroidBackend;

impl AndroidBackend {
    fn clipboard_manager<'a, 'b>(env: &mut JNIEnv<'a>, context: &JObject<'b>) -> Result<JObject<'a>> {
        let svc = JObject::from(env.new_string("clipboard")?);
        let obj = env
            .call_method(
                context,
                "getSystemService",
                "(Ljava/lang/String;)Ljava/lang/Object;",
                &[JValue::from(&svc)],
            )?
            .l()?;
        if obj.is_null() {
            anyhow::bail!("getSystemService(clipboard) 返回空");
        }
        Ok(obj)
    }
}

impl ClipboardBackend for AndroidBackend {
    fn write_text(&mut self, text: &str) -> Result<()> {
        let ctx = ndk_context::android_context();
        let vm = unsafe { jni::JavaVM::from_raw(ctx.vm() as *mut _) }?;
        let context = unsafe { JObject::from_raw(ctx.context() as jni::sys::jobject) };
        let mut guard = vm
            .attach_current_thread()
            .map_err(|e| anyhow!("attach_current_thread 失败：{e:?}"))?;
        let env: &mut JNIEnv = &mut guard;
        let manager = Self::clipboard_manager(env, &context)?;
        let label = JObject::from(env.new_string("aweft")?);
        let content = JObject::from(env.new_string(text)?);
        let clip = env
            .call_static_method(
                "android/content/ClipData",
                "newPlainText",
                "(Ljava/lang/CharSequence;Ljava/lang/CharSequence;)Landroid/content/ClipData;",
                &[JValue::from(&label), JValue::from(&content)],
            )?
            .l()?;
        env.call_method(
            &manager,
            "setPrimaryClip",
            "(Landroid/content/ClipData;)V",
            &[JValue::from(&clip)],
        )?
        .v()?;
        Ok(())
    }

    fn read_text(&mut self) -> Result<Option<String>> {
        let ctx = ndk_context::android_context();
        let vm = unsafe { jni::JavaVM::from_raw(ctx.vm() as *mut _) }?;
        let context = unsafe { JObject::from_raw(ctx.context() as jni::sys::jobject) };
        let mut guard = vm
            .attach_current_thread()
            .map_err(|e| anyhow!("attach_current_thread 失败：{e:?}"))?;
        let env: &mut JNIEnv = &mut guard;
        let manager = Self::clipboard_manager(env, &context)?;
        let has = env
            .call_method(&manager, "hasPrimaryClip", "()Z", &[])?
            .z()?;
        if !has {
            return Ok(None);
        }
        let clip = env
            .call_method(
                &manager,
                "getPrimaryClip",
                "()Landroid/content/ClipData;",
                &[],
            )?
            .l()?;
        if clip.is_null() {
            return Ok(None);
        }
        let count = env.call_method(&clip, "getItemCount", "()I", &[])?.i()?;
        if count <= 0 {
            return Ok(None);
        }
        let item = env
            .call_method(
                &clip,
                "getItemAt",
                "(I)Landroid/content/ClipData$Item;",
                &[JValue::from(0)],
            )?
            .l()?;
        let text_obj = env
            .call_method(&item, "getText", "()Ljava/lang/CharSequence;", &[])?
            .l()?;
        if text_obj.is_null() {
            return Ok(None);
        }
        // CharSequence → String
        let text_str = env
            .call_method(&text_obj, "toString", "()Ljava/lang/String;", &[])?
            .l()?;
        let jstr = JString::from(text_str);
        let rust: String = env.get_string(&jstr)?.into();
        Ok(Some(rust))
    }
}
