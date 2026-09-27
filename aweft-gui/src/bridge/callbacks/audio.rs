use crate::bridge::types::MainWindow;

use crate::store::AppStore;
// winit 事件只在桌面后端存在：Android 特性集里 winit 后端被彻底排除
//（见 aweft-gui/Cargo.toml），slint::winit_030 模块在该目标上不存在。
use slint::ComponentHandle;
use std::{cell::RefCell, rc::Rc};

use super::dispatch_and_render;

/// 设置音频接线：绑定 AudioState 音轨模型，并启动波形批次的 UI 线程 pump 任务。
/// 波形批次属于实时渲染增量例外，直接行级更新持久 VecModel（见
/// `docs/architecture/video-playback.md`），不走 reducer/depsgraph。
pub(super) fn setup_audio_callbacks(instance: &MainWindow, store: Rc<RefCell<AppStore>>) {
    // 一次性绑定 Rust 持有的音轨模型到 AudioState 全局（共享 inner，后续自动通知）
    {
        let s = store.borrow();
        let astate = instance.global::<crate::bridge::types::AudioState>();
        astate.set_audio_tracks(slint::ModelRc::from(s.ui_models.audio.tracks.clone()));
        astate.set_playback_tracks(slint::ModelRc::from(
            s.ui_models.audio.playback_tracks.clone(),
        ));
    }

    // 波形后台 worker 经通道发批次，pump 任务在 UI 线程写模型（见
    // `store::audio::ui_pump_waveform_batches`）。发送端存入 AppState 供任务入队时克隆。
    {
        let (ui_sender, ui_receiver) =
            futures::channel::mpsc::unbounded::<crate::store::audio::WaveformBatch>();
        let _ = slint::spawn_local(crate::store::audio::ui_pump_waveform_batches(
            Rc::clone(&store),
            ui_receiver,
        ));
        store.borrow_mut().state_mut().waveform_ui_sender = Some(ui_sender);
    }

    // 视图上报：Slint 绑定按导航求值，返回该 Area 的持久列模型（标识恒定，
    // Slint 侧不重建槽位）。副作用是登记视图并同步列，pump 据此重算新批次。
    {
        use crate::bridge::types::{AudioCallbacks, SlintKey};
        use crate::screen::area::ScrAreaKey;
        use crate::store::audio::AreaWaveformView;
        let s = Rc::clone(&store);
        instance.global::<AudioCallbacks>().on_waveform_view(
            move |area_key: SlintKey,
                  video_index: i32,
                  track_index: i32,
                  bucket_ms: f32,
                  first: i32,
                  count: i32| {
                if video_index < 0 || track_index < 0 || count <= 0 {
                    return slint::ModelRc::from(Rc::new(slint::VecModel::default()));
                }
                let key: ScrAreaKey = crate::bridge::types::slint_area_key_to_key(&area_key);
                let view = AreaWaveformView {
                    video_index: video_index as usize,
                    track_index: track_index as usize,
                    bucket_ms: (bucket_ms * 1_000.0).round().max(1.0) as usize,
                    first,
                    count: count.max(0) as usize,
                };
                let mut store = s.borrow_mut();
                slint::ModelRc::from(crate::store::audio::ui_sync_waveform_columns(
                    &mut store.ui_models.audio,
                    key,
                    view,
                ))
            },
        );
    }

    {
        use crate::actions::media::{MediaAction, VideoAction};
        use crate::bridge::types::AudioCallbacks;
        let weak = instance.as_weak();
        let s = Rc::clone(&store);
        instance.global::<AudioCallbacks>().on_select_audio_track(
            move |video_index, track_index| {
                if video_index < 0 || track_index < 0 {
                    return;
                }
                dispatch_and_render(
                    &s,
                    crate::actions::Action::Media(MediaAction::Video(
                        VideoAction::SelectAudioTrack {
                            video_index: video_index as usize,
                            track_index: track_index as usize,
                        },
                    )),
                    &weak,
                );
            },
        );
    }
}
