//! 工程存储的协作订阅者：本地时间变更 → 待发编辑 → 交后台引擎发送。
//!
//! 与插件订阅者同一条规矩（见 `aweft-plugins/src/manager.rs` 的 `PluginSubscriber`）：
//! 回调发生在存储写入中途，这里只做纯读取与一次通道发送，绝不等网络、绝不写回存储。

use std::rc::Rc;

use aweft_project::{
    MutationKind, MutationReceipt, MutationSubscriber, Origin, ProjectStore, SubscriberRef,
};

use crate::cap::row_location;
use crate::engine::{CollabEngine, RemoteEdit};
use crate::wire::PendingEdit;

struct CollabSubscriber {
    origin: Origin,
    engine: Rc<CollabEngine>,
}

impl MutationSubscriber for CollabSubscriber {
    fn origin(&self) -> Origin {
        self.origin.clone()
    }

    fn on_mutations(&self, store: &mut ProjectStore, mutations: &[MutationReceipt]) {
        let mut edits = Vec::new();
        for receipt in mutations {
            // 远端来的写入一律不再转发。存储的按来源过滤只拦得下本机自己的标识，
            // 而对端落盘时用的是**写作者**的标识：照发的话 A 的变更会被 B 收下再发回，
            // 两端各签一条新操作、哈希不同、去重拦不住，一次拖动就放大成往返风暴。
            if matches!(receipt.origin, Origin::Collab(_)) {
                continue;
            }
            let MutationKind::EventTimeChanged(detail) = &receipt.kind;
            let Some((track, row)) = row_location(store, detail.item) else {
                // 行不在本地工程里：结构增删本版不同步，这一行的变更自然也就无从定位。
                log::warn!(
                    "协作跳过一次时间变更，本地无此行：{}（origin={:?}）",
                    detail.item,
                    receipt.origin
                );
                continue;
            };
            edits.push(PendingEdit {
                track: track.0.as_bytes().to_vec(),
                row: row.0.as_bytes().to_vec(),
                start_ms: detail.start_ms,
                end_ms: detail.end_ms,
            });
        }
        if edits.is_empty() {
            log::debug!("协作订阅者：本批无可发送变更（可能全是远端回声或行不在本地）");
            return;
        }
        log::info!("协作订阅者：发送 {} 条时间变更", edits.len());
        if let Err(error) = self.engine.publish(edits) {
            log::warn!("{error:?}");
        }
    }
}

/// 把一条已验算的远端变更写回真相。
///
/// 来源标记用**写作者**的标识，界面与日志因此看得出这一行是谁改的；订阅者按
/// `Origin::Collab(_)` 整体过滤，所以这次写入不会再绕回网络。
/// 行在本地不存在时只记日志：结构增删本版不同步，这属于预期内的分歧。
pub fn apply_remote(store: &mut ProjectStore, edit: &RemoteEdit) {
    if let Err(error) = store.set_event_times_ms(
        Origin::Collab(edit.author.clone()),
        edit.row,
        edit.start_ms,
        edit.end_ms,
    ) {
        log::warn!("远端变更落盘失败（行 {}）：{error:?}", edit.row);
    }
}

/// 把协作登记为 `store` 的订阅者。回声过滤用的来源标记取自引擎，两边必然一致。
pub fn register(store: &mut ProjectStore, engine: Rc<CollabEngine>) {
    let origin = engine.origin();
    let subscriber: SubscriberRef = Rc::new(CollabSubscriber { origin, engine });
    store.subscribe(subscriber);
}
