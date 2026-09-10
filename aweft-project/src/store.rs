use anyhow::{bail, Context, Result};
use aweft_model::ids::ItemId;
use aweft_model::project::Project;
use aweft_model::sequence::{Sequence, ItemContent};
use chrono::TimeDelta;
use std::path::Path;

use crate::document::{AweftDocument, AWEFT_DOCUMENT_FORMAT};
use crate::receipt::{EventTimeChanged, MutationKind, MutationReceipt, Origin};

/// 工程文档的唯一所有者。
/// 图形界面、命令行、插件、协作四方只持有对本存储的借用，
/// 所有已建模的写入经本存储单一入口。
///
/// 每次写入在 `pending` 里留一条回执（写日志），由持有者在一轮交互结束时用
/// [`ProjectStore::take_pending`] 取走并分发。回执不在写入瞬间同步回调：
/// 插件运行时会在收到通知后继续写本存储，同步回调等于在借用可变期间再次写入。
pub struct ProjectStore {
    doc: AweftDocument,
    pending: Vec<MutationReceipt>,
    dirty: bool,
}

impl ProjectStore {
    pub fn new(doc: AweftDocument) -> Self {
        Self {
            doc,
            pending: Vec::new(),
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
    /// 与 [`Self::sequence_mut`] 同一限制：这类写入尚无 `MutationKind`，不进写日志。
    pub fn project_mut(&mut self) -> &mut Project {
        &mut self.doc.project
    }

    pub fn sequence(&self) -> &Sequence {
        self.doc.project.sequence()
    }

    /// 直接改结构的时间线入口：只用于尚未建模为 `MutationKind` 的写入
    /// （增删行、层级、折叠、文本）。时间写入必须走本存储的 `set_*` / `adjust_*`，
    /// 否则插件与协作拿不到回执。
    pub fn sequence_mut(&mut self) -> &mut Sequence {
        self.doc.project.sequence_mut()
    }

    /// 自上次保存后是否有未落盘变更。
    pub fn is_dirty(&self) -> bool {
        self.dirty
    }

    /// 取走并清空写日志。调用方在一轮交互结束时取一次，逐条分发。
    pub fn take_pending(&mut self) -> Vec<MutationReceipt> {
        std::mem::take(&mut self.pending)
    }

    fn record(&mut self, receipt: MutationReceipt) {
        self.dirty = true;
        self.pending.push(receipt);
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
        let time = self.doc.project.sequence_mut().adjust_event_time(id, true, delta)?;
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
    use aweft_model::sequence::{EventData, ItemLocation};

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

    #[test]
    fn write_lands_in_pending_log_with_new_times() {
        let (mut store, ids) = store_with_events(&[(1000, 3000)]);
        store
            .set_event_start_ms(Origin::LocalGui, ids[0], 1500)
            .expect("写入失败");
        let pending = store.take_pending();
        assert_eq!(pending.len(), 1);
        assert_eq!(pending[0].origin, Origin::LocalGui);
        assert_eq!(time_of(&pending[0]), (1500, 3000));
        assert!(store.take_pending().is_empty(), "取走后队列应当清空");
        assert!(store.is_dirty());
    }

    #[test]
    fn set_start_clamps_at_end_time() {
        let (mut store, ids) = store_with_events(&[(1000, 3000)]);
        let time = store
            .set_event_start_ms(Origin::LocalGui, ids[0], 5000)
            .expect("写入失败");
        assert_eq!(time, TimeDelta::milliseconds(3000));
        assert_eq!(time_of(&store.take_pending()[0]), (3000, 3000));
    }

    #[test]
    fn clamped_noop_leaves_no_receipt() {
        let (mut store, ids) = store_with_events(&[(1000, 3000)]);
        store
            .set_event_start_ms(Origin::LocalGui, ids[0], 5000)
            .expect("写入失败");
        assert_eq!(store.take_pending().len(), 1);
        // 行已是 3000–3000，再要更远的开始时间被钳成同一个值：不该打扰插件
        store
            .set_event_start_ms(Origin::LocalGui, ids[0], 6000)
            .expect("写入失败");
        assert!(store.take_pending().is_empty(), "钳制后值未变不应记回执");
    }

    #[test]
    fn adjust_and_fuzzy_also_record() {
        let (mut store, ids) = store_with_events(&[(1000, 2000), (5000, 6000)]);
        store
            .adjust_event_time(
                Origin::LocalCli,
                ids[0],
                true,
                TimeDelta::milliseconds(1000),
            )
            .expect("平移失败");
        store
            .set_event_time_fuzzy(Origin::LocalGui, ids[1], false, "0:00:07.50")
            .expect("输入失败");
        let pending = store.take_pending();
        assert_eq!(pending.len(), 2);
        assert_eq!(time_of(&pending[0]), (2000, 2000));
        assert_eq!(time_of(&pending[1]), (5000, 7500));
    }

    #[test]
    fn pair_write_survives_short_row_and_skips_noop() {
        let (mut store, ids) = store_with_events(&[(1000, 1200)]);
        store
            .set_event_times_ms(Origin::Plugin("p".into()), ids[0], 2000, 2200)
            .expect("成对写入失败");
        assert_eq!(store.event_times_ms(ids[0]).expect("读回失败"), (2000, 2200));
        assert_eq!(time_of(&store.take_pending()[0]), (2000, 2200));

        store
            .set_event_times_ms(Origin::Plugin("p".into()), ids[0], 2000, 2200)
            .expect("重复写入失败");
        assert!(store.take_pending().is_empty(), "值没变不应记回执");
    }

    #[test]
    fn shift_moves_both_ends_without_clamping() {
        let (mut store, ids) = store_with_events(&[(500, 900)]);
        let (start, end) = store
            .shift_event_times(Origin::LocalGui, ids[0], TimeDelta::milliseconds(-1000))
            .expect("平移失败");
        assert_eq!(
            (start, end),
            (
                TimeDelta::milliseconds(-500),
                TimeDelta::milliseconds(-100)
            ),
            "整体平移不受钳制"
        );
        let pending = store.take_pending();
        assert_eq!(pending.len(), 1);
        assert_eq!(time_of(&pending[0]), (-500, -100));

        store
            .shift_event_times(Origin::LocalGui, ids[0], TimeDelta::zero())
            .expect("零位移调用失败");
        assert!(store.take_pending().is_empty(), "零位移不应记回执");
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
