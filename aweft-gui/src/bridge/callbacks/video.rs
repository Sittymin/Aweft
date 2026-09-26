use crate::actions::Action;
use crate::bridge::types::{MainWindow, VideoCallbacks};

use crate::store::AppStore;
use slint::ComponentHandle;
use std::{cell::RefCell, rc::Rc};

use super::dispatch_and_render;

/// 设置视口回调：打开/追加/更换视频走系统文件对话框（rfd xdg-portal，异步不阻塞 UI）。
/// 选中后 dispatch 建管线（reducer 层），再连接 WGPU 输出并自动播放（渲染层）。
/// 对话框 future 在 `spawn_local` 中 await（同官方 gstreamer-player 示例的
/// 异步模式；portal 后端用 async-std 特性以脱离 tokio 上下文）。
/// 命中检测（`video-picked`/`video-context-picked`）为同步查询（点击需要立即结果）。
pub(super) fn setup_video_callbacks(instance: &MainWindow, store: Rc<RefCell<AppStore>>) {
    // 一次性绑定 Rust 持有的 VecModel 到 VideoState 全局（共享 inner，后续自动通知）
    {
        let s = store.borrow();
        let vstate = instance.global::<crate::bridge::types::VideoState>();
        vstate.set_video_frames(slint::ModelRc::from(s.ui_models.video.frames.clone()));
        vstate.set_video_widths(slint::ModelRc::from(s.ui_models.video.widths.clone()));
        vstate.set_video_heights(slint::ModelRc::from(s.ui_models.video.heights.clone()));
        vstate.set_video_positions(slint::ModelRc::from(s.ui_models.video.positions.clone()));
        vstate.set_time_bars(slint::ModelRc::from(s.ui_models.video.time_bars.clone()));
    }

    // 打开/追加视频（首个自动成为主视频）：系统文件对话框（异步不阻塞 UI）
    {
        let weak = instance.as_weak();
        let s = Rc::clone(&store);
        instance.global::<VideoCallbacks>().on_open_video(move || {
            let future = rfd::AsyncFileDialog::new()
                .add_filter("视频文件", &["mp4", "mkv", "mov", "avi", "webm", "m4v"])
                .pick_file();
            let weak = weak.clone();
            let store = Rc::clone(&s);
            let _ = slint::spawn_local(async move {
                // 用户取消选择
                let Some(file) = future.await else {
                    return;
                };
                // 本地路径 → file URI（playbin 需要 URI）
                let path = file.path().to_path_buf();
                let Ok(uri) = url::Url::from_file_path(&path).map(|u| u.to_string()) else {
                    log::warn!("打开视频：路径无法转换为 URI：{}", path.display());
                    return;
                };
                dispatch_and_render(
                    &store,
                    Action::Media(crate::actions::media::MediaAction::Video(
                        crate::actions::media::VideoAction::Append { uri },
                    )),
                    &weak,
                );
            });
        });
    }

    // 播放/暂停切换（时间线 Header 播放按钮）
    {
        let weak = instance.as_weak();
        let s = Rc::clone(&store);
        instance
            .global::<VideoCallbacks>()
            .on_toggle_playback(move || {
                dispatch_and_render(
                    &s,
                    Action::Media(crate::actions::media::MediaAction::Video(
                        crate::actions::media::VideoAction::TogglePlayback,
                    )),
                    &weak,
                );
            });
    }

    // 点击视口视频 / 控制时间条：选中该视频（-1 = 取消选中）
    {
        let weak = instance.as_weak();
        let s = Rc::clone(&store);
        instance
            .global::<VideoCallbacks>()
            .on_select_video(move |index, modifier| {
                dispatch_and_render(
                    &s,
                    Action::Media(crate::actions::media::MediaAction::Video(
                        crate::actions::media::VideoAction::Select {
                            index: (index >= 0).then_some(index as usize),
                            modifier,
                        },
                    )),
                    &weak,
                );
            });
    }

    // 控制时间条按下：先选中该视频（随后拖拽 seek 即作用到它）
    {
        let weak = instance.as_weak();
        let s = Rc::clone(&store);
        instance
            .global::<VideoCallbacks>()
            .on_time_bar_pressed(move |index, is_batch| {
                dispatch_and_render(
                    &s,
                    Action::Media(crate::actions::media::MediaAction::Video(
                        crate::actions::media::VideoAction::SeekBegin {
                            index: index as usize,
                            is_batch,
                        },
                    )),
                    &weak,
                );
            });
    }

    // 控制时间条拖拽：seek 当前选中（无选中则主视频）到指定秒
    {
        let weak = instance.as_weak();
        let s = Rc::clone(&store);
        instance
            .global::<VideoCallbacks>()
            .on_time_bar_dragged(move |time| {
                dispatch_and_render(
                    &s,
                    Action::Media(crate::actions::media::MediaAction::Video(
                        crate::actions::media::VideoAction::SeekPreview {
                            timestamp: (time * 1000.0) as i64,
                        },
                    )),
                    &weak,
                );
            });
    }

    // 控制时间条拖拽结束：若拖拽前处于暂停，回到暂停（暂停预览）
    {
        let weak = instance.as_weak();
        let s = Rc::clone(&store);
        instance
            .global::<VideoCallbacks>()
            .on_time_bar_released(move |_index| {
                dispatch_and_render(
                    &s,
                    Action::Media(crate::actions::media::MediaAction::Video(
                        crate::actions::media::VideoAction::SeekCommit,
                    )),
                    &weak,
                );
            });
    }

    // 音频波形可查看任意视频的音轨；定位该音轨不能改变全局选中视频。
    {
        let weak = instance.as_weak();
        let s = Rc::clone(&store);
        instance
            .global::<VideoCallbacks>()
            .on_seek_standalone(move |index, time| {
                if index < 0 {
                    return;
                }
                dispatch_and_render(
                    &s,
                    Action::Media(crate::actions::media::MediaAction::Video(
                        crate::actions::media::VideoAction::SeekStandalone {
                            index: index as usize,
                            timestamp: (time * 1000.0) as i64,
                        },
                    )),
                    &weak,
                );
            });
    }

    // 右键非主视频"设为主视频"
    {
        let weak = instance.as_weak();
        let s = Rc::clone(&store);
        instance
            .global::<VideoCallbacks>()
            .on_set_main_video(move |index| {
                dispatch_and_render(
                    &s,
                    Action::Media(crate::actions::media::MediaAction::Video(
                        crate::actions::media::VideoAction::SetMain {
                            index: index as usize,
                        },
                    )),
                    &weak,
                );
            });
    }

    {
        let weak = instance.as_weak();
        let s = Rc::clone(&store);
        instance
            .global::<VideoCallbacks>()
            .on_toggle_mute_video(move |index| {
                if index < 0 {
                    return;
                }
                dispatch_and_render(
                    &s,
                    Action::Media(crate::actions::media::MediaAction::Video(
                        crate::actions::media::VideoAction::ToggleMute {
                            index: index as usize,
                        },
                    )),
                    &weak,
                );
            });
    }

    // 右键任意视频"更换视频"：重新选择文件原地替换
    {
        let weak = instance.as_weak();
        let s = Rc::clone(&store);
        instance
            .global::<VideoCallbacks>()
            .on_replace_video(move |index| {
                let future = rfd::AsyncFileDialog::new()
                    .add_filter("视频文件", &["mp4", "mkv", "mov", "avi", "webm", "m4v"])
                    .pick_file();
                let weak = weak.clone();
                let store = Rc::clone(&s);
                let _ = slint::spawn_local(async move {
                    let Some(file) = future.await else {
                        return;
                    };
                    let path = file.path().to_path_buf();
                    let Ok(uri) = url::Url::from_file_path(&path).map(|u| u.to_string()) else {
                        log::warn!("更换视频：路径无法转换为 URI：{}", path.display());
                        return;
                    };
                    dispatch_and_render(
                        &store,
                        Action::Media(crate::actions::media::MediaAction::Video(
                            crate::actions::media::VideoAction::Replace {
                                index: index as usize,
                                uri,
                            },
                        )),
                        &weak,
                    );
                });
            });
    }

    // 视口点击命中：重叠视频按选中顺序循环（同步查询）
    let s = Rc::clone(&store);
    instance
        .global::<VideoCallbacks>()
        .on_video_picked(move |x, y| -> i32 {
            s.borrow()
                .pick_video_at(x as f64, y as f64)
                .map(|i| i as i32)
                .unwrap_or(-1)
        });

    // 拖拽移动非主视频
    {
        let weak = instance.as_weak();
        let s = Rc::clone(&store);
        instance
            .global::<VideoCallbacks>()
            .on_video_moved(move |index, x, y| {
                dispatch_and_render(
                    &s,
                    Action::Media(crate::actions::media::MediaAction::Video(
                        crate::actions::media::VideoAction::MoveVideo {
                            index: index as usize,
                            x: x as f64,
                            y: y as f64,
                        },
                    )),
                    &weak,
                );
            });
    }

    // 视口命中：该点最上层的视频（拖拽移动起点/右键菜单目标）
    let s = Rc::clone(&store);
    instance
        .global::<VideoCallbacks>()
        .on_video_hit_top(move |x, y| -> i32 {
            s.borrow()
                .hit_top_video_at(x as f64, y as f64)
                .map(|i| i as i32)
                .unwrap_or(-1)
        });
}
