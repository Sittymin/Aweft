use anyhow::Result;
use aweft_model::ids::ItemId;
use aweft_project::{Origin, ProjectStore};

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
/// 时间读写先在快照上进行（保证 Guest 读到自己刚写的值），Guest 返回后再由
/// `flush_to_store` 差分写回工程存储；面板状态行与日志按插件实例长期保留。
#[derive(Debug, Default)]
pub(crate) struct HostCtx {
    pub rows: Vec<RowSnap>,
    pub status_lines: Vec<String>,
    pub log_lines: Vec<String>,
}

impl HostCtx {
    /// 从存储重建行快照。每次调用 Guest 导出函数前执行一次。
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

    /// 把快照与存储的差异写回存储。变更由存储记进写日志，本函数不返回回执。
    pub fn flush_to_store(&self, store: &mut ProjectStore, origin: &Origin) -> Result<()> {
        for row in &self.rows {
            store.set_event_times_ms(origin.clone(), row.id, row.start_ms, row.end_ms)?;
        }
        Ok(())
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
