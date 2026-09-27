use aweft_model::ids::ItemId;
use aweft_project::ProjectStore;
use futures::channel::mpsc::UnboundedSender;

use crate::worker::{PluginEvent, RowSnap, WorkerEvent};

/// wasmi `Store` 携带的宿主上下文。
///
/// Guest 的读写一律落在本快照上，不碰工程存储：一来 Guest 能立刻读到自己刚写的值，
/// 二来写回真相发生在 Guest 调用结束、插件借用释放之后——存储会在写入时广播给
/// 其它订阅者，而其中可能又是本插件，借用未释放就会撞成嵌套借用。
/// 从存储重建行快照（纯读，可在任意线程为 worker 打包调用）。
/// 调用方语义与旧 `sync_from_store` 一致：快照替换后上一批未写回的脏行即丢弃。
pub(crate) fn snapshot_rows(store: &ProjectStore) -> Vec<RowSnap> {
    let mut rows = Vec::new();
    for id in store.ordered_event_ids() {
        if let Ok((start_ms, end_ms)) = store.event_times_ms(id) {
            rows.push(RowSnap {
                id,
                start_ms,
                end_ms,
            });
        }
    }
    rows
}

#[derive(Debug, Default)]
pub(crate) struct HostCtx {
    pub rows: Vec<RowSnap>,
    /// Guest 写过哪几行（去重，按首次写入顺序）。写回真相时只取这些行。
    pub dirty: Vec<ItemId>,
    pub status_lines: Vec<String>,
    pub log_lines: Vec<String>,
    /// 进度直发通道：`set_status_line`/`log_text` 宿主函数在 Guest 调用中途
    /// 直接经此发送 `Progress`，不等调用返回。发送端是 `Send`，故本类型仍是
    /// `Send`，整个 `Store<HostCtx>` 可搬到 worker 线程。
    /// 通道由装载方（`PluginWorker::spawn`）在搬运前装好；测试里直接调宿主
    /// 函数时为 `None`，此时只写本地缓存。
    pub plugin_id: String,
    pub event_sink: Option<UnboundedSender<PluginEvent>>,
}

impl HostCtx {
    /// 把本次 `set_status_line`/`log_text` 的产出实时发给客户端。
    /// 跑在 worker 线程的 Guest 调用中途；无界发送不阻塞，接收端已关闭则丢弃。
    pub fn emit_progress(&self) {
        let Some(sink) = self.event_sink.clone() else {
            return;
        };
        let event = WorkerEvent::Progress {
            status_lines: self.status_lines.clone(),
            log_lines: self.log_lines.clone(),
        };
        if sink
            .unbounded_send((self.plugin_id.clone(), event))
            .is_err()
        {
            log::warn!("插件事件通道已关闭，进度丢弃（{}）", self.plugin_id);
        }
    }

    pub fn row_count(&self) -> usize {
        self.rows.len()
    }

    pub fn row_start(&self, handle: i32) -> Option<i64> {
        self.rows.get(handle as usize).map(|row| row.start_ms)
    }

    pub fn row_end(&self, handle: i32) -> Option<i64> {
        self.rows.get(handle as usize).map(|row| row.end_ms)
    }

    /// 行 id → 句柄。快照里没有这一行时返回 `None`。
    pub fn handle_of(&self, id: ItemId) -> Option<i32> {
        self.rows
            .iter()
            .position(|row| row.id == id)
            .map(|index| index as i32)
    }

    pub fn set_row_start(&mut self, handle: i32, ms: i64) -> bool {
        let Some(row) = self.rows.get_mut(handle as usize) else {
            return false;
        };
        row.start_ms = ms;
        let id = row.id;
        self.mark_dirty(id);
        true
    }

    pub fn set_row_end(&mut self, handle: i32, ms: i64) -> bool {
        let Some(row) = self.rows.get_mut(handle as usize) else {
            return false;
        };
        row.end_ms = ms;
        let id = row.id;
        self.mark_dirty(id);
        true
    }

    fn mark_dirty(&mut self, id: ItemId) {
        if !self.dirty.contains(&id) {
            self.dirty.push(id);
        }
    }

    /// 取走脏行的当前快照值并清空脏标记：交回调用方写进工程存储。
    pub fn take_dirty_rows(&mut self) -> Vec<RowSnap> {
        let dirty = std::mem::take(&mut self.dirty);
        dirty
            .into_iter()
            .filter_map(|id| self.rows.iter().find(|row| row.id == id).cloned())
            .collect()
    }

    /// 写面板状态行。行号超出已有长度时补空行；负数行号丢弃。
    pub fn set_status_line(&mut self, line: i32, text: String) {
        if line < 0 {
            return;
        }
        let index = line as usize;
        if self.status_lines.len() <= index {
            self.status_lines.resize(index + 1, String::new());
        }
        self.status_lines[index] = text;
    }
}
