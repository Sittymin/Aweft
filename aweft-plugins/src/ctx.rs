use aweft_model::ids::ItemId;
use aweft_project::ProjectStore;

/// Guest 可见的一行快照。句柄 = 本向量下标，宿主与 Guest 按
/// `ProjectStore::ordered_event_ids` 的确定性顺序对齐。
#[derive(Debug, Clone)]
pub(crate) struct RowSnap {
    pub id: ItemId,
    pub start_ms: i64,
    pub end_ms: i64,
}

/// wasmi `Store` 携带的宿主上下文。
///
/// Guest 的读写一律落在本快照上，不碰工程存储：一来 Guest 能立刻读到自己刚写的值，
/// 二来写回真相发生在 Guest 调用结束、插件借用释放之后——存储会在写入时广播给
/// 其它订阅者，而其中可能又是本插件，借用未释放就会撞成嵌套借用。
#[derive(Debug, Default)]
pub(crate) struct HostCtx {
    pub rows: Vec<RowSnap>,
    /// Guest 写过哪几行（去重，按首次写入顺序）。写回真相时只取这些行。
    pub dirty: Vec<ItemId>,
    pub status_lines: Vec<String>,
    pub log_lines: Vec<String>,
}

impl HostCtx {
    /// 从存储重建行快照。每次调用 Guest 导出前执行一次；未写回的脏行会随之丢弃，
    /// 所以调用方要先 `take_dirty_rows` 把上一批写回。
    pub fn sync_from_store(&mut self, store: &ProjectStore) {
        self.rows.clear();
        for id in store.ordered_event_ids() {
            if let Ok((start_ms, end_ms)) = store.event_times_ms(id) {
                self.rows.push(RowSnap {
                    id,
                    start_ms,
                    end_ms,
                });
            }
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
