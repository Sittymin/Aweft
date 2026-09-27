//! 核心序列数据 → 完整 ASS 脚本：视口预览与文件导出共用这一条转换。
//!
//! 预览把这里产出的 `ASSFile` 经 `ASSFile::to_bytes` 交给 libass，导出把同一份
//! `ASSFile` 写成文件。两条路出自同一个内存结构，字节不会分叉。

use std::collections::{HashMap, HashSet};

use aweft_ass::types::event::{Dialogue, Event, TextSegment};
use aweft_ass::types::style::StyleWithoutName;
use aweft_ass::types::tags::StyleTag;
use aweft_ass::types::time::AssTime;
use aweft_ass::types::ASSFile;
use aweft_model::application_graph::{ApplicationError, ApplicationGraphMatch, LineContext};
use aweft_model::ids::{AnimDataId, ItemId, NodeGraphId};
use aweft_model::library::ProjectLibrary;
use aweft_model::node_graph::style::StyleResult;
use aweft_model::node_graph::EvalContext;
use aweft_model::project::Project;
use aweft_model::sequence::{EventData, ItemContent, Sequence};
use chrono::TimeDelta;

use crate::animate_to_ass::{match_to_ass_tags, ConvertOptions, TimedTags};

/// 所有生成行统一引用的样式名。逐行的静态样式不建独立 Style，而是以覆盖标签写进
/// 行文本：`match_to_ass_tags` 已把样式基线预置进每个段（animate_to_ass.rs:306）。
const DEFAULT_STYLE: &str = "Default";

/// 把项目的序列与样式层转成一份完整脚本。
///
/// `frame_size` 是渲染帧尺寸（视口侧取主视频的显示像素尺寸），写进
/// `[Script Info]` 的 PlayResX/PlayResY；libass 按它解释边距与对齐锚点。
///
/// 一次性转换用这个。界面那样反复转换同一个工程请用 [`sequence_to_ass_with_cache`]：
/// 逐行拟合结果留在调用方的 [`SequenceFitCache`] 里，挪动一行不必把整个序列重拟合一遍。
pub fn sequence_to_ass(
    project: &mut Project,
    frame_size: (u32, u32),
    opts: &ConvertOptions,
) -> Result<Converted, ApplicationError> {
    let mut cache = SequenceFitCache::default();
    sequence_to_ass_with_cache(project, frame_size, opts, &mut cache)
}

/// [`sequence_to_ass`] 的复用版：拟合结果在 `cache` 里跨调用留存。
///
/// 命中条件是「这一行拟合所依赖的输入一个都没变」，判定见 [`LineFitKey`]，所以输出与
/// 不带缓存时逐字节相同。缓存按内容指纹自我失效，调用方不需要在编辑之后手动清理。
pub fn sequence_to_ass_with_cache(
    project: &mut Project,
    frame_size: (u32, u32),
    opts: &ConvertOptions,
    cache: &mut SequenceFitCache,
) -> Result<Converted, ApplicationError> {
    // 拆开借用：sequence 全程只读，而 node_graphs 与 style_layer 求值时要写各自
    // 的求值缓存（NodeGraph::evaluate 与 ApplicationLayer::evaluate_matches 都收 &mut self）。
    let Project {
        sequence,
        library,
        style_layer,
        ..
    } = project;

    let style_results = evaluate_style_results(library)?;
    cache.begin_run(animation_library_text(library));

    let mut ass = ASSFile::new();
    let mut lines = Vec::new();
    ass.script_info.layout_res_x = frame_size.0;
    ass.script_info.layout_res_y = frame_size.1;
    ass.styles
        .insert(DEFAULT_STYLE.to_string(), StyleWithoutName::default());

    let mut seen = Vec::new();
    for (index, (item_id, event)) in visible_events(sequence).into_iter().enumerate() {
        let duration = event.end_time - event.start_time;
        if duration <= TimeDelta::zero() {
            continue;
        }
        let duration_ms = duration.num_milliseconds();
        let start_ms = event.start_time.num_milliseconds() as f64;

        let matches = style_layer.evaluate_matches(
            &LineContext {
                actor: actor_name(library, event),
                line_text: event.text.clone(),
                line_index: index as i64,
                item_id,
            },
            &style_results,
        )?;
        let merged = merge_matches(matches);
        let key = LineFitKey {
            style: merged.style.clone(),
            animation: merged.animation,
            duration_ms,
            frame_size,
            tolerance: opts.error_tolerance.to_bits(),
        };
        seen.push(item_id);
        let hit = cache
            .get(item_id, &key)
            .map(|entry| (entry.segments.clone(), entry.fallbacks));
        let (mut segments, fallbacks) = match hit {
            Some(hit) => hit,
            None => {
                let converted = match_to_ass_tags(
                    &merged,
                    &library.animation_library,
                    frame_size,
                    duration_ms as f64 / 1000.0,
                    opts,
                );
                let fallbacks = converted.fallbacks.len();
                cache.put(item_id, key, &converted.segments, fallbacks);
                (converted.segments, fallbacks)
            }
        };
        if fallbacks > 0 {
            log::warn!(
                "字幕行 {:?} 有 {fallbacks} 处动画无法表达成 ASS 标签，这些帧不进脚本",
                event.text
            );
        }

        let text = ass_text(&event.text);
        if segments.is_empty() {
            // 既无样式基线也无动画时 match_to_ass_tags 返回空段（animate_to_ass.rs:331），
            // 补一个覆盖整行的空标签段，这一行才不会从脚本里消失。
            segments.push(TimedTags {
                start_ms: 0.0,
                end_ms: duration_ms as f64,
                tags: Vec::new(),
            });
        }
        for segment in segments {
            let dialogue = dialogue(
                start_ms + segment.start_ms,
                start_ms + segment.end_ms,
                segment.tags,
                &text,
            );
            lines.push(LineHit::of(&dialogue, item_id));
            ass.events.push(Event::Dialogue(dialogue));
        }
    }

    cache.end_run(&seen);
    Ok(Converted { ass, lines })
}

/// 一次转换的产物：脚本，和一份与脚本里 `Dialogue` **同序、等长**的行索引。
///
/// 索引不进脚本、也不从脚本反解：导出只需要 `ass`，视口命中需要 `lines`。
pub struct Converted {
    pub ass: ASSFile,
    pub lines: Vec<LineHit>,
}

/// 一条 `Dialogue` 属于哪一行字幕，以及它在画面上可被辨认的那几点信息。
///
/// 视口要靠它把 libass 吐出的字形块配回具体某一行：块列表按 `(layer, 这里的下标)` 成段
/// 出现（见 `aweft_ass::render::Frame::tiles`），`anchor` 用来在段数对不上时兜底配位。
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct LineHit {
    pub item_id: ItemId,
    /// `Dialogue.layer`：libass 先按它给事件分组排序
    pub layer: i32,
    /// 生效区间，毫秒，取脚本里写进去的厘秒值（与 libass 的事件判定同一口径：
    /// `start <= now < end`）
    pub start_ms: i64,
    pub end_ms: i64,
    /// 这段带的 `\pos`（排版像素，锚点是**行框底部中点**）。没有就是 `None`：
    /// 位置由对齐边距定死，多行之间再由 libass 的碰撞挤开。
    pub anchor: Option<(f64, f64)>,
}

impl LineHit {
    fn of(dialogue: &Dialogue, item_id: ItemId) -> Self {
        let mut anchor = None;
        for segment in &dialogue.content {
            let TextSegment::Overrides(tags) = segment else {
                continue;
            };
            for tag in tags {
                if let StyleTag::Position(p) = tag {
                    anchor = Some((p.x, p.y));
                }
            }
        }
        Self {
            item_id,
            layer: dialogue.layer,
            start_ms: dialogue.start.as_centiseconds() * 10,
            end_ms: dialogue.end.as_centiseconds() * 10,
            anchor,
        }
    }
}

/// 一行拟合结果的键：`match_to_ass_tags` 读到的全部输入。
///
/// 不含行的起始时间——标签段是行内相对时间（`sequence_to_ass_with_cache` 加上起始），
/// 所以整行平移时这一项不变，缓存直接命中。动画只记引用 id，内容改动由
/// [`animation_library_text`] 那一层兜住。
#[derive(Debug, Clone, PartialEq)]
struct LineFitKey {
    style: StyleResult,
    animation: Option<AnimDataId>,
    /// 行时长，整毫秒。`match_to_ass_tags` 拿到的是它除以 1000，二者一一对应。
    duration_ms: i64,
    frame_size: (u32, u32),
    /// 误差容限的位型：拟合判据本身，换容限必须重拟合。
    tolerance: u64,
}

/// 一份缓存的行拟合：行内相对时间的标签段，加上转不出标签的帧数（只用于告警）。
struct CachedLineFit {
    key: LineFitKey,
    segments: Vec<TimedTags>,
    fallbacks: usize,
}

/// 跨多次 [`sequence_to_ass_with_cache`] 留存的逐行拟合结果。
///
/// 界面每动一次字幕就重转整个工程，而行与行的拟合互不影响（每行自配一个
/// `Cursor::from_style`，animate_to_ass.rs:363）。把这个 map 放在拟合线程上，改动一行
/// 就只重拟合那一行：其余行的键没变，直接复用。
///
/// 自我失效，调用方不需要清理：动画库内容变了整份丢掉（[`animation_library_text`]），
/// 样式、时长、帧尺寸、容限变了那一行的键对不上，行删了随 [`Self::end_run`] 剪掉。
#[derive(Default)]
pub struct SequenceFitCache {
    lines: HashMap<ItemId, CachedLineFit>,
    /// 上次拟合时的动画库内容；对不上就把整份缓存丢掉。
    library_text: String,
    /// 本次转换真正跑了拟合的行数，界面上「重拟合几行」读它。
    refits: usize,
}

impl SequenceFitCache {
    fn begin_run(&mut self, library_text: String) {
        if library_text != self.library_text {
            self.lines.clear();
        }
        self.library_text = library_text;
        self.refits = 0;
    }

    fn get(&self, item_id: ItemId, key: &LineFitKey) -> Option<&CachedLineFit> {
        self.lines.get(&item_id).filter(|entry| &entry.key == key)
    }

    fn put(&mut self, item_id: ItemId, key: LineFitKey, segments: &[TimedTags], fallbacks: usize) {
        self.refits += 1;
        self.lines.insert(
            item_id,
            CachedLineFit {
                key,
                segments: segments.to_vec(),
                fallbacks,
            },
        );
    }

    /// 丢掉本次没再出现的行：删掉的、隐藏掉的，还有粘贴出来的新 id 都从这里出清。
    fn end_run(&mut self, seen: &[ItemId]) {
        let seen: HashSet<ItemId> = seen.iter().copied().collect();
        self.lines.retain(|item_id, _| seen.contains(item_id));
    }

    /// 最近一次转换里实际重跑拟合的行数。等于行数就是全量重拟合，0 表示全部复用。
    pub fn refits(&self) -> usize {
        self.refits
    }
}

/// 动画库内容的判据。拟合读的就是这一份数据（`anim_datas` 与它引用的 `actions`），
/// 而行 id 在改关键帧时不变，所以只能按内容判新旧。
///
/// 取派生的 `Debug` 文本并整份留着逐字比对：`AnimationLibrary` 的两个容器都是
/// `BTreeMap`（按 id 有序，library.rs:18 就是为这个写的），成员类型一律 derive(Debug)，
/// 所以新增字段自动进判据，不会漏掉一类编辑，也不存在哈希撞车留下的旧结果。
fn animation_library_text(library: &ProjectLibrary) -> String {
    format!("{:?}", library.animation_library)
}

/// 预先求值所有样式链：应用图的 AppRef 节点按被引用图的求值结果取样式
/// （aweft-model/src/application_graph.rs:228）。
fn evaluate_style_results(
    library: &mut ProjectLibrary,
) -> Result<HashMap<NodeGraphId, StyleResult>, ApplicationError> {
    let mut results = HashMap::new();
    for (id, graph) in &mut library.node_graphs {
        results.insert(*id, graph.evaluate(&EvalContext::style())?);
    }
    Ok(results)
}

/// 前序遍历取出参与渲染的字幕行，顺序与行编辑器一致。
///
/// 只按字幕行自身的 `visible` 过滤：分组的 `visible` 目前只作用于分组头那一行
/// 自己的显示（aweft-gui/src/store/sequence_rows_model.rs:643,674），不向子项传播。
/// 带上 `ItemId` 是因为逐行拟合缓存按它存。
fn visible_events(sequence: &Sequence) -> Vec<(ItemId, &EventData)> {
    let mut out = Vec::new();
    collect_events(&sequence.root_items, sequence, &mut out);
    out
}

fn collect_events<'a>(
    ids: &'a [ItemId],
    sequence: &'a Sequence,
    out: &mut Vec<(ItemId, &'a EventData)>,
) {
    for id in ids {
        let Some(item) = sequence.get(*id) else {
            continue;
        };
        match &item.content {
            ItemContent::Event(event) if item.visible => out.push((*id, event)),
            ItemContent::Group(group) => collect_events(&group.children, sequence, out),
            _ => {}
        }
    }
}

/// 把命中的多张应用图合并成逐行的一份样式加一个动画引用。
///
/// 叠加顺序同 `ApplicationLayer::evaluate`（后者覆盖前者）。动画只取最后命中的那个：
/// 两段动画同时驱动同一属性时，ASS 标签表达不出它们的合成结果。
fn merge_matches(matches: Vec<ApplicationGraphMatch>) -> ApplicationGraphMatch {
    let mut merged = ApplicationGraphMatch::default();
    for matched in matches {
        merged.style = merged.style.merge_over(matched.style);
        if matched.animation.is_some() {
            merged.animation = matched.animation;
        }
    }
    merged
}

fn actor_name(library: &ProjectLibrary, event: &EventData) -> String {
    event
        .actor
        .and_then(|id| library.actor_name(id))
        .unwrap_or_default()
        .to_string()
}

/// 换行必须转成 `\N`：Dialogue 是行式格式，正文里出现真实换行会把脚本切断。
fn ass_text(text: &str) -> String {
    text.replace('\n', "\\N")
}

/// 毫秒取整到厘秒——ASS 时间戳的精度就是厘秒（`H:MM:SS.CC`）。
///
/// 相邻段共享边界不出字两次也不留缝：libass 的事件判定是
/// `Start <= now < Start + Duration`（ass_render.c:3384），段的尾巴是开区间。
fn ass_time(ms: f64) -> AssTime {
    AssTime::from_centiseconds((ms / 10.0).round() as i64)
}

fn dialogue(start_ms: f64, end_ms: f64, tags: Vec<StyleTag>, text: &str) -> Dialogue {
    let mut content = Vec::with_capacity(2);
    if !tags.is_empty() {
        content.push(TextSegment::Overrides(tags));
    }
    content.push(TextSegment::Text(text.to_string()));

    Dialogue {
        layer: 0,
        start: ass_time(start_ms),
        end: ass_time(end_ms),
        style: DEFAULT_STYLE.to_string(),
        name: None,
        margin_l: 0,
        margin_r: 0,
        margin_v: 0,
        effect: None,
        content,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use aweft_model::sequence::ItemLocation;

    fn event(start_ms: i64, end_ms: i64, text: &str) -> EventData {
        EventData {
            start_time: TimeDelta::milliseconds(start_ms),
            end_time: TimeDelta::milliseconds(end_ms),
            actor: None,
            text: text.to_string(),
            segment_timing: None,
            style: String::new(),
        }
    }

    /// 走完整链路：序列 → ASSFile → 脚本字节。断言全部针对字节，
    /// 因为预览交给 libass 的就是这份字节。
    fn script_text(project: &mut Project) -> String {
        let opts = ConvertOptions::default();
        let converted = sequence_to_ass(project, (1920, 1080), &opts).unwrap();
        String::from_utf8(converted.ass.to_bytes().unwrap()).unwrap()
    }

    fn dialogue_lines(text: &str) -> Vec<&str> {
        text.lines()
            .filter(|line| line.starts_with("Dialogue:"))
            .collect()
    }

    #[test]
    fn frame_size_and_row_times_reach_the_script() {
        let mut project = Project::new();
        project
            .sequence
            .create_event(event(1500, 4250, "第一行"), ItemLocation::Root(0))
            .unwrap();

        let text = script_text(&mut project);
        assert!(text.contains("PlayResX: 1920"), "{text}");
        assert!(text.contains("PlayResY: 1080"), "{text}");
        assert_eq!(
            dialogue_lines(&text),
            vec!["Dialogue:0,0:00:01.50,0:00:04.25,Default,,0,0,0,,第一行"],
        );
    }

    #[test]
    fn rows_are_collected_preorder_and_hidden_rows_are_out() {
        let mut project = Project::new();
        project
            .sequence
            .create_event(event(0, 1000, "根上"), ItemLocation::Root(0))
            .unwrap();
        let group = project
            .sequence
            .create_group(
                aweft_model::sequence::GroupData {
                    name: "组".to_string(),
                    collapsed: false,
                    color: None,
                    children: Vec::new(),
                },
                ItemLocation::Root(1),
            )
            .unwrap();
        project
            .sequence
            .create_event(event(1000, 2000, "组内"), ItemLocation::InGroup(group, 0))
            .unwrap();
        let hidden = project
            .sequence
            .create_event(event(2000, 3000, "隐藏"), ItemLocation::Root(2))
            .unwrap();
        project.sequence.get_mut(hidden).unwrap().visible = false;

        assert_eq!(
            dialogue_lines(&script_text(&mut project)),
            vec![
                "Dialogue:0,0:00:00.00,0:00:01.00,Default,,0,0,0,,根上",
                "Dialogue:0,0:00:01.00,0:00:02.00,Default,,0,0,0,,组内",
            ],
        );
    }

    /// 视口拖动将要生成的那两条链，端到端只作用在它绑定的那一行上。
    ///
    /// 同时验两件事：行身份（`LineContext.item_id` + `TextEq`）能不能锁定一行，以及求值
    /// 缓存是否换行作废——不作废时第二行会沿用第一行的命中结果，两行标签变得一样。
    #[test]
    fn a_per_line_chain_hits_only_its_own_line() {
        use aweft_model::application_graph::{ApplicationGraph, ApplicationNodeKind};
        use aweft_model::node_graph::{
            NodeGraph, NodeId, NodeKind, SocketDecl, SocketType, SocketValue, StyleOpKind,
        };

        let mut project = Project::new();
        let moved = project
            .sequence
            .create_event(event(0, 1_000, "甲"), ItemLocation::Root(0))
            .unwrap();
        let untouched = project
            .sequence
            .create_event(event(0, 1_000, "乙"), ItemLocation::Root(1))
            .unwrap();

        // 样式链：StyleSource → SetPosition(排版像素) → 组输出
        let mut chain = NodeGraph::new("视口位置");
        let source = chain
            .add_node::<NodeId>(NodeKind::StyleSource, &[], &[])
            .unwrap();
        let set_pos = chain
            .add_node::<NodeId>(NodeKind::StyleOp(StyleOpKind::SetPosition), &[], &[])
            .unwrap();
        let style_out = chain
            .add_node::<NodeId>(NodeKind::GroupOutput, &[], &[])
            .unwrap();
        chain
            .connect_raw(
                chain.nodes[&source].outputs[0],
                chain.nodes[&set_pos].inputs[0],
            )
            .unwrap();
        chain
            .connect_raw(
                chain.nodes[&set_pos].outputs[0],
                chain.nodes[&style_out].inputs[0],
            )
            .unwrap();
        chain
            .set_socket_value(
                chain.nodes[&set_pos].inputs[1],
                Some(SocketValue::Vec2 { x: 300.0, y: 400.0 }),
            )
            .unwrap();
        let chain_id = chain.id;
        project.library.node_graphs.insert(chain_id, chain);

        // 应用链：上下文 id 全等本行 → 条件；样式链引用 → 应用匹配 → 组输出
        let mut app = ApplicationGraph::new("视口位置");
        let ctx = app
            .add_node::<NodeId>(ApplicationNodeKind::ContextInput, &[], &[])
            .unwrap();
        let eq = app
            .add_node::<NodeId>(ApplicationNodeKind::TextEq, &[], &[])
            .unwrap();
        let style_ref = app
            .add_node::<NodeId>(
                ApplicationNodeKind::AppRef(chain_id),
                &[],
                &[SocketDecl::output("style", SocketType::Style)],
            )
            .unwrap();
        let apply = app
            .add_node::<NodeId>(ApplicationNodeKind::ApplyMatch, &[], &[])
            .unwrap();
        let app_out = app
            .add_node::<NodeId>(
                ApplicationNodeKind::GroupOutput,
                &[SocketDecl::input("application", SocketType::Application)],
                &[],
            )
            .unwrap();
        // 上下文第 4 个输出口是行身份
        app.connect_raw(app.nodes[&ctx].outputs[3], app.nodes[&eq].inputs[0])
            .unwrap();
        app.connect_raw(app.nodes[&eq].outputs[0], app.nodes[&apply].inputs[1])
            .unwrap();
        app.connect_raw(
            app.nodes[&style_ref].outputs[0],
            app.nodes[&apply].inputs[0],
        )
        .unwrap();
        app.connect_raw(app.nodes[&apply].outputs[0], app.nodes[&app_out].inputs[0])
            .unwrap();
        app.set_socket_value(
            app.nodes[&eq].inputs[1],
            Some(SocketValue::Text(moved.to_string())),
        )
        .unwrap();
        project.style_layer.graphs.push(app);

        let text = script_text(&mut project);
        let lines = dialogue_lines(&text);
        assert_eq!(lines.len(), 2, "{lines:?}");
        assert!(
            lines[0].contains(r"\pos(300,400)"),
            "绑定行该带上拖动位置：{}",
            lines[0]
        );
        assert!(
            !lines[1].contains(r"\pos("),
            "未绑定的行不该吃到这个位置：{}",
            lines[1]
        );

        // 视口命中靠这份索引认行：与脚本里的 Dialogue 同序等长，锚点就是写进标签的那个。
        let opts = ConvertOptions::default();
        let converted = sequence_to_ass(&mut project, (1920, 1080), &opts).unwrap();
        assert_eq!(converted.lines.len(), lines.len());
        assert_eq!(converted.lines[0].item_id, moved);
        assert_eq!(converted.lines[0].anchor, Some((300.0, 400.0)));
        assert_eq!(converted.lines[1].item_id, untouched);
        assert_eq!(converted.lines[1].anchor, None);
    }

    #[test]
    fn default_style_scales_are_percent() {
        let mut project = Project::new();
        project
            .sequence
            .create_event(event(0, 1000, "一行"), ItemLocation::Root(0))
            .unwrap();

        let text = script_text(&mut project);
        let style = text
            .lines()
            .find(|line| line.starts_with("Style: "))
            .expect("脚本里没有 Style 行");
        // Format 顺序：Name,Fontname,Fontsize,四种颜色,Bold,Italic,Underline,
        // StrikeOut,ScaleX,ScaleY,... → ScaleX 下标 11、ScaleY 下标 12
        let fields: Vec<&str> = style.trim_start_matches("Style: ").split(',').collect();
        assert_eq!(fields.get(11), Some(&"100"), "{style}");
        assert_eq!(fields.get(12), Some(&"100"), "{style}");
    }

    #[test]
    fn newline_in_row_text_becomes_ass_line_break() {
        let mut project = Project::new();
        project
            .sequence
            .create_event(event(0, 1000, "上\n下"), ItemLocation::Root(0))
            .unwrap();

        assert_eq!(
            dialogue_lines(&script_text(&mut project)),
            vec![r"Dialogue:0,0:00:00.00,0:00:01.00,Default,,0,0,0,,上\N下"],
        );
    }

    /// 带缓存的那条路，界面上的拟合线程走的就是它。
    fn script_text_cached(project: &mut Project, cache: &mut SequenceFitCache) -> String {
        let opts = ConvertOptions::default();
        let converted = sequence_to_ass_with_cache(project, (1920, 1080), &opts, cache).unwrap();
        String::from_utf8(converted.ass.to_bytes().unwrap()).unwrap()
    }

    /// 把一行的两端一起挪：时长不变，只有起始时间动。
    fn shift_row(project: &mut Project, id: ItemId, delta_ms: i64) {
        row_times(project, id, delta_ms, delta_ms);
    }

    fn row_times(project: &mut Project, id: ItemId, start_delta_ms: i64, end_delta_ms: i64) {
        let ItemContent::Event(row) = &mut project.sequence.get_mut(id).unwrap().content else {
            unreachable!()
        };
        row.start_time += TimeDelta::milliseconds(start_delta_ms);
        row.end_time += TimeDelta::milliseconds(end_delta_ms);
    }

    /// 三份行、一份缓存。第一遍每行都得拟合；之后只挪其中一行，一行都不必重拟合——
    /// 标签段是行内相对时间，起始时间在外面加上去。
    #[test]
    fn shifting_rows_reuses_every_fit() {
        let mut project = Project::new();
        let a = project
            .sequence
            .create_event(event(0, 1000, "甲"), ItemLocation::Root(0))
            .unwrap();
        let b = project
            .sequence
            .create_event(event(2000, 3500, "乙"), ItemLocation::Root(1))
            .unwrap();
        project
            .sequence
            .create_event(event(5000, 6000, "丙"), ItemLocation::Root(2))
            .unwrap();

        let mut cache = SequenceFitCache::default();
        script_text_cached(&mut project, &mut cache);
        assert_eq!(cache.refits(), 3, "第一遍没有可复用的结果");

        shift_row(&mut project, a, 500);
        shift_row(&mut project, b, -750);
        script_text_cached(&mut project, &mut cache);
        assert_eq!(cache.refits(), 0, "纯平移不该重拟合任何一行");
    }

    /// 缓存不能改变结果：热缓存下挪出来的脚本与从零算一遍逐字节相同。
    #[test]
    fn cached_script_is_the_same_script_as_a_cold_run() {
        let build = || {
            let mut project = Project::new();
            let id = project
                .sequence
                .create_event(event(0, 1000, "甲"), ItemLocation::Root(0))
                .unwrap();
            project
                .sequence
                .create_event(event(2000, 2500, "乙"), ItemLocation::Root(1))
                .unwrap();
            (project, id)
        };

        let (mut cold, id) = build();
        shift_row(&mut cold, id, 1500);
        let cold_text = script_text(&mut cold);

        let (mut warm, id) = build();
        let mut cache = SequenceFitCache::default();
        script_text_cached(&mut warm, &mut cache);
        shift_row(&mut warm, id, 1500);
        assert_eq!(script_text_cached(&mut warm, &mut cache), cold_text);
    }

    /// 只改一行的时长：那一行的键变了，其余行照常复用。
    #[test]
    fn resizing_one_row_refits_only_that_row() {
        let mut project = Project::new();
        let a = project
            .sequence
            .create_event(event(0, 1000, "甲"), ItemLocation::Root(0))
            .unwrap();
        project
            .sequence
            .create_event(event(2000, 3000, "乙"), ItemLocation::Root(1))
            .unwrap();

        let mut cache = SequenceFitCache::default();
        script_text_cached(&mut project, &mut cache);
        row_times(&mut project, a, 0, 800);
        script_text_cached(&mut project, &mut cache);
        assert_eq!(cache.refits(), 1, "改时长只重拟合被改的那一行");
    }

    /// 动画库内容变了，行自身的键一个都没动，也必须是全量重拟合：拟合读的就是这份
    /// 数据，而行 id 在改关键帧时不变。
    #[test]
    fn animation_library_edits_invalidate_every_row() {
        use aweft_model::animation::action::Action;
        use aweft_model::ids::ActionId;

        let mut project = Project::new();
        project
            .sequence
            .create_event(event(0, 1000, "甲"), ItemLocation::Root(0))
            .unwrap();
        project
            .sequence
            .create_event(event(2000, 3000, "乙"), ItemLocation::Root(1))
            .unwrap();

        let mut cache = SequenceFitCache::default();
        script_text_cached(&mut project, &mut cache);
        assert_eq!(cache.refits(), 2);

        project.library.animation_library.insert_action(Action {
            id: ActionId::new(),
            name: "新动作".to_string(),
            duration: 1.0,
            curves: Vec::new(),
        });
        script_text_cached(&mut project, &mut cache);
        assert_eq!(cache.refits(), 2, "动画库变了要逐行重来");
    }

    /// 不参与渲染的行要从缓存里出清，否则重新显示它会拿到一份没人要的结果；
    /// 邻居不受影响。
    #[test]
    fn hidden_rows_leave_the_cache() {
        let mut project = Project::new();
        project
            .sequence
            .create_event(event(0, 1000, "甲"), ItemLocation::Root(0))
            .unwrap();
        let hidden = project
            .sequence
            .create_event(event(2000, 3000, "乙"), ItemLocation::Root(1))
            .unwrap();

        let mut cache = SequenceFitCache::default();
        script_text_cached(&mut project, &mut cache);
        project.sequence.get_mut(hidden).unwrap().visible = false;
        script_text_cached(&mut project, &mut cache);
        assert_eq!(cache.refits(), 0, "藏起来的行的邻居不必重拟合");

        project.sequence.get_mut(hidden).unwrap().visible = true;
        script_text_cached(&mut project, &mut cache);
        assert_eq!(cache.refits(), 1, "重新显示的那一行要重拟合，另一行仍复用");
    }
}
