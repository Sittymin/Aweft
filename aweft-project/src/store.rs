use anyhow::{bail, Context, Result};
use aweft_model::ids::ItemId;
use aweft_model::project::Project;
use aweft_model::sequence::{ItemContent, Sequence};
use chrono::TimeDelta;
use std::path::Path;

use crate::document::{AweftDocument, AWEFT_DOCUMENT_FORMAT};
use crate::receipt::{EventTimeChanged, MutationKind, MutationReceipt, Origin};
use crate::subscriber::SubscriberRef;

/// 一次广播最多推进的轮数。订阅者在通知里写回会再进队列，下一轮继续投；
/// 超过这个轮数按成环处理：丢弃剩余回执并记一条告警，不再进订阅者回调。
pub const MAX_BROADCAST_ROUNDS: usize = 8;

/// [`ProjectStore::begin_batch`] 的凭据。只有它能把批量作用域结束掉，
/// 因此一处 `begin_batch` 配一处 `end_batch`，不会串台。
#[derive(Debug)]
pub struct BatchToken {
    _priv: (),
}

/// 工程文档的唯一所有者。
/// 图形界面、命令行、插件、协作四方只持有对本存储的借用，
/// 所有已建模的写入经本存储单一入口。
///
/// 写入即广播：每次写入在 [`ProjectStore::record`] 里落成回执后立刻交给订阅者，
/// 分发规则（谁能收到、收到几条、什么时候收尾）全在这里，使用方不需要记得 pump。
/// 批量作用域（[`ProjectStore::begin_batch`]）把一段写入合成一次广播：
/// 一次拖动改 N 行应当只通知订阅者一次。
pub struct ProjectStore {
    doc: AweftDocument,
    /// 尚未广播出去的回执。广播循环按轮取空它，订阅者写回的新回执排在后面。
    pending: Vec<MutationReceipt>,
    subscribers: Vec<SubscriberRef>,
    /// >0 时广播推迟：批量作用域或广播循环自身持有。
    hold: usize,
    dirty: bool,
}

impl ProjectStore {
    pub fn new(doc: AweftDocument) -> Self {
        Self {
            doc,
            pending: Vec::new(),
            subscribers: Vec::new(),
            hold: 0,
            dirty: false,
        }
    }

    pub fn document(&self) -> &AweftDocument {
        &self.doc
    }

    /// 可变访问文档：仅初始播种（测试夹具/文件装载）使用。
    /// 正式写入必须走带回执的方法，否则其它接入方看不到这次变更。
    pub fn document_mut(&mut self) -> &mut AweftDocument {
        &mut self.doc
    }

    pub fn project(&self) -> &Project {
        &self.doc.project
    }

    /// 可变访问 `Project`（库表、样式层、字幕结构）。
    /// 与 [`Self::sequence_mut`] 同一限制：这类写入尚无 `MutationKind`，不产生回执。
    pub fn project_mut(&mut self) -> &mut Project {
        &mut self.doc.project
    }

    pub fn sequence(&self) -> &Sequence {
        self.doc.project.sequence()
    }

    /// 直接改结构的时间线入口：只用于尚未建模为 `MutationKind` 的写入
    /// （增删行、层级、折叠、文本）。时间写入必须走本存储的 `set_*` / `adjust_*`，
    /// 否则订阅者收不到这次变更。
    pub fn sequence_mut(&mut self) -> &mut Sequence {
        self.doc.project.sequence_mut()
    }

    /// 自上次保存后是否有未落盘变更。
    pub fn is_dirty(&self) -> bool {
        self.dirty
    }

    /// 注册订阅者。回调里对 `store` 的写入由本存储继续分发，因此订阅者之间
    /// 不需要彼此知道对方存在。
    pub fn subscribe(&mut self, subscriber: SubscriberRef) {
        self.subscribers.push(subscriber);
    }

    /// 开始一个批量作用域：其间所有写入只排队，到 [`Self::end_batch`] 才一次广播。
    pub fn begin_batch(&mut self) -> BatchToken {
        self.hold += 1;
        BatchToken { _priv: () }
    }

    /// 结束批量作用域，并广播其间攒下的回执。
    pub fn end_batch(&mut self, _token: BatchToken) {
        self.hold -= 1;
        self.broadcast();
    }

    fn record(&mut self, receipt: MutationReceipt) {
        self.dirty = true;
        self.pending.push(receipt);
        self.broadcast();
    }

    /// 把排队的回执按轮投给订阅者。订阅者在通知里写回的回执排进下一轮，
    /// 所以本函数只在栈顶跑一次：循环期间 `hold` 非零，写入方的嵌套调用直接返回。
    fn broadcast(&mut self) {
        if self.hold > 0 || self.pending.is_empty() {
            return;
        }
        self.hold += 1;
        // 取走注册表：回调要以 &mut self 写回真相，注册表不能同时被借用。
        let mut subs = std::mem::take(&mut self.subscribers);
        let mut rounds = 0;
        while !self.pending.is_empty() {
            rounds += 1;
            if rounds > MAX_BROADCAST_ROUNDS {
                log::warn!(
                    "变更广播达到 {MAX_BROADCAST_ROUNDS} 轮上限，丢弃剩余 {} 条回执",
                    self.pending.len()
                );
                self.pending.clear();
                break;
            }
            let batch = std::mem::take(&mut self.pending);
            for sub in &subs {
                let origin = sub.origin();
                let mine: Vec<MutationReceipt> = batch
                    .iter()
                    .filter(|receipt| receipt.origin != origin)
                    .cloned()
                    .collect();
                if !mine.is_empty() {
                    sub.on_mutations(self, &mine);
                }
            }
        }
        // 广播期间新注册的订阅者进的是空列表，合并回来再放回。
        subs.append(&mut self.subscribers);
        self.subscribers = subs;
        self.hold -= 1;
    }

    /// 读某行的起止时间（毫秒）。
    pub fn event_times_ms(&self, id: ItemId) -> Result<(i64, i64)> {
        let sequence = self.doc.project.sequence();
        let start = sequence.event_time(id, true)?;
        let end = sequence.event_time(id, false)?;
        Ok((start.num_milliseconds(), end.num_milliseconds()))
    }

    /// 确定性顺序的全部事件行 id：先按开始时间，再按 id 字符串。
    /// 插件侧数字句柄按此顺序编号，保证 host 与 guest 对齐。
    pub fn ordered_event_ids(&self) -> Vec<ItemId> {
        let sequence = self.doc.project.sequence();
        let mut rows: Vec<(i64, String, ItemId)> = sequence
            .items
            .iter()
            .filter_map(|(id, item)| match &item.content {
                ItemContent::Event(event) => {
                    Some((event.start_time.num_milliseconds(), id.to_string(), *id))
                }
                _ => None,
            })
            .collect();
        rows.sort_by(|a, b| a.0.cmp(&b.0).then_with(|| a.1.cmp(&b.1)));
        rows.into_iter().map(|(_, _, id)| id).collect()
    }

    /// 写完一个元素后记账：回执带该行完整的起止时间，接收方需要成对值。
    /// `before` 是写入前的成对值——钳制可能让写入什么也没改，
    /// 两端都没变时不记回执，否则插件会为一次无效写入收到通知。
    fn record_time_write(&mut self, origin: Origin, id: ItemId, before: (i64, i64)) -> Result<()> {
        let (start_ms, end_ms) = self.event_times_ms(id)?;
        if (start_ms, end_ms) == before {
            return Ok(());
        }
        self.record(MutationReceipt {
            origin,
            kind: MutationKind::EventTimeChanged(EventTimeChanged {
                item: id,
                start_ms,
                end_ms,
            }),
        });
        Ok(())
    }

    /// 设置某行开始时间的绝对值（毫秒）。
    /// 复用 `Sequence::adjust_event_time` 的钳制语义：开始时间不越过结束时间。
    /// 返回该元素写入后的实际值（钳制后可能不等于请求值）。
    pub fn set_event_start_ms(
        &mut self,
        origin: Origin,
        id: ItemId,
        target_ms: i64,
    ) -> Result<TimeDelta> {
        let before = self.event_times_ms(id)?;
        let delta = TimeDelta::milliseconds(target_ms - before.0);
        let time = self
            .doc
            .project
            .sequence_mut()
            .adjust_event_time(id, true, delta)?;
        self.record_time_write(origin, id, before)?;
        Ok(time)
    }

    /// 设置某行结束时间的绝对值（毫秒）。
    /// 复用 `Sequence::adjust_event_time` 的钳制语义：结束时间不早于开始时间。
    pub fn set_event_end_ms(
        &mut self,
        origin: Origin,
        id: ItemId,
        target_ms: i64,
    ) -> Result<TimeDelta> {
        let before = self.event_times_ms(id)?;
        let delta = TimeDelta::milliseconds(target_ms - before.1);
        let time = self
            .doc
            .project
            .sequence_mut()
            .adjust_event_time(id, false, delta)?;
        self.record_time_write(origin, id, before)?;
        Ok(time)
    }

    /// 同时设置某行起止时间（毫秒），供一次写回一行的整对值。
    /// 顺序必须先结束、后开始：`Sequence::adjust_event_time` 用另一端的当前值做
    /// 钳制，先写开始会把「时长小于平移量」的行截断（例如 1000–1200 平移 1s）。
    /// 两端都没变时不记回执。
    pub fn set_event_times_ms(
        &mut self,
        origin: Origin,
        id: ItemId,
        start_ms: i64,
        end_ms: i64,
    ) -> Result<()> {
        let (current_start, current_end) = self.event_times_ms(id)?;
        if end_ms != current_end {
            self.doc.project.sequence_mut().adjust_event_time(
                id,
                false,
                TimeDelta::milliseconds(end_ms - current_end),
            )?;
        }
        if start_ms != current_start {
            self.doc.project.sequence_mut().adjust_event_time(
                id,
                true,
                TimeDelta::milliseconds(start_ms - current_start),
            )?;
        }
        self.record_time_write(origin, id, (current_start, current_end))?;
        Ok(())
    }

    /// 某行起止时间同加 `delta`，不做钳制：时间线片段整体拖拽允许开始时间为负，
    /// 与 [`Self::adjust_event_time`] 的钳制语义不同。
    pub fn shift_event_times(
        &mut self,
        origin: Origin,
        id: ItemId,
        delta: TimeDelta,
    ) -> Result<(TimeDelta, TimeDelta)> {
        let before = self.event_times_ms(id)?;
        let item = self
            .doc
            .project
            .sequence_mut()
            .get_mut(id)
            .context("平移的字幕行不存在")?;
        let ItemContent::Event(event) = &mut item.content else {
            bail!("平移目标不是字幕行：{id:?}");
        };
        event.start_time += delta;
        event.end_time += delta;
        let times = (event.start_time, event.end_time);
        self.record_time_write(origin, id, before)?;
        Ok(times)
    }

    /// 按用户输入的文本设置某行时间（模糊解析，含钳制）。
    pub fn set_event_time_fuzzy(
        &mut self,
        origin: Origin,
        id: ItemId,
        is_start_time: bool,
        text: &str,
    ) -> Result<TimeDelta> {
        let before = self.event_times_ms(id)?;
        let time = self
            .doc
            .project
            .sequence_mut()
            .set_event_time_fuzzy(id, is_start_time, text)?;
        self.record_time_write(origin, id, before)?;
        Ok(time)
    }

    /// 某行时间平移 `delta`（含钳制：开始不越过结束、结束不早于开始）。
    pub fn adjust_event_time(
        &mut self,
        origin: Origin,
        id: ItemId,
        is_start_time: bool,
        delta: TimeDelta,
    ) -> Result<TimeDelta> {
        let before = self.event_times_ms(id)?;
        let time = self
            .doc
            .project
            .sequence_mut()
            .adjust_event_time(id, is_start_time, delta)?;
        self.record_time_write(origin, id, before)?;
        Ok(time)
    }

    /// 工程文件写盘（JSON 文本）。成功后清除脏标记。
    pub fn save_to_file(&mut self, path: &Path) -> Result<()> {
        let text = serde_json::to_string_pretty(&self.doc).context("工程文档转为文本失败")?;
        std::fs::write(path, text)
            .with_context(|| format!("工程文件写盘失败：{}", path.display()))?;
        self.dirty = false;
        Ok(())
    }

    /// 工程文件读回。格式版本不匹配时拒绝，不做静默兼容。
    pub fn load_from_file(path: &Path) -> Result<Self> {
        let text = std::fs::read_to_string(path)
            .with_context(|| format!("工程文件读取失败：{}", path.display()))?;
        let doc: AweftDocument = serde_json::from_str(&text).context("工程文件解析失败")?;
        if doc.format != AWEFT_DOCUMENT_FORMAT {
            bail!(
                "工程文件版本不受支持：文件 format={}，当前支持 format={}",
                doc.format,
                AWEFT_DOCUMENT_FORMAT
            );
        }
        Ok(Self::new(doc))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::subscriber::MutationSubscriber;
    use aweft_model::sequence::{EventData, ItemLocation};
    use std::cell::{Cell, RefCell};
    use std::rc::Rc;

    fn event_data(start_ms: i64, end_ms: i64) -> EventData {
        EventData {
            start_time: TimeDelta::milliseconds(start_ms),
            end_time: TimeDelta::milliseconds(end_ms),
            actor: None,
            text: String::new(),
            segment_timing: None,
            style: String::new(),
        }
    }

    fn store_with_events(times: &[(i64, i64)]) -> (ProjectStore, Vec<ItemId>) {
        let mut store = ProjectStore::new(AweftDocument::new());
        let mut ids = Vec::new();
        for (index, (start, end)) in times.iter().enumerate() {
            let id = store
                .doc
                .project
                .sequence_mut()
                .create_event(event_data(*start, *end), ItemLocation::Root(index))
                .expect("测试事件行创建失败");
            ids.push(id);
        }
        (store, ids)
    }

    fn time_of(receipt: &MutationReceipt) -> (i64, i64) {
        match &receipt.kind {
            MutationKind::EventTimeChanged(detail) => (detail.start_ms, detail.end_ms),
        }
    }

    /// 只记账的订阅者。它的 `origin` 既给自己写入打来源标记，
    /// 也决定它收不到哪种回执（回声过滤按值比对）。
    struct Recorder {
        origin: Origin,
        batches: RefCell<Vec<Vec<MutationReceipt>>>,
    }

    impl Recorder {
        fn new(origin: Origin) -> Rc<Self> {
            Rc::new(Self {
                origin,
                batches: RefCell::new(Vec::new()),
            })
        }

        fn receipts(&self) -> Vec<MutationReceipt> {
            self.batches.borrow().iter().flatten().cloned().collect()
        }

        fn calls(&self) -> usize {
            self.batches.borrow().len()
        }
    }

    impl MutationSubscriber for Recorder {
        fn origin(&self) -> Origin {
            self.origin.clone()
        }

        fn on_mutations(&self, _store: &mut ProjectStore, mutations: &[MutationReceipt]) {
            self.batches.borrow_mut().push(mutations.to_vec());
        }
    }

    /// 收到通知就把某行整体后移 100ms 的订阅者，用于测链式投递与轮数上限。
    struct Writer {
        origin: Origin,
        target: ItemId,
        steps: Cell<usize>,
        calls: Cell<usize>,
    }

    impl Writer {
        fn new(origin: Origin, target: ItemId, steps: usize) -> Rc<Self> {
            Rc::new(Self {
                origin,
                target,
                steps: Cell::new(steps),
                calls: Cell::new(0),
            })
        }

        fn calls(&self) -> usize {
            self.calls.get()
        }
    }

    impl MutationSubscriber for Writer {
        fn origin(&self) -> Origin {
            self.origin.clone()
        }

        fn on_mutations(&self, store: &mut ProjectStore, mutations: &[MutationReceipt]) {
            self.calls.set(self.calls.get() + 1);
            if mutations.is_empty() || self.steps.get() == 0 {
                return;
            }
            self.steps.set(self.steps.get() - 1);
            if let Ok((start, end)) = store.event_times_ms(self.target) {
                let shifted = store.set_event_times_ms(
                    self.origin.clone(),
                    self.target,
                    start + 100,
                    end + 100,
                );
                assert!(shifted.is_ok(), "订阅者写回应当成功");
            }
        }
    }

    #[test]
    fn write_broadcasts_before_the_write_returns() {
        let (mut store, ids) = store_with_events(&[(1000, 3000)]);
        let rec = Recorder::new(Origin::LocalCli);
        store.subscribe(Rc::clone(&rec) as SubscriberRef);
        store
            .set_event_start_ms(Origin::LocalGui, ids[0], 1500)
            .expect("写入失败");
        // 没有人 pump：回执在 set 返回前就已投出，带的是写入后的成对值。
        let receipts = rec.receipts();
        assert_eq!(receipts.len(), 1);
        assert_eq!(receipts[0].origin, Origin::LocalGui);
        assert_eq!(time_of(&receipts[0]), (1500, 3000));
        assert!(store.is_dirty());
    }

    #[test]
    fn writers_never_hear_their_own_writes() {
        let (mut store, ids) = store_with_events(&[(1000, 3000)]);
        let rec = Recorder::new(Origin::Plugin("p".into()));
        store.subscribe(Rc::clone(&rec) as SubscriberRef);
        store
            .set_event_start_ms(Origin::Plugin("p".into()), ids[0], 1500)
            .expect("自己写失败");
        assert_eq!(rec.calls(), 0, "回声不该投回写入者自己");
        store
            .set_event_start_ms(Origin::LocalGui, ids[0], 1600)
            .expect("外部写失败");
        assert_eq!(rec.receipts().len(), 1, "别人的写入要收到");
    }

    #[test]
    fn batch_scope_coalesces_writes_into_one_callback() {
        let (mut store, ids) = store_with_events(&[(1000, 3000), (5000, 6000)]);
        let rec = Recorder::new(Origin::LocalCli);
        store.subscribe(Rc::clone(&rec) as SubscriberRef);
        let batch = store.begin_batch();
        store
            .set_event_start_ms(Origin::LocalGui, ids[0], 1200)
            .expect("写第一行失败");
        store
            .set_event_start_ms(Origin::LocalGui, ids[1], 5200)
            .expect("写第二行失败");
        assert_eq!(rec.calls(), 0, "批量作用域期间不广播");
        store.end_batch(batch);
        assert_eq!(rec.calls(), 1, "一次批量只通知一次");
        assert_eq!(rec.receipts().len(), 2);
    }

    #[test]
    fn subscriber_write_back_reaches_the_others() {
        let (mut store, ids) = store_with_events(&[(1000, 2000)]);
        // recorder 与写入方同源，因此按回声过滤收不到那次外部写入，只收写回。
        let rec = Recorder::new(Origin::LocalGui);
        let writer = Writer::new(Origin::Plugin("w".into()), ids[0], 1);
        store.subscribe(Rc::clone(&rec) as SubscriberRef);
        store.subscribe(Rc::clone(&writer) as SubscriberRef);
        store
            .set_event_start_ms(Origin::LocalGui, ids[0], 1500)
            .expect("写入失败");
        // 同一次广播里继续排轮：writer 写回的那一行要在下一轮才投给 recorder。
        assert_eq!(writer.calls(), 1);
        let receipts = rec.receipts();
        assert_eq!(receipts.len(), 1, "recorder 只该收到 writer 的写回");
        assert_eq!(receipts[0].origin, Origin::Plugin("w".into()));
        assert_eq!(time_of(&receipts[0]), (1600, 2100));
    }

    #[test]
    fn runaway_pingpong_is_cut_at_round_cap() {
        let (mut store, ids) = store_with_events(&[(1000, 2000)]);
        let a = Writer::new(Origin::Plugin("a".into()), ids[0], usize::MAX);
        let b = Writer::new(Origin::Plugin("b".into()), ids[0], usize::MAX);
        store.subscribe(Rc::clone(&a) as SubscriberRef);
        store.subscribe(Rc::clone(&b) as SubscriberRef);
        store
            .set_event_start_ms(Origin::LocalGui, ids[0], 1500)
            .expect("写入失败");
        // 两个订阅者互相触发、永不定止：按轮上限切断，不递归、不 panic。
        assert_eq!(
            a.calls() + b.calls(),
            MAX_BROADCAST_ROUNDS * 2,
            "每个订阅者每轮各一次，共 {MAX_BROADCAST_ROUNDS} 轮"
        );
    }

    #[test]
    fn clamped_noop_does_not_broadcast() {
        let (mut store, ids) = store_with_events(&[(1000, 3000)]);
        let rec = Recorder::new(Origin::LocalCli);
        store.subscribe(Rc::clone(&rec) as SubscriberRef);
        store
            .set_event_start_ms(Origin::LocalGui, ids[0], 5000)
            .expect("写入失败");
        assert_eq!(time_of(&rec.receipts()[0]), (3000, 3000), "钳制到结束时间");
        // 行已是 3000–3000，再要更远的开始时间被钳成同一个值：不该打扰订阅者
        store
            .set_event_start_ms(Origin::LocalGui, ids[0], 6000)
            .expect("写入失败");
        assert_eq!(rec.calls(), 1, "钳制后值未变不应再广播");
    }

    #[test]
    fn adjust_and_fuzzy_also_broadcast() {
        let (mut store, ids) = store_with_events(&[(1000, 2000), (5000, 6000)]);
        let rec = Recorder::new(Origin::LocalCli);
        store.subscribe(Rc::clone(&rec) as SubscriberRef);
        store
            .adjust_event_time(
                Origin::LocalGui,
                ids[0],
                true,
                TimeDelta::milliseconds(1000),
            )
            .expect("平移失败");
        store
            .set_event_time_fuzzy(Origin::LocalGui, ids[1], false, "0:00:07.50")
            .expect("输入失败");
        let receipts = rec.receipts();
        assert_eq!(receipts.len(), 2);
        assert_eq!(time_of(&receipts[0]), (2000, 2000));
        assert_eq!(time_of(&receipts[1]), (5000, 7500));
    }

    #[test]
    fn pair_write_survives_short_row_and_skips_noop() {
        let (mut store, ids) = store_with_events(&[(1000, 1200)]);
        let rec = Recorder::new(Origin::LocalCli);
        store.subscribe(Rc::clone(&rec) as SubscriberRef);
        store
            .set_event_times_ms(Origin::Plugin("p".into()), ids[0], 2000, 2200)
            .expect("成对写入失败");
        assert_eq!(
            store.event_times_ms(ids[0]).expect("读回失败"),
            (2000, 2200)
        );
        assert_eq!(time_of(&rec.receipts()[0]), (2000, 2200));

        store
            .set_event_times_ms(Origin::Plugin("p".into()), ids[0], 2000, 2200)
            .expect("重复写入失败");
        assert_eq!(rec.receipts().len(), 1, "值没变不应记回执");
    }

    #[test]
    fn shift_moves_both_ends_without_clamping() {
        let (mut store, ids) = store_with_events(&[(500, 900)]);
        let rec = Recorder::new(Origin::LocalCli);
        store.subscribe(Rc::clone(&rec) as SubscriberRef);
        let (start, end) = store
            .shift_event_times(Origin::LocalGui, ids[0], TimeDelta::milliseconds(-1000))
            .expect("平移失败");
        assert_eq!(
            (start, end),
            (TimeDelta::milliseconds(-500), TimeDelta::milliseconds(-100)),
            "整体平移不受钳制"
        );
        let receipts = rec.receipts();
        assert_eq!(receipts.len(), 1);
        assert_eq!(time_of(&receipts[0]), (-500, -100));

        store
            .shift_event_times(Origin::LocalGui, ids[0], TimeDelta::zero())
            .expect("零位移调用失败");
        assert_eq!(rec.receipts().len(), 1, "零位移不应记回执");
    }

    #[test]
    fn save_and_load_roundtrip() {
        let (mut store, ids) = store_with_events(&[(1000, 2000)]);
        let path = std::env::temp_dir().join("aweft_project_store_test.aweft.json");
        store.save_to_file(&path).expect("写盘失败");
        assert!(!store.is_dirty());
        let loaded = ProjectStore::load_from_file(&path).expect("读回失败");
        let (start, end) = loaded.event_times_ms(ids[0]).expect("读回失败");
        assert_eq!((start, end), (1000, 2000));
        std::fs::remove_file(&path).ok();
    }

    #[test]
    fn load_rejects_unknown_format() {
        let path = std::env::temp_dir().join("aweft_project_store_bad.aweft.json");
        std::fs::write(
            &path,
            r#"{"format":999,"project":null,"layout":null,"media":[]}"#,
        )
        .expect("写测试文件失败");
        assert!(ProjectStore::load_from_file(&path).is_err());
        std::fs::remove_file(&path).ok();
    }
}
