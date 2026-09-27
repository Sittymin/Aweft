//! 预览时钟：没有视频会话时，播放按钮靠它走时间。
//!
//! 视频在场时播放头由媒体时钟驱动（appsink 回调 → 请求重绘 → GStreamer 位置查询，
//! 见 `docs/architecture/video-playback.md`）。没有视频就没有媒体时钟，也没有任何东西
//! 会唤醒 Slint 的事件循环，因此这里放一个 repeated [`slint::Timer`]：仅在「播放中且
//! 无视频会话」时活着，两条任一不成立就自己停表。
//!
//! 起停不写在 reducer 里：`video_playing` 是权威状态，投影 `ui_sync_video_playback_scalar`
//! 既已负责把它镜像给按钮图标，也顺手决定心跳该不该跑。
//!
//! 心跳只做一件事：把动画域当前时间按墙钟往前推，写入走既有的 `SetCurrentTime`
//! reducer —— 与手动拖播放头同一条路，同一份投影、同一次字幕出图。播放速度按真实
//! 时间累加，与心跳间隔无关；间隔只决定播放头与画面的刷新密度。

use std::cell::{Cell, RefCell};
use std::rc::{Rc, Weak};
use std::time::{Duration, Instant};

use crate::actions::Action;
use crate::actions::animation::{AnimationAction, graph::GraphAction};
use crate::bridge::MainWindow;
use crate::bridge::callbacks::dispatch_and_render;
use crate::store::AppStore;

/// 心跳间隔。播放速度与它无关（推进量按真实时间累加），它只决定刷新密度。
const TICK: Duration = Duration::from_millis(16);

pub(crate) struct PreviewClock {
    timer: slint::Timer,
    /// 上一次心跳的时刻。`None` 表示刚起步：那一跳没有基准，不推进。
    last_tick: Cell<Option<Instant>>,
}

impl Default for PreviewClock {
    fn default() -> Self {
        Self {
            timer: slint::Timer::default(),
            last_tick: Cell::default(),
        }
    }
}

impl PreviewClock {
    /// 登记心跳回调，登记完立即停表（何时起步由 `set_running` 决定）。
    ///
    /// 必须在 Slint UI 线程上调用一次。闭包抓 store 的**弱**引用：时钟本身住在
    /// `AppState` 里，抓强引用会构成循环。
    pub(crate) fn install(
        &self,
        store: &Weak<RefCell<AppStore>>,
        instance: &slint::Weak<MainWindow>,
    ) {
        let (store, instance) = (store.clone(), instance.clone());
        self.timer.start(slint::TimerMode::Repeated, TICK, move || {
            let Some(store) = store.upgrade() else {
                return;
            };
            let Some(time) = preview_tick(&store) else {
                return;
            };
            dispatch_and_render(
                &store,
                Action::Animation(AnimationAction::Graph(GraphAction::SetCurrentTime { time })),
                &instance,
            );
        });
        self.timer.stop();
    }

    /// 起停。播放且无视频会话时才有必要跑；有视频时媒体时钟是时间权威。
    pub(crate) fn set_running(&self, running: bool) {
        if self.timer.running() == running {
            return;
        }
        self.last_tick.set(None);
        if running {
            self.timer.restart();
        } else {
            self.timer.stop();
        }
    }

    /// 记录本次心跳，返回距上一次的时长；起步那一跳没有基准，返回 `None`。
    fn advance(&self, now: Instant) -> Option<Duration> {
        self.last_tick
            .replace(Some(now))
            .map(|previous| now - previous)
    }
}

/// 一次心跳：该推进就返回新的播放头时间，否则 `None`（并顺手停表）。
fn preview_tick(store: &Rc<RefCell<AppStore>>) -> Option<f32> {
    let store = store.borrow();
    let state = store.state();
    // 已暂停，或视频会话已能提供媒体时钟：停表，本心跳是最后一次。
    if !state.video_playing || !state.video_sessions.is_empty() {
        state.preview_clock.set_running(false);
        return None;
    }
    // 时间条拖拽中：目标位置由用户的手决定，不抢。
    if state.video_seek_session.is_some() {
        return None;
    }
    let elapsed = state.preview_clock.advance(Instant::now())?;
    Some((store.ui_models.animation.graph.current_time + elapsed.as_secs_f64()) as f32)
}
