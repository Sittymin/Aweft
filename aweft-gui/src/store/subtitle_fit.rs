//! 字幕脚本拟合的后台通路：编脚本的 DP 分段跑在专用线程，UI 线程只收进度、只画视口
//! 右上角那块浮层（`ui/areas/viewport/subtitle_fit.slint`）。浮层不挡操作，所以拟合期间
//! 用户照常改东西、新请求照常进来——并发全靠下面第 1 点收口。
//!
//! 两件事叠在这里：
//!
//! 1. **单飞 + 合并最新**。同一时刻只允许一份拟合在跑，期间新到的请求只置一个待办标记，
//!    等手上这份跑完再用**当时最新**的数据重跑一份，浮层在两次之间不撤。
//! 2. **逐行复用拟合结果**。工作线程身上挂一份 [`aweft_convert::SequenceFitCache`]：
//!    挪动一行字幕只有那一行的键对不上，其余行直接复用。于是浮层上通常只出现真正重跑的
//!    那条动画——全命中时一个进度事件也没有，浮层一闪即撤。
//!
//! 后台线程拿到的是 `Project` 的副本，不回写任何真相——拟合的唯一产物是脚本文本。副本
//! 自带的求值缓存可能比真身旧，那只影响一次缓存命中，不影响结果。
//!
//! 浮层画的是 `aweft-convert/examples/live_dp.rs` 那张图：曲线 + 网格 + 底部有效性色带
//! + 当前格点（黄）+ 本格候选段（绿）+ 已锁定前缀（蓝），收尾再把最终分段逐段着色。这些
//! 线全部由 Rust 拼成 `Path.commands` 字符串——Slint 的 `Path` 子元素不支持 `for` 重复
//! （`compile_paths.rs` 直接报错）。坐标统一归一到 0..1000 × 0..400 的 viewbox。

use std::cell::RefCell;
use std::rc::Rc;
use std::sync::OnceLock;
use std::sync::mpsc;

use aweft_convert::ProgressEvent;
use futures::StreamExt;

use crate::bridge::types::{FitSegmentInfo, SubtitleFitOverlayData};
use crate::store::AppStore;

/// 曲线图 viewbox。必须与 `ui/areas/viewport/subtitle_fit.slint` 里每个 `Path` 写死的
/// `viewbox-width: 1000` / `viewbox-height: 400` 一致。
const CHART_WIDTH: f64 = 1000.0;
const CHART_HEIGHT: f64 = 400.0;
/// 绘图区四边留白：顶部给 `end=` 与 `start→end` 两行标签，底部给有效性色带，左右各留一点
/// 免得最外侧两根竖线被描边切掉。
const PLOT_LEFT: f64 = 16.0;
const PLOT_RIGHT: f64 = CHART_WIDTH - 16.0;
const PLOT_TOP: f64 = 46.0;
const PLOT_BOTTOM: f64 = CHART_HEIGHT - 34.0;
/// 底部有效性色带那一条。
const RIBBON_TOP: f64 = CHART_HEIGHT - 20.0;
const RIBBON_BOTTOM: f64 = CHART_HEIGHT - 4.0;
/// 曲线上下各让出值域的这一比例，极值就不贴着边框（同 `live_dp` 的 `yp`）。
const VALUE_PAD: f64 = 0.15;
/// 参考竖线的密度：每 n/16 一根。
const GRID_DIVISIONS: usize = 16;
/// 收尾逐段着色的循环配色，抄 `live_dp` 的 `render_final`。
const SEGMENT_COLORS: [(u8, u8, u8); 5] = [
    (0xe7, 0x4c, 0x3c),
    (0x34, 0x98, 0xdb),
    (0x2e, 0xcc, 0x71),
    (0xf3, 0x9c, 0x12),
    (0x9b, 0x59, 0xb6),
];

/// 后台送回 UI 线程的消息。`generation` 是请求序号，用于丢弃过期消息。
#[derive(Debug)]
pub enum FitMessage {
    /// 转换进度（`ProgressDetail::Diagnostic` 那组），只用来画浮层。
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
                // 逐行拟合缓存跟着这条线程走：它是唯一读者，不需要锁。
                let mut cache = aweft_convert::SequenceFitCache::default();
                // 单飞保证同时最多一份在飞，所以这里不会有排队积压。
                for job in receiver {
                    run_job(job, &mut cache);
                }
            });
        sender
    })
}

fn run_job(job: FitJob, cache: &mut aweft_convert::SequenceFitCache) {
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
        // 有效性色带要逐起点的判据，比轻量档只多每格一张布尔表。
        progress_detail: aweft_convert::ProgressDetail::Diagnostic,
        ..Default::default()
    };
    let converted =
        aweft_convert::sequence_to_ass_with_cache(&mut project, frame_size, &options, cache);
    let script = match converted {
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
    log::debug!("字幕脚本拟合完成，本次重拟合 {} 行", cache.refits());
    // sender 藏在 `options` 里，丢掉它转发线程才收得到断开信号；join 一下保证
    // 最后几个进度事件排在 `Finished` 前面。
    drop(options);
    let _ = relay.join();
    let _ = ui_sender.unbounded_send(FitMessage::Finished { generation, script });
}

/// 提交一次拟合请求。浮层立即显出（不挡操作），脚本在后台线程编好后由 pump 换进字幕通路。
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
    show_blank_overlay(store);
}

/// 一份拟合跑完后的收尾：换上脚本，再按待办标记决定重跑一份还是撤浮层。
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
///
/// 每轮先把队列里攒下的事件一次吃干净，再写一次 Slint：DP 一个格点一个事件，逐事件写会
/// 把 UI 线程占满，而那些中间帧既看不见也没意义。唯一必须逐事件处理的是 `chosen`——每格
/// 的起点一旦报出来就是定案，漏记一格，本地重建的锁定前缀就踩空洞。
pub(crate) async fn ui_pump_fit_messages(
    store: Rc<RefCell<AppStore>>,
    mut incoming: futures::channel::mpsc::UnboundedReceiver<FitMessage>,
) {
    let mut session: Option<Session> = None;
    while let Some(first) = incoming.next().await {
        apply(&store, &mut session, first);
        while let Ok(message) = incoming.try_recv() {
            apply(&store, &mut session, message);
        }
        if let Some(active) = session.as_ref() {
            write_overlay(&store, &active.data());
        }
    }
}

fn apply(store: &Rc<RefCell<AppStore>>, session: &mut Option<Session>, message: FitMessage) {
    match message {
        FitMessage::Progress { generation, event } => {
            if store.borrow().state().fit_generation != generation {
                return;
            }
            match event {
                ProgressEvent::ActionStarted {
                    name,
                    n,
                    grid_times,
                    curve_samples,
                    ..
                } => {
                    *session = Some(Session::started(name, n, grid_times, &curve_samples));
                }
                other => {
                    if let Some(active) = session.as_mut() {
                        active.update(other);
                    }
                }
            }
        }
        FitMessage::Finished { generation, script } => {
            if store.borrow().state().fit_generation != generation {
                return;
            }
            // 浮层的去留由 `settle` 定；先丢掉这一场，别让随后的重绘把它显回去。
            *session = None;
            settle(&mut store.borrow_mut(), generation, script);
        }
    }
}

/// 一场拟合在浮层上的状态。曲线与网格是每条动画算一次的静态部分，DP 逐格往前推的是
/// `end`/`chosen`/`valid` 这三样，外加从 `parents` 回溯出来的已锁定前缀。
struct Session {
    name: String,
    chart: Chart,
    /// 逐格定案的起点，`usize::MAX` 表示本格还不可达。长度 `n + 1`。
    parents: Vec<usize>,
    /// 已处理到的格点。
    end: usize,
    /// 最近一格的逐起点判据。
    valid: Vec<bool>,
    chosen: Option<usize>,
    /// 收尾路径；`Some` 之后画面换成逐段着色。
    final_path: Option<Vec<(usize, usize)>>,
}

impl Session {
    fn started(name: String, n: usize, grid_times: Vec<f64>, curve_samples: &[(f64, f64)]) -> Self {
        Self {
            name,
            chart: Chart::new(grid_times, n, curve_samples),
            parents: vec![usize::MAX; n + 1],
            end: 0,
            valid: Vec::new(),
            chosen: None,
            final_path: None,
        }
    }

    fn update(&mut self, event: ProgressEvent) {
        match event {
            ProgressEvent::DpProgress {
                end, chosen, valid, ..
            } => {
                if let Some(slot) = self.parents.get_mut(end) {
                    *slot = chosen.unwrap_or(usize::MAX);
                }
                self.end = end;
                self.chosen = chosen;
                self.valid = valid;
            }
            ProgressEvent::SegmentsFound { path } => {
                self.final_path = Some(path);
                self.chosen = None;
                self.valid = Vec::new();
            }
            // 一条动画一份图，`ActionStarted` 在 `apply` 里就分派掉了。
            ProgressEvent::ActionStarted { .. } => {}
        }
    }

    /// 沿 `parent` 链回溯出的已锁定分段，从最新一段往回数。
    fn locked_spans(&self) -> Vec<(usize, usize)> {
        let mut spans = Vec::new();
        let mut cur = self.end;
        while cur > 0 && cur < self.parents.len() {
            let parent = self.parents[cur];
            if parent == usize::MAX {
                break;
            }
            spans.push((parent, cur));
            cur = parent;
        }
        spans
    }

    fn data(&self) -> SubtitleFitOverlayData {
        let locked = self.locked_spans();
        let mut data = SubtitleFitOverlayData {
            action_name: slint::SharedString::from(self.name.as_str()),
            grid_commands: self.chart.grid_commands.clone().into(),
            curve_commands: self.chart.curve_commands.clone().into(),
            ..blank_data()
        };
        match self.final_path.as_ref() {
            // 收尾：最终分段逐段着色，DP 期间那几样 overlay 全部让位。
            Some(path) => {
                data.percent = 100;
                data.stats = format!("{} | 分段完成：{} 段", self.name, path.len())
                    .as_str()
                    .into();
                data.segments = finale_model(&self.chart, path);
            }
            None => {
                data.percent = percent(self.end, self.chart.n);
                data.stats = format!(
                    "{} | {}/{} | {} 个起点达标 | {} 段已定",
                    self.name,
                    self.end,
                    self.chart.n,
                    self.valid.iter().filter(|&&ok| ok).count(),
                    locked.len(),
                )
                .as_str()
                .into();
                let mut locked_commands = String::new();
                let mut span_commands = String::new();
                for (index, &(start, end)) in locked.iter().enumerate() {
                    // 最新那一段交给绿色高亮，蓝线里就不必再画一遍。
                    if index == 0 && self.chosen.is_some() {
                        continue;
                    }
                    if let Some(x) = self.chart.pair(start, end) {
                        locked_commands.push_str(&vline(x.0));
                        locked_commands.push_str(&vline(x.1));
                        span_commands.push_str(&fill_span(x.0, x.1));
                    }
                }
                let (ribbon_valid, ribbon_invalid) = self.chart.ribbons(&self.valid);
                let end_x = self.chart.time_at(self.end).map(|t| self.chart.x_of(t));
                let chosen_x = self
                    .chosen
                    .and_then(|start| self.chart.pair(start, self.end));
                data.locked_commands = locked_commands.as_str().into();
                data.span_commands = span_commands.as_str().into();
                data.ribbon_valid_commands = ribbon_valid.as_str().into();
                data.ribbon_invalid_commands = ribbon_invalid.as_str().into();
                if let Some(x) = end_x {
                    data.end_commands = vline(x).as_str().into();
                    data.end_fraction = (x / CHART_WIDTH) as f32;
                    data.end_label = format!("end={}", self.end).as_str().into();
                }
                if let Some((x0, x1)) = chosen_x {
                    data.chosen_commands = vline(x0).as_str().into();
                    data.chosen_span_commands = fill_span(x0, x1).as_str().into();
                    data.chosen_fraction = ((x0 + x1) / 2.0 / CHART_WIDTH) as f32;
                    data.chosen_label = format!("{}→{}", self.chosen.unwrap_or(0), self.end)
                        .as_str()
                        .into();
                }
            }
        }
        data
    }
}

/// 一条动画的画图状态：时间域与值域都取自这条动画自己，曲线、竖线、色带共用同一套归一化，
/// 否则分段边界会对不上它切的那根线。
struct Chart {
    grid_times: Vec<f64>,
    n: usize,
    t0: f64,
    span_t: f64,
    vmin: f64,
    span_v: f64,
    flat: bool,
    curve_commands: String,
    grid_commands: String,
}

impl Chart {
    fn new(grid_times: Vec<f64>, n: usize, samples: &[(f64, f64)]) -> Self {
        let t0 = grid_times.first().copied().unwrap_or(0.0);
        let t1 = grid_times.last().copied().unwrap_or(t0);
        let (lo, hi) = samples
            .iter()
            .fold((f64::INFINITY, f64::NEG_INFINITY), |acc, &(_, v)| {
                (acc.0.min(v), acc.1.max(v))
            });
        let flat = !(hi > lo);
        // 值域为零时别去算 `pad`：`inf - inf` 会炸出 `NaN`。
        let (vmin, span_v) = if flat {
            (0.0, 1.0)
        } else {
            let pad = (hi - lo) * VALUE_PAD;
            (lo - pad, (hi - lo) + 2.0 * pad)
        };
        let mut chart = Self {
            grid_times,
            n,
            t0,
            span_t: span_or_one(t1 - t0),
            vmin,
            span_v,
            flat,
            curve_commands: String::new(),
            grid_commands: String::new(),
        };
        let mut curve_commands = String::new();
        for (i, (t, v)) in samples.iter().enumerate() {
            curve_commands.push_str(if i == 0 { "M" } else { " L" });
            curve_commands.push_str(&format!(" {:.1} {:.1}", chart.x_of(*t), chart.y_of(*v)));
        }
        let grid_commands = chart.build_grid();
        chart.curve_commands = curve_commands;
        chart.grid_commands = grid_commands;
        chart
    }

    fn x_of(&self, t: f64) -> f64 {
        PLOT_LEFT + (t - self.t0) / self.span_t * (PLOT_RIGHT - PLOT_LEFT)
    }

    /// 值大在上（屏幕 y 向下）；值域为零时画在绘图区正中。
    fn y_of(&self, v: f64) -> f64 {
        if self.flat {
            return (PLOT_TOP + PLOT_BOTTOM) / 2.0;
        }
        PLOT_TOP + (1.0 - (v - self.vmin) / self.span_v) * (PLOT_BOTTOM - PLOT_TOP)
    }

    fn time_at(&self, index: usize) -> Option<f64> {
        self.grid_times.get(index).copied()
    }

    /// 两个格点在屏幕上的横向位置。
    fn pair(&self, a: usize, b: usize) -> Option<(f64, f64)> {
        let (ta, tb) = (self.time_at(a)?, self.time_at(b)?);
        Some((self.x_of(ta), self.x_of(tb)))
    }

    /// 主网格四等分线 + 每 n/16 一根参考竖线，全进同一根 `Path`（Slint 一支只能一色）。
    fn build_grid(&self) -> String {
        let mut out = String::new();
        for i in 0..=4 {
            out.push_str(&vline(
                PLOT_LEFT + (PLOT_RIGHT - PLOT_LEFT) * i as f64 / 4.0,
            ));
        }
        for i in (0..=self.n).step_by((self.n / GRID_DIVISIONS).max(1)) {
            if let Some(t) = self.time_at(i) {
                out.push_str(&vline(self.x_of(t)));
            }
        }
        out
    }

    /// 底部色带：本格达标的那些起点绿、其余红。`valid[i]` 说的是「格点 `i` 当起点可不可行」，
    /// 所以色带随 `end` 推进逐格变脸，正是 DP 在做判据的样子。
    fn ribbons(&self, valid: &[bool]) -> (String, String) {
        let mut ok = String::new();
        let mut bad = String::new();
        for (i, &is_valid) in valid.iter().enumerate() {
            let Some(t) = self.time_at(i) else {
                break;
            };
            let x = self.x_of(t);
            let line = format!(" M {x:.1} {RIBBON_TOP:.1} L {x:.1} {RIBBON_BOTTOM:.1}");
            if is_valid {
                ok.push_str(&line);
            } else {
                bad.push_str(&line);
            }
        }
        (ok, bad)
    }
}

/// 收尾的最终分段：一段一块 Rectangle，横向位置给成整图宽度的比例。
fn finale_model(chart: &Chart, path: &[(usize, usize)]) -> slint::ModelRc<FitSegmentInfo> {
    let rows: Vec<FitSegmentInfo> = path
        .iter()
        .enumerate()
        .filter_map(|(index, &(start, end))| {
            let (x0, x1) = chart.pair(start, end)?;
            Some(FitSegmentInfo {
                x0: (x0 / CHART_WIDTH) as f32,
                x1: (x1 / CHART_WIDTH) as f32,
                color: segment_color(index),
            })
        })
        .collect();
    slint::ModelRc::from(Rc::new(slint::VecModel::from(rows)))
}

fn segment_color(index: usize) -> slint::Color {
    let (r, g, b) = SEGMENT_COLORS[index % SEGMENT_COLORS.len()];
    slint::Color::from_rgb_u8(r, g, b)
}

/// 贯穿绘图区的一根竖线。
fn vline(x: f64) -> String {
    format!(" M {x:.1} {PLOT_TOP:.1} L {x:.1} {PLOT_BOTTOM:.1}")
}

/// 一段的底色：闭合矩形，交给只认 `fill` 的那支 `Path`。
fn fill_span(x0: f64, x1: f64) -> String {
    format!(
        " M {x0:.1} {PLOT_TOP:.1} L {x1:.1} {PLOT_TOP:.1} L {x1:.1} {PLOT_BOTTOM:.1} L {x0:.1} {PLOT_BOTTOM:.1} Z"
    )
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

/// 一份什么都没有的浮层数据：新请求显出浮层时先清场，别把上一场的曲线留在屏上。
fn blank_data() -> SubtitleFitOverlayData {
    SubtitleFitOverlayData {
        action_name: slint::SharedString::default(),
        stats: slint::SharedString::default(),
        percent: 0,
        grid_commands: slint::SharedString::default(),
        curve_commands: slint::SharedString::default(),
        ribbon_valid_commands: slint::SharedString::default(),
        ribbon_invalid_commands: slint::SharedString::default(),
        locked_commands: slint::SharedString::default(),
        span_commands: slint::SharedString::default(),
        end_commands: slint::SharedString::default(),
        chosen_commands: slint::SharedString::default(),
        chosen_span_commands: slint::SharedString::default(),
        end_fraction: 0.0,
        end_label: slint::SharedString::default(),
        chosen_fraction: 0.0,
        chosen_label: slint::SharedString::default(),
        segments: slint::ModelRc::default(),
    }
}

/// 显出浮层并清场。
fn show_blank_overlay(store: &AppStore) {
    let Some(overlay) = store.overlay_state.as_ref().and_then(|weak| weak.upgrade()) else {
        return;
    };
    overlay.set_subtitle_fit_overlay_data(blank_data());
    overlay.set_show_subtitle_fit_overlay(true);
}

fn write_overlay(store: &Rc<RefCell<AppStore>>, data: &SubtitleFitOverlayData) {
    let Some(overlay) = store
        .borrow()
        .overlay_state
        .as_ref()
        .and_then(|weak| weak.upgrade())
    else {
        return;
    };
    overlay.set_subtitle_fit_overlay_data(data.clone());
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
    use slint::Model as _;

    fn session_for(grid_times: &[f64], samples: &[(f64, f64)]) -> Session {
        Session::started(
            "a".into(),
            grid_times.len() - 1,
            grid_times.to_vec(),
            samples,
        )
    }

    fn dp(end: usize, chosen: Option<usize>, valid: Vec<bool>) -> ProgressEvent {
        ProgressEvent::DpProgress {
            end,
            n: 4,
            chosen,
            valid,
        }
    }

    /// 后台那条路整体跑一遍（工作线程内的转发线程 + 两条通道 + 逐行缓存）。转发线程靠
    /// `drop(options)` 断开才收得到结束信号，忘了这一句 `relay.join()` 会永远挂住，
    /// 而这正是这份测试唯一能钉住的东西——静态读代码读不出来。
    #[test]
    fn a_job_delivers_exactly_one_finished_message() {
        let (ui_sender, mut ui_receiver) = futures::channel::mpsc::unbounded::<FitMessage>();
        run_job(
            FitJob {
                generation: 7,
                project: aweft_model::project::Project::new(),
                frame_size: (1920, 1080),
                ui_sender,
            },
            &mut aweft_convert::SequenceFitCache::default(),
        );
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

    /// 平线（值域为零）不能把归一化除爆：y 落在绘图区正中，x 仍铺满整个 viewbox。
    #[test]
    fn a_flat_curve_still_spans_the_plot() {
        let chart = Chart::new(
            vec![0.0, 1.0, 2.0],
            2,
            &[(0.0, 7.0), (1.0, 7.0), (2.0, 7.0)],
        );
        assert_eq!(
            chart.curve_commands,
            format!("M {PLOT_LEFT:.1} 206.0 L 500.0 206.0 L {PLOT_RIGHT:.1} 206.0")
        );
    }

    /// 值域非零时大值画在上方，且上下极值都留在绘图区内（负值也要能画，旋转角就是负的）。
    #[test]
    fn extremes_land_inside_the_plot_flipped() {
        let chart = Chart::new(vec![0.0, 1.0], 1, &[(0.0, -10.0), (0.5, 0.0), (1.0, 10.0)]);
        assert!((chart.y_of(10.0) - PLOT_TOP).abs() > 1.0);
        assert!(chart.y_of(-10.0) < PLOT_BOTTOM);
        assert!(chart.y_of(10.0) < chart.y_of(0.0));
        assert!(chart.y_of(0.0) < chart.y_of(-10.0));
    }

    /// 零跨度只让点叠在一起，不能产出 `NaN`——那是一份解析不了的图。
    #[test]
    fn a_zero_span_yields_no_nan() {
        let chart = Chart::new(vec![3.0, 3.0], 1, &[(3.0, 1.0), (3.0, 2.0)]);
        assert!(
            !chart.curve_commands.contains("NaN"),
            "{}",
            chart.curve_commands
        );
        assert!(
            !chart.grid_commands.contains("NaN"),
            "{}",
            chart.grid_commands
        );
    }

    #[test]
    fn percent_clamps_and_rounds() {
        assert_eq!(percent(0, 100), 0);
        assert_eq!(percent(50, 100), 50);
        assert_eq!(percent(999, 1000), 100);
        assert_eq!(percent(3, 0), 100);
    }

    /// 每格的起点一旦报出来就是定案，逐格记下来就能在本地重建已锁定前缀——这正是
    /// `min_segmentation` 不再每格回传整条 `parent` 链的前提。
    #[test]
    fn chosen_per_point_rebuilds_the_locked_prefix() {
        let mut session = session_for(&[0.0, 1.0, 2.0, 3.0, 4.0], &[(0.0, 0.0), (4.0, 1.0)]);
        for (end, chosen) in [(1, Some(0)), (2, Some(1)), (3, Some(2)), (4, Some(2))] {
            session.update(dp(end, chosen, vec![true; end]));
        }
        assert_eq!(
            session.locked_spans(),
            vec![(2, 4), (1, 2), (0, 1)],
            "从最新一段往回数"
        );
    }

    /// DP 进行中的那一帧：黄线在当前格点，绿线在候选段起点，最新那段蓝线让位给绿色，
    /// 色带按本格逐起点的判据分绿红两支。
    #[test]
    fn a_dp_frame_draws_end_chosen_and_the_ribbon() {
        let mut session = session_for(&[0.0, 1.0, 2.0, 3.0, 4.0], &[(0.0, 0.0), (4.0, 1.0)]);
        session.update(dp(1, Some(0), vec![true]));
        session.update(dp(2, Some(1), vec![false, true]));
        session.update(dp(3, Some(2), vec![true, false, true]));
        let data = session.data();
        // x_of: 格点 0..4 → 16, 258, 500, 742, 984。
        assert!(data.end_commands.contains("742.0"), "{}", data.end_commands);
        assert!(
            data.chosen_commands.contains("500.0"),
            "{}",
            data.chosen_commands
        );
        assert!(
            data.chosen_span_commands.contains("500.0")
                && data.chosen_span_commands.contains("742.0"),
            "{}",
            data.chosen_span_commands
        );
        // 最新一段 (2,3) 让位给绿色，所以 742 不在蓝线里；上一段 (1,2) 的两根都在。
        assert!(
            data.locked_commands.contains("258.0") && !data.locked_commands.contains("742.0"),
            "{}",
            data.locked_commands
        );
        assert!(
            data.ribbon_valid_commands.matches("M ").count() == 2
                && data.ribbon_invalid_commands.matches("M ").count() == 1,
            "valid = [true,false,true] 该是两支绿、一支红：{} / {}",
            data.ribbon_valid_commands,
            data.ribbon_invalid_commands
        );
        assert_eq!(data.percent, 75);
        assert_eq!(data.end_label, "end=3");
        assert_eq!(data.chosen_label, "2→3");
        assert_eq!(slint::Model::row_count(&data.segments), 0);
    }

    /// 收尾那一帧：进度打满、DP 那几样让位，最终分段一段一块、横向位置给成整图比例。
    #[test]
    fn the_finale_replaces_dp_overlay_with_colored_segments() {
        let mut session = session_for(&[0.0, 1.0, 2.0, 3.0, 4.0], &[(0.0, 0.0), (4.0, 1.0)]);
        session.update(dp(4, Some(2), vec![true, false, true, false]));
        session.update(ProgressEvent::SegmentsFound {
            path: vec![(0, 2), (2, 4)],
        });
        let data = session.data();
        assert_eq!(data.percent, 100);
        assert_eq!(data.end_commands, "");
        assert_eq!(data.chosen_commands, "");
        assert_eq!(data.ribbon_valid_commands, "");
        assert_eq!(slint::Model::row_count(&data.segments), 2);
        let first = data.segments.row_data(0).expect("两段");
        assert!((first.x0 - 0.016).abs() < 1e-6, "{}", first.x0);
        assert!((first.x1 - 0.5).abs() < 1e-6, "{}", first.x1);
        assert!(data.stats.contains("2 段"), "{}", data.stats);
    }

    /// 不可达的格子（`chosen = None`）记成 `usize::MAX`，回溯到它就停，不能拿它当下标。
    #[test]
    fn an_unreachable_point_stops_the_walk_back() {
        let mut session = session_for(&[0.0, 1.0, 2.0, 3.0, 4.0], &[(0.0, 0.0), (4.0, 1.0)]);
        session.update(dp(1, Some(0), vec![true]));
        session.update(dp(2, None, vec![false, false]));
        assert_eq!(session.parents[2], usize::MAX);
        assert!(session.locked_spans().is_empty());
        // 蓝线一支没有，但黄线仍然标出当前格点。
        let data = session.data();
        assert_eq!(data.locked_commands, "");
        assert!(data.end_commands.contains("500.0"), "{}", data.end_commands);
    }
}
