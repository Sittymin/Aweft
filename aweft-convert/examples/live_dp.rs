//! 实时 DP 分段（拟合）可视化。
//!
//! 内置最小 HTTP 服务器，浏览器每 150ms 轮询 `/frame` 获取当前 SVG 帧。
//! 依次对四条贝塞尔曲线（ease-in-out / overshoot / ease-out / back-out）
//! 各跑一遍 DP 拟合动画，每条的结尾展示其分段路径。

use std::io::{Read, Write};
use std::net::{TcpListener, TcpStream};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc;
use std::sync::{Arc, Mutex};
use std::thread;
use std::time::Duration;

use aweft_convert::animate_to_ass::ConvertOptions;
use aweft_convert::{convert_anim_data, Cursor, ProgressEvent};
use aweft_model::animation::action::{Action, ActionCurve};
use aweft_model::animation::anim_data::PropertyPath;
use aweft_model::animation::fcurve::FCurve;
use aweft_model::animation::keyframe::{BezierHandle, Interpolation, Keyframe};
use aweft_model::animation::strip::{NlaStrip, NlaTrack};
use aweft_model::animation::{AnimData, AnimationLibrary};
use aweft_model::ids::{ActionId, AnimDataId};

const SVG_HEAD: &str = "<svg viewBox=\"0 0 800 300\" style=\"background:#1a1a1a;font-family:monospace\" xmlns=\"http://www.w3.org/2000/svg\">";
const SVG_TAIL: &str = "</svg>";

/// 坐标映射：把数据域 (x, y) 映射到画布像素区。
struct Chart {
    pl: f64,
    pr: f64,
    pt: f64,
    pb: f64,
    xmin: f64,
    xmax: f64,
    y0: f64,
    y1: f64,
}
impl Chart {
    fn new(xmin: f64, xmax: f64, ymin: f64, ymax: f64) -> Self {
        let yp = (ymax - ymin).max(1.0) * 0.15;
        Self { pl: 52.0, pr: 784.0, pt: 14.0, pb: 280.0, xmin, xmax, y0: ymin - yp, y1: ymax + yp }
    }
    fn sx(&self, x: f64) -> f64 {
        self.pl + (x - self.xmin) / (self.xmax - self.xmin) * (self.pr - self.pl)
    }
    fn sy(&self, y: f64) -> f64 {
        self.pb - (y - self.y0) / (self.y1 - self.y0) * (self.pb - self.pt)
    }
}

/// 折线（可选虚线）。
fn polyline(chart: &Chart, pts: &[(f64, f64)], stroke: &str, width: f64, dash: Option<&str>) -> String {
    let p: String = pts
        .iter()
        .map(|(x, y)| format!("{:.1},{:.1}", chart.sx(*x), chart.sy(*y)))
        .collect::<Vec<_>>()
        .join(" ");
    match dash {
        Some(d) => format!(
            "<polyline points=\"{p}\" fill=\"none\" stroke=\"{stroke}\" stroke-width=\"{width}\" stroke-dasharray=\"{d}\"/>"
        ),
        None => format!(
            "<polyline points=\"{p}\" fill=\"none\" stroke=\"{stroke}\" stroke-width=\"{width}\"/>"
        ),
    }
}

/// 竖直参考线（可选虚线）。
fn vline(chart: &Chart, x: f64, stroke: &str, width: f64, dash: Option<&str>) -> String {
    let x = chart.sx(x);
    match dash {
        Some(d) => format!(
            "<line x1=\"{x:.1}\" y1=\"{}\" x2=\"{x:.1}\" y2=\"{}\" stroke=\"{stroke}\" stroke-width=\"{width}\" stroke-dasharray=\"{d}\"/>",
            chart.pt, chart.pb
        ),
        None => format!(
            "<line x1=\"{x:.1}\" y1=\"{}\" x2=\"{x:.1}\" y2=\"{}\" stroke=\"{stroke}\" stroke-width=\"{width}\"/>",
            chart.pt, chart.pb
        ),
    }
}

/// DP 进行中：曲线 + 锁定段（蓝）+ 当前 end（黄）+ 候选段（绿）+ 底部有效性色带。
fn render_frame(
    label: &str, end: usize, n: usize, grid_times: &[f64], curve: &[(f64, f64)],
    valid: &[bool], chosen: Option<usize>, parent_chain: &[usize],
) -> String {
    use std::fmt::Write as _;
    let (xmin, xmax) = (grid_times[0], grid_times[grid_times.len() - 1]);
    let ymin = curve.iter().map(|(_, v)| *v).fold(f64::MAX, f64::min);
    let ymax = curve.iter().map(|(_, v)| *v).fold(f64::MIN, f64::max);
    let chart = Chart::new(xmin, xmax, ymin, ymax);

    let mut s = String::new();
    let _ = write!(s, "{SVG_HEAD}");
    for i in 0..=4 {
        s.push_str(&vline(&chart, xmin + (xmax - xmin) * i as f64 / 4.0, "#333", 1.0, None));
    }
    s.push_str(&polyline(&chart, curve, "#eee", 2.0, None));

    // 已锁定前缀路径：沿 parent 链回溯，蓝色实线 + 浅蓝底
    let mut locked = Vec::new();
    let mut cur = end;
    while cur > 0 && cur < parent_chain.len() {
        let p = parent_chain[cur];
        if p == usize::MAX { break; }
        locked.push((p, cur));
        cur = p;
    }
    for (i, (ps, pe)) in locked.iter().enumerate() {
        let (xs, xe) = (chart.sx(grid_times[*ps]), chart.sx(grid_times[*pe]));
        if i == 0 && chosen.is_some() { continue; } // 最新段由绿色高亮
        let _ = write!(
            s, "<line x1=\"{xs:.1}\" y1=\"{}\" x2=\"{xs:.1}\" y2=\"{}\" stroke=\"#39f\" stroke-width=\"2\" opacity=\"0.7\"/>",
            chart.pt, chart.pb
        );
        let _ = write!(
            s, "<line x1=\"{xe:.1}\" y1=\"{}\" x2=\"{xe:.1}\" y2=\"{}\" stroke=\"#39f\" stroke-width=\"2\" opacity=\"0.7\"/>",
            chart.pt, chart.pb
        );
        let _ = write!(
            s, "<rect x=\"{xs:.1}\" y=\"{}\" width=\"{:.1}\" height=\"{}\" fill=\"#39f\" opacity=\"0.05\"/>",
            chart.pt, xe - xs, chart.pb - chart.pt
        );
    }

    // 每 n/16 一根灰色参考竖线 + 底部有效性色带
    let step = (n / 16).max(1);
    let (bar_y, bar_h) = (294.0, 5.0);
    for i in (0..=n).step_by(step) {
        s.push_str(&vline(&chart, grid_times[i.min(grid_times.len() - 1)], "#444", 1.0, Some("2,4")));
    }
    for i in 0..=n.min(grid_times.len() - 1) {
        if i > end { break; }
        let x = chart.sx(grid_times[i]);
        let c = if i < valid.len() && valid[i] { "#4c4" } else { "#c33" };
        let _ = write!(s, "<line x1=\"{x:.1}\" y1=\"{bar_y}\" x2=\"{x:.1}\" y2=\"{}\" stroke=\"{c}\" stroke-width=\"1.5\"/>", bar_y + bar_h);
    }

    // 当前 end：黄色粗线
    if end < grid_times.len() {
        let xe = chart.sx(grid_times[end]);
        let _ = write!(
            s, "<line x1=\"{xe:.1}\" y1=\"{}\" x2=\"{xe:.1}\" y2=\"{}\" stroke=\"#ff0\" stroke-width=\"2.5\"/>",
            chart.pt, chart.pb
        );
        let _ = write!(s, "<text x=\"{xe:.1}\" y=\"{}\" fill=\"#ff0\" font-size=\"12\" text-anchor=\"middle\" font-weight=\"bold\">end={end}</text>", chart.pt - 2.0);
    }

    // 当前选中的 start→end：绿色粗线 + 半透明绿底
    if let Some(start) = chosen {
        let (xs, xe) = (chart.sx(grid_times[start]), chart.sx(grid_times[end]));
        let _ = write!(
            s, "<line x1=\"{xs:.1}\" y1=\"{}\" x2=\"{xs:.1}\" y2=\"{}\" stroke=\"#0f0\" stroke-width=\"2.5\"/>",
            chart.pt, chart.pb
        );
        let _ = write!(
            s, "<rect x=\"{xs:.1}\" y=\"{}\" width=\"{:.1}\" height=\"{}\" fill=\"#0f0\" opacity=\"0.08\"/>",
            chart.pt, xe - xs, chart.pb - chart.pt
        );
        let _ = write!(s, "<text x=\"{:.1}\" y=\"28\" fill=\"#4c4\" font-size=\"10\" text-anchor=\"middle\">{start}→{end}</text>", (xs + xe) / 2.0);
    }

    // 统计
    let vc = valid.iter().filter(|&&v| v).count();
    let _ = write!(s, "<text x=\"10\" y=\"20\" fill=\"#888\" font-size=\"12\">{label} | {end}/{n} | {vc} valid | {} locked</text>", locked.len());
    let _ = write!(s, "{SVG_TAIL}");
    s
}

/// 最终路径的三步动画：亮起边界 → 锁定段 → 完成。
fn render_final(
    grid_times: &[f64], curve: &[(f64, f64)], path: &[(usize, usize)], phase: usize, name: &str,
) -> String {
    use std::fmt::Write as _;
    let (xmin, xmax) = (grid_times[0], grid_times[grid_times.len() - 1]);
    let ymin = curve.iter().map(|(_, v)| *v).fold(f64::MAX, f64::min);
    let ymax = curve.iter().map(|(_, v)| *v).fold(f64::MIN, f64::max);
    let chart = Chart::new(xmin, xmax, ymin, ymax);
    let colors = ["#e74c3c", "#3498db", "#2ecc71", "#f39c12", "#9b59b6"];

    let mut s = String::new();
    let _ = write!(s, "{SVG_HEAD}");
    for i in 0..=4 {
        s.push_str(&vline(&chart, xmin + (xmax - xmin) * i as f64 / 4.0, "#333", 1.0, None));
    }
    s.push_str(&polyline(&chart, curve, "#eee", 2.0, None));

    let n = grid_times.len() - 1;
    let step = (n / 16).max(1);
    for i in (0..=n).step_by(step) {
        s.push_str(&vline(&chart, grid_times[i], "#444", 0.5, Some("2,4")));
    }

    match phase {
        0 => {
            // 步 1：所有路径边界亮起黄色虚线
            for &(_start, end) in path {
                s.push_str(&vline(&chart, grid_times[end], "#ff0", 2.5, Some("8,4")));
            }
            let _ = write!(s, "<text x=\"10\" y=\"20\" fill=\"#fa0\" font-size=\"13\">Building final path: {} segments</text>", path.len());
        }
        1 => {
            // 步 2：边界变为绿色实线，段内填充浅色
            for (seg_idx, &(start, end)) in path.iter().enumerate() {
                let c = colors[seg_idx % colors.len()];
                let (xs, xe) = (chart.sx(grid_times[start]), chart.sx(grid_times[end]));
                let _ = write!(s, "<line x1=\"{xe:.1}\" y1=\"{}\" x2=\"{xe:.1}\" y2=\"{}\" stroke=\"{c}\" stroke-width=\"2\"/>", chart.pt, chart.pb);
                let _ = write!(s, "<rect x=\"{xs:.1}\" y=\"{}\" width=\"{:.1}\" height=\"{}\" fill=\"{c}\" opacity=\"0.06\"/>", chart.pt, xe - xs, chart.pb - chart.pt);
                let _ = write!(s, "<text x=\"{:.1}\" y=\"{}\" fill=\"{c}\" font-size=\"9\" text-anchor=\"middle\">seg {seg_idx}</text>", (xs + xe) / 2.0, chart.pt - 2.0);
            }
            let _ = write!(s, "<text x=\"10\" y=\"20\" fill=\"#4c4\" font-size=\"13\">Locking segments...</text>");
        }
        _ => {
            // 步 3：终态 — 彩色段 + 段标签 + 段内曲线
            for (seg_idx, &(start, end)) in path.iter().enumerate() {
                let c = colors[seg_idx % colors.len()];
                let (xs, xe) = (chart.sx(grid_times[start]), chart.sx(grid_times[end]));
                let _ = write!(s, "<line x1=\"{xe:.1}\" y1=\"{}\" x2=\"{xe:.1}\" y2=\"{}\" stroke=\"{c}\" stroke-width=\"2.5\"/>", chart.pt, chart.pb);
                let _ = write!(s, "<rect x=\"{xs:.1}\" y=\"{}\" width=\"{:.1}\" height=\"{}\" fill=\"{c}\" opacity=\"0.1\"/>", chart.pt, xe - xs, chart.pb - chart.pt);
                let seg_pts: String = curve
                    .iter()
                    .filter(|(t, _)| *t >= grid_times[start] - 1e-9 && *t <= grid_times[end] + 1e-9)
                    .map(|(x, y)| format!("{:.1},{:.1}", chart.sx(*x), chart.sy(*y)))
                    .collect::<Vec<_>>()
                    .join(" ");
                let _ = write!(s, "<polyline points=\"{seg_pts}\" fill=\"none\" stroke=\"{c}\" stroke-width=\"3\"/>");
            }
            let _ = write!(s, "<text x=\"10\" y=\"20\" fill=\"#4c4\" font-size=\"14\" font-weight=\"bold\">{name}: {} 段 ✓</text>", path.len());
        }
    }
    let _ = write!(s, "{SVG_TAIL}");
    s
}

/// 四条贝塞尔曲线（linear 不算贝塞尔，因此只有四条）。
fn bezier_curves() -> Vec<(&'static str, FCurve)> {
    let kf = |time: f64, value: f64, out_t: f64, out_v: f64, in_t: f64, in_v: f64| Keyframe {
        time, value,
        interpolation: Interpolation::Bezier {
            handle_out: BezierHandle::new(out_t, out_v),
            handle_in: BezierHandle::new(in_t, in_v),
        },
    };
    vec![
        ("ease-in-out", FCurve {
            keyframes: vec![kf(0.0, 0.0, 0.5, 0.2, -0.5, -0.2), kf(1.0, 1.0, 0.5, 0.2, -0.5, -0.2)],
            ..Default::default()
        }),
        ("overshoot", FCurve {
            keyframes: vec![kf(0.0, 0.0, 0.5, 2.0, 0.0, 0.0), kf(1.0, 1.0, 0.0, 0.0, -0.5, 0.5)],
            ..Default::default()
        }),
        ("ease-out", FCurve {
            keyframes: vec![kf(0.0, 0.0, 0.1, 1.0, 0.0, 0.0), kf(1.0, 1.0, 0.0, 0.0, -0.9, 0.0)],
            ..Default::default()
        }),
        ("back-out", FCurve {
            keyframes: vec![kf(0.0, 0.0, 0.68, -0.55, 0.0, 0.0), kf(1.0, 1.0, 0.0, 0.0, -0.73, 0.55)],
            ..Default::default()
        }),
    ]
}

fn http_ok(body: &str, ct: &str) -> String {
    format!("HTTP/1.1 200 OK\r\nContent-Type: {ct}\r\nContent-Length: {}\r\nAccess-Control-Allow-Origin: *\r\nConnection: close\r\n\r\n{body}", body.len())
}

fn handle_client(mut stream: TcpStream, frame: &Arc<Mutex<String>>, done: &AtomicBool) {
    let mut buf = [0u8; 2048];
    let _ = stream.read(&mut buf);
    let req = String::from_utf8_lossy(&buf);
    let f = frame.lock().unwrap();

    if req.starts_with("GET /frame ") {
        let body = f.clone();
        let _ = stream.write_all(http_ok(&body, "image/svg+xml").as_bytes());
    } else if req.starts_with("GET / ") {
        let done_flag = done.load(Ordering::Relaxed);
        let svg_block = if f.is_empty() {
            "<text x=\"400\" y=\"150\" fill=\"#888\" font-size=\"20\" text-anchor=\"middle\">等待 DP 开始...</text>".to_string()
        } else {
            f.clone()
        };
        let done_banner = if done_flag {
            "<div style=\"position:fixed;top:1rem;right:1rem;color:#4c4;font-size:1.5rem;font-weight:bold\">DONE ✓</div>"
        } else {
            ""
        };
        let js = r#"let p=0;let iv=setInterval(async()=>{try{
         let r=await fetch('/frame');if(!r.ok)throw new Error(r.status);
         let t=await r.text();document.body.innerHTML=t+'<span class="err"></span>';
         p=0;}catch(e){if(++p>5){clearInterval(iv);document.body.innerHTML+='<span class="done">DONE ✓</span>'}}},33);"#;
        let html = format!(
            "<!DOCTYPE html><html><head><meta charset=\"utf-8\"><title>live DP</title>\
             <style>body{{margin:0;background:#111;overflow:hidden}}\
             .done{{position:fixed;top:1rem;right:1rem;color:#4c4;font-size:1.5rem}}\
             .err{{position:fixed;bottom:1rem;left:1rem;color:#c33;font-size:.8rem}}\
             </style></head><body>{done_banner}<span class=\"err\"></span>\
             {svg_block}<script>{js}</script></body></html>");
        let _ = stream.write_all(http_ok(&html, "text/html; charset=utf-8").as_bytes());
    } else {
        let _ = stream.write_all(b"HTTP/1.1 404\r\n\r\n");
    }
}

fn main() {
    let port = 9000;
    println!("Live DP viewer: open http://localhost:{port}");

    let curves = bezier_curves();

    let frame = Arc::new(Mutex::new(String::new()));
    let is_done = Arc::new(AtomicBool::new(false));
    let shutdown = Arc::new(AtomicBool::new(false));

    let http_frame = frame.clone();
    let http_done = is_done.clone();
    let http_shutdown = shutdown.clone();
    let listener = TcpListener::bind(format!("127.0.0.1:{port}")).unwrap();
    let http_thread = thread::spawn(move || {
        listener.set_nonblocking(true).ok();
        loop {
            if let Ok((stream, _)) = listener.accept() {
                let fc = http_frame.clone();
                let dc = http_done.clone();
                thread::spawn(move || handle_client(stream, &fc, &dc));
            }
            if http_shutdown.load(Ordering::Relaxed) {
                // 再服务 2 秒让浏览器收到最后一块面板
                let deadline = std::time::Instant::now() + Duration::from_secs(2);
                while std::time::Instant::now() < deadline {
                    if let Ok((stream, _)) = listener.accept() {
                        let fc = http_frame.clone();
                        let dc = http_done.clone();
                        thread::spawn(move || handle_client(stream, &fc, &dc));
                    }
                    thread::sleep(Duration::from_millis(20));
                }
                break;
            }
            thread::sleep(Duration::from_millis(20));
        }
    });

    // DP 线程：依次转换四条曲线，逐条把拟合进度事件发进通道
    let (tx, rx) = mpsc::channel::<ProgressEvent>();
    let render_curves = curves.clone();
    let dp_thread = thread::spawn(move || {
        for (name, curve) in &curves {
            let mut lib = AnimationLibrary::default();
            let aid = ActionId::new();
            lib.insert_action(Action {
                id: aid, name: (*name).into(), duration: 1.0,
                curves: vec![ActionCurve::new(PropertyPath::FontSize, curve.clone())],
            });
            let anim = AnimData {
                id: AnimDataId::new(), name: "a".into(),
                tracks: vec![NlaTrack {
                    name: "t".into(), muted: false, solo: false,
                    strips: vec![NlaStrip::new_action("s", aid)],
                }],
            };
            let _ = convert_anim_data(
                &Default::default(),
                &mut Cursor::from_style(&Default::default()),
                &anim, &lib, 1.0,
                &ConvertOptions { error_tolerance: 0.01, progress_sender: Some(tx.clone()) },
            );
        }
    });

    // 渲染线程：逐条播放拟合动画，DP 结束标志 + 退出标志
    let render_thread = thread::spawn(move || {
        let total = render_curves.len();
        let mut grid_times = Vec::new();
        let mut curve_samples = Vec::new();
        let mut n = 0;
        let mut curve_idx = 0usize;
        for event in rx {
            match event {
                ProgressEvent::ActionStarted { action_id: _, n: dn, grid_times: gt, curve_samples: cs } => {
                    n = dn; grid_times = gt; curve_samples = cs;
                    curve_idx += 1;
                }
                ProgressEvent::DpStep { end, valid, chosen, parent_chain } => {
                    // 采样约 1/4 帧（浏览器 33ms 轮询，全量 256 帧既拖慢也看不完），
                    // 再用自适应定速：渲染耗时不足 33ms 就补觉到 ~30fps，
                    // 渲染本身慢（debug）则顺其自然，避免叠加卡顿。
                    let skip = (n / 64).max(1);
                    if end % skip != 0 && end != n {
                        continue;
                    }
                    let name = render_curves[curve_idx.saturating_sub(1)].0;
                    let label = format!("{name}  {curve_idx}/{total}  拟合中");
                    let t0 = std::time::Instant::now();
                    *frame.lock().unwrap() = render_frame(
                        &label, end, n, &grid_times, &curve_samples, &valid, chosen,
                        &parent_chain,
                    );
                    let target = Duration::from_millis(33);
                    let elapsed = t0.elapsed();
                    if elapsed < target {
                        thread::sleep(target - elapsed);
                    }
                }
                ProgressEvent::SegmentsFound { path } => {
                    let name = render_curves[curve_idx.saturating_sub(1)].0;
                    for phase in 0..3 {
                        *frame.lock().unwrap() = render_final(
                            &grid_times, &curve_samples, &path, phase, name,
                        );
                        thread::sleep(Duration::from_millis(300));
                    }
                }
            }
        }
        is_done.store(true, Ordering::Relaxed);
        shutdown.store(true, Ordering::Relaxed);
    });

    dp_thread.join().unwrap();
    render_thread.join().unwrap();
    http_thread.join().unwrap();
    println!("Done!");
}
