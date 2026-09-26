use crate::bridge::types::MainWindow;
// 媒体选择弹层的回调只在 Android 装配（见 setup_media_picker_callbacks）。
#[cfg(target_os = "android")]
use crate::bridge::types::MediaPickerCallbacks;

use crate::store::AppStore;
use std::{cell::RefCell, rc::Rc};

/// 媒体选择弹层的选择/取消（仅 Android）：取走 AppStore 里的等待，
/// 把下标（或取消）发回给 run_android_picker 的 oneshot。弹层收起由
/// run_android_picker 在收到回执后统一做，这里只管送信。
#[cfg(target_os = "android")]
pub(super) fn setup_media_picker_callbacks(instance: &MainWindow, store: Rc<RefCell<AppStore>>) {
    let s = Rc::clone(&store);
    instance
        .global::<MediaPickerCallbacks>()
        .on_pick_media(move |index| {
            if let Some(pending) = s.borrow_mut().pending_media_pick.take() {
                let _ = pending.reply.send(Some(index.max(0) as usize));
            }
        });
    let s = Rc::clone(&store);
    instance
        .global::<MediaPickerCallbacks>()
        .on_cancel_media(move || {
            if let Some(pending) = s.borrow_mut().pending_media_pick.take() {
                let _ = pending.reply.send(None);
            }
        });
}

/// 空实现：桌面没有媒体选择弹层（走 rfd），保持 setup_callbacks 调用点无分叉。
#[cfg(not(target_os = "android"))]
pub(super) fn setup_media_picker_callbacks(_instance: &MainWindow, _store: Rc<RefCell<AppStore>>) {}
