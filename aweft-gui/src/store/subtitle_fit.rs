//! 字幕脚本拟合的后台通路：编脚本的 DP 分段跑在专用线程，UI 线程只收进度。
//!
//! 一条动画的分段是格点平方量级的求值，整份脚本在真实工程上要几百毫秒。过去它在依赖图
//! 的投影里同步跑，等于动一下字幕就把整个界面冻住。这里改成「单飞 + 合并最新」：同一
//! 时刻只允许一份拟合在跑，期间新到的请求只置一个待办标记，等手上这份跑完再用**当时
//! 最新**的数据重跑一份，遮罩在两次之间不撤。
//!
//! 后台线程拿到的是 `Project` 的副本，不回写任何真相——拟合的唯一产物是脚本文本。副本
//! 自带的求值缓存可能比真身旧，那只影响一次缓存命中，不影响结果。

use std::cell::RefCell;
use std::rc::Rc;
use std::sync::OnceLock;
use std::sync::mpsc;

use aweft_convert::ProgressEvent;
use futures::StreamExt;

use crate::store::AppStore;

/// 遮罩那张曲线图的 viewbox。必须与 `ui/screen/overlays/subtitle_fit.slint` 里的
/// `viewbox-width/-height` 一致：Slint 的 `Path` 子元素不支持 `for` 重复
/// （`compile_paths.rs` 直接报错），数据驱动的折线只能靠 `commands` 字符串喂。
const CHART_WIDTH: f64 = 100.0;
const CHART_HEIGHT: f64 = 40.0;
/// 曲线上下留白，避免极值贴边后看不出来。
const CHART_PADDING: f64 = 3.0;

/// 后台送回 UI 线程的消息。`generation` 是请求序号，用于丢弃过期消息。
#[derive(Debug)]
pub enum FitMessage {
    /// 转换进度（`ProgressDetail::Light` 那组），只用来画遮罩。
    Progress {
        generation: u64,
        event: ProgressEvent,
    },
    /// 这份拟合跑完了。`script` 为 `None` 表示生成失败，保留上一份脚本。
    Finished {
        generation: u64,
        script: Option<Vec<u8>>,
    },
}

struct FitJob {
    generation: u64,
    project: aweft_model::project::Project,
    frame_size: (u32, u32),
    ui_sender: futures::channel::mpsc::UnboundedSender<FitMessage>,
}

/// 拟合工作线程的唯一投递口（首次请求时懒启动，进程生命周期内常驻）。
static FIT_WORKER: OnceLock<mpsc::Sender<FitJob>> = OnceLock::new();

fn worker_sender() -> &'static mpsc::Sender<FitJob> {
    FIT_WORKER.get_or_init(|| {
        let (sender, receiver) = mpsc::channel::<FitJob>();
        let _ = std::thread::Builder::new()
            .name("aweft-subtitle-fit".into())
            .spawn(move || {
                // 单飞保证同时最多一份在飞，所以这里不会有排队积压。
                for job in receiver {
                    run_job(job);
                }
            });
        sender
    })
}

fn run_job(job: FitJob) {
    let FitJob {
        generation,
        mut project,
        frame_size,
        ui_sender,
    } = job;
    // `sequence_to_ass` 会占住自己这个线程直到编完，进度只能经同步通道出去，
    // 因此另起一条转发线程消费接收端。转发线程在唯一 sender 析构后自然收尾。
    let (progress_sender, progress_receiver) = mpsc::channel::<ProgressEvent>();
    let relay = {
        let ui_sender = ui_sender.clone();
        std::thread::spawn(move || {
            for event in progress_receiver {
                let _ = ui_sender.unbounded_send(FitMessage::Progress { generation, event });
            }
        })
    };
    let options = aweft_convert::ConvertOptions {
        progress_sender: Some(progress_sender),
        ..Default::default()
    };
    let script = match aweft_convert::sequence_to_ass(&mut project, frame_size, &options) {
        Ok(ass) => match ass.to_bytes() {
            Ok(bytes) => Some(bytes),
            Err(error) => {
                log::warn!("字幕脚本写进内存失败: {error}");
                None
            }
        },
        Err(error) => {
            log::warn!("字幕脚本生成失败: {error}");
            None
        }
    };
    // sender 藏在 `options` 里，丢掉它转发线程才收得到断开信号；join 一下保证
    // 最后几个进度事件排在 `Finished` 前面。
    drop(options);
    let _ = relay.join();
    let _ = ui_sender.unbounded_send(FitMessage::Finished { generation, script });
}

/// 提交一次拟合请求。遮罩立即盖上，脚本在后台线程编好后由 pump 换进字幕通路。
///
/// 门禁与同步版一致：字幕通路还没建好时不做（`handle_rendering_setup` 建好会再调一次）。
/// 通道没装（无头测试不启动 pump）时也跳过——脚本只是显示产物，没人收就别白跑一趟。
pub(crate) fn request_rebuild(store: &mut AppStore) {
    let Some(ui_sender) = store.state().fit_ui_sender.clone() else {
        return;
    };
    {
        let state = store.state_mut();
        if state.subtitle.is_none() {
            return;
        }
        if state.fit_in_flight {
            state.fit_pending = true;
            return;
        }
        state.fit_generation += 1;
        state.fit_in_flight = true;
    }
    let state = store.state_mut();
    let job = FitJob {
        generation: state.fit_generation,
        project: state.project.project().clone(),
        frame_size: state.preview_frame_size(),
        ui_sender,
    };
    if worker_sender().send(job).is_err() {
        state.fit_in_flight = false;
        log::warn!("字幕拟合线程没起来，本次刷新跳过");
        return;
    }
    show_overlay(store, default_data());
}

/// 一份拟合跑完后的收尾：换上脚本，再按待办标记决定重跑一份还是撤遮罩。
fn settle(store: &mut AppStore, generation: u64, script: Option<Vec<u8>>) {
    let pending;
    {
        let state = store.state_mut();
        if state.fit_generation != generation {
            return;
        }
        state.fit_in_flight = false;
        pending = std::mem::take(&mut state.fit_pending);
        if let (Some(bytes), Some(pipeline)) = (script, state.subtitle.as_mut()) {
            pipeline.set_script(bytes);
        }
    }
    if pending {
        request_rebuild(store);
    } else {
        set_overlay_visible(store, false);
    }
}

/// 进度 pump（通道与任务由 `bridge::setup_subtitle_fit` 装好）。
/// 过期 `generation` 的消息直接丢；进度只写遮罩属性，不进依赖图。
pub(crate) async fn ui_pump_fit_messages(
    store: Rc<RefCell<AppStore>>,
    mut incoming: futures::channel::mpsc::UnboundedReceiver<FitMessage>,
) {
    let mut chart: Option<Chart> = None;
    while let Some(message) = incoming.next().await {
        match message {
            FitMessage::Progress { generation, event } => {
                if store.borrow().state().fit_generation != generation {
                    continue;
                }
                let Some(overlay) = store
                    .borrow()
                    .overlay_state
                    .as_ref()
                    .and_then(|weak| weak.upgrade())
                else {
                    continue;
                };
                let mut data = overlay.get_subtitle_fit_overlay_data();
                match event {
                    ProgressEvent::ActionStarted {
                        name,
                        grid_times,
                        curve_samples,
                        ..
                    } => {
                        let chart_state = Chart::new(grid_times, &curve_samples);
                        data.action_name = slint::SharedString::from(name.as_str());
                        data.percent = 0;
                        data.curve_commands = chart_state.curve_commands.clone().into();
                        data.bounds_commands = chart_state.sweep(0).into();
                        chart = Some(chart_state);
                    }
                    ProgressEvent::DpProgress { end, n } => {
                        data.percent = percent(end, n);
                        if let Some(chart_state) = chart.as_ref() {
                            data.bounds_commands = chart_state.sweep(end).into();
                        }
                    }
                    ProgressEvent::SegmentsFound { path } => {
                        if let Some(chart_state) = chart.as_ref() {
                            data.bounds_commands = chart_state.segments(&path).into();
                        }
                    }
                    // 调试版事件只有 `live_dp` 那条路会发；这里收到也只是不画。
                    ProgressEvent::DpStep { .. } => {}
                }
                overlay.set_subtitle_fit_overlay_data(data);
            }
            FitMessage::Finished { generation, script } => {
                chart = None;
                settle(&mut store.borrow_mut(), generation, script);
            }
        }
    }
}

/// 一条动画的画图状态：时间域与值域都来自这条动画自己，曲线和竖线共用同一套归一化，
/// 否则分段边界会对不上它切的那根线。
struct Chart {
    grid_times: Vec<f64>,
    t0: f64,
    span_t: f64,
    vmin: f64,
    span_v: f64,
    flat: bool,
    curve_commands: String,
}

fn default_data() -> crate::bridge::types::SubtitleFitOverlayData {
    crate::bridge::types::SubtitleFitOverlayData {
        action_name: slint::SharedString::default(),
        percent: 0,
        curve_commands: slint::SharedString::default(),
        bounds_commands: slint::SharedString::default(),
    }
}

fn percent(end: usize, n: usize) -> i32 {
    if n == 0 {
        100
    } else {
        ((end as f64 / n as f64) * 100.0).round().clamp(0.0, 100.0) as i32
    }
}

/// 零跨度会让归一化除出 `NaN`，写进 `commands` 就是一份解析不了的图。落到 1.0 让
/// 点叠在一起即可——真零跨度的动画在 DP 之前就短路了，走不到这里。
fn span_or_one(span: f64) -> f64 {
    if span > 1e-12 { span } else { 1.0 }
}

impl Chart {
    fn new(grid_times: Vec<f64>, samples: &[(f64, f64)]) -> Self {
        let t0 = grid_times.first().copied().unwrap_or(0.0);
        let t1 = grid_times.last().copied().unwrap_or(t0);
        let (lo, hi) = samples
            .iter()
            .fold((f64::INFINITY, f64::NEG_INFINITY), |acc, &(_, v)| {
                (acc.0.min(v), acc.1.max(v))
            });
        let flat = !(hi > lo);
        let chart = Self {
            grid_times,
            t0,
            span_t: span_or_one(t1 - t0),
            vmin: lo,
            span_v: span_or_one(hi - lo),
            flat,
            curve_commands: String::new(),
        };
        let mut curve_commands = String::new();
        for (i, (t, v)) in samples.iter().enumerate() {
            curve_commands.push_str(if i == 0 { "M" } else { " L" });
            curve_commands.push_str(&format!(" {:.2} {:.2}", chart.x_of(*t), chart.y_of(*v)));
        }
        Self {
            curve_commands,
            ..chart
        }
    }

    fn x_of(&self, t: f64) -> f64 {
        (t - self.t0) / self.span_t * CHART_WIDTH
    }

    /// 值大在上（屏幕 y 向下）；值域为零时画在正中。
    fn y_of(&self, v: f64) -> f64 {
        if self.flat {
            CHART_HEIGHT / 2.0
        } else {
            CHART_PADDING
                + (1.0 - (v - self.vmin) / self.span_v) * (CHART_HEIGHT - 2.0 * CHART_PADDING)
        }
    }

    /// 扫到格点 `end` 时的那根竖线：屏幕上看到的是 DP 沿时间轴推进。
    fn sweep(&self, end: usize) -> String {
        self.grid_times
            .get(end)
            .map(|&t| vline(self.x_of(t)))
            .unwrap_or_default()
    }

    /// 最终分段：每段起点一根竖线，末端再补一根，段数一目了然。
    fn segments(&self, path: &[(usize, usize)]) -> String {
        let mut out = String::new();
        for &(start, _) in path {
            if let Some(&t) = self.grid_times.get(start) {
                out.push_str(&vline(self.x_of(t)));
            }
        }
        if let Some(&(_, last)) = path.last()
            && let Some(&t) = self.grid_times.get(last)
        {
            out.push_str(&vline(self.x_of(t)));
        }
        out
    }
}

fn vline(x: f64) -> String {
    format!(" M {x:.2} 0 L {x:.2} {CHART_HEIGHT}")
}

/// 盖上遮罩并填初始数据。
fn show_overlay(store: &AppStore, data: crate::bridge::types::SubtitleFitOverlayData) {
    let Some(overlay) = store.overlay_state.as_ref().and_then(|weak| weak.upgrade()) else {
        return;
    };
    overlay.set_subtitle_fit_overlay_data(data);
    overlay.set_show_subtitle_fit_overlay(true);
}

fn set_overlay_visible(store: &AppStore, visible: bool) {
    let Some(overlay) = store.overlay_state.as_ref().and_then(|weak| weak.upgrade()) else {
        return;
    };
    overlay.set_show_subtitle_fit_overlay(visible);
}

#[cfg(test)]
mod tests {
    use super::*;

    /// 后台那条路整体跑一遍（工作线程内的转发线程 + 两条通道）。转发线程靠
    /// `drop(options)` 断开才收得到结束信号，忘了这一句 `relay.join()` 会永远挂住，
    /// 而这正是这份测试唯一能钉住的东西——静态读代码读不出来。
    #[test]
    fn a_job_delivers_exactly_one_finished_message() {
        let (ui_sender, mut ui_receiver) = futures::channel::mpsc::unbounded::<FitMessage>();
        run_job(FitJob {
            generation: 7,
            project: aweft_model::project::Project::new(),
            frame_size: (1920, 1080),
            ui_sender,
        });
        let first = ui_receiver.try_recv().expect("应当有一条消息");
        assert!(ui_receiver.try_recv().is_err(), "收尾之后不该还有消息");
        match first {
            FitMessage::Finished { generation, script } => {
                assert_eq!(generation, 7);
                assert!(script.is_some_and(|bytes| !bytes.is_empty()));
            }
            other => panic!("空工程没有动画可分段，收尾应当直接到 Finished，实际 {other:?}"),
        }
    }

    /// 平线（值域为零）不能把归一化除爆：y 落在中线上，x 仍铺满整个 viewbox。
    #[test]
    fn a_flat_curve_still_spans_the_chart() {
        let chart = Chart::new(vec![0.0, 1.0, 2.0], &[(0.0, 7.0), (1.0, 7.0), (2.0, 7.0)]);
        assert_eq!(
            chart.curve_commands,
            "M 0.00 20.00 L 50.00 20.00 L 100.00 20.00"
        );
        assert_eq!(chart.sweep(2), " M 100.00 0 L 100.00 40");
    }

    /// 值域非零时极值贴住上下留白，且大值画在上方（负值也要能画，旋转角就是负的）。
    #[test]
    fn curve_maps_its_extremes_to_the_paddings() {
        let chart = Chart::new(vec![0.0, 1.0], &[(0.0, -10.0), (0.5, 0.0), (1.0, 10.0)]);
        assert_eq!(
            chart.curve_commands,
            "M 0.00 37.00 L 50.00 20.00 L 100.00 3.00"
        );
    }

    /// 零跨度只让点叠在一起，不能产出 `NaN`——那是一份解析不了的图。
    #[test]
    fn a_zero_span_yields_no_nan() {
        let chart = Chart::new(vec![3.0, 3.0], &[(3.0, 1.0), (3.0, 2.0)]);
        assert!(
            !chart.curve_commands.contains("NaN"),
            "{}",
            chart.curve_commands
        );
    }

    #[test]
    fn percent_clamps_and_rounds() {
        assert_eq!(percent(0, 100), 0);
        assert_eq!(percent(50, 100), 50);
        assert_eq!(percent(999, 1000), 100);
        assert_eq!(percent(3, 0), 100);
    }

    /// 最终分段的竖线数 = 段数 + 末端那一根。
    #[test]
    fn segments_draw_one_line_per_boundary() {
        let chart = Chart::new(vec![0.0, 1.0, 2.0], &[(0.0, 0.0), (2.0, 1.0)]);
        assert_eq!(
            chart.segments(&[(0, 1), (1, 2)]),
            " M 0.00 0 L 0.00 40 M 50.00 0 L 50.00 40 M 100.00 0 L 100.00 40"
        );
        assert_eq!(chart.segments(&[]), "");
    }
}
