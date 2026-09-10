//! 界面侧的工程变更订阅者。
//!
//! 其它接入方（插件、命令行，以后的协作）写进 `ProjectStore` 的时间变更要在界面上刷出来。
//! 存储写入即广播，而广播发生在 `AppStore` 已被可变借用的中途：这时回头借同一个
//! `RefCell<AppStore>`、或直接改 `depsgraph`，都会撞成借用冲突。所以本订阅者只把回执
//! 排进队列，等本轮 `dispatch` 收尾时由 `AppStore` 自己取走、换成刷新 tag。
//!
//! 回声过滤顺带免掉重复刷新：界面自己的写入（同为 `Origin::LocalGui`）不会绕回本队列，
//! 那部分的 tag 由 reducer 当场打。

use std::cell::RefCell;
use std::collections::HashMap;
use std::rc::Rc;

use aweft_model::ids::ItemId;
use aweft_project::{
    MutationKind, MutationReceipt, MutationSubscriber, Origin, ProjectStore, SubscriberRef,
};
use chrono::TimeDelta;

use crate::depsgraph::Tag;
use crate::store::AppStore;

/// 排队句柄：订阅者往里放，`AppStore` 在回合末取走。两边共用同一块内存。
#[derive(Clone, Default)]
pub(crate) struct RefreshQueue {
    receipts: Rc<RefCell<Vec<MutationReceipt>>>,
}

impl RefreshQueue {
    /// 取走排队中的回执并清空队列。
    pub(crate) fn take(&self) -> Vec<MutationReceipt> {
        std::mem::take(&mut *self.receipts.borrow_mut())
    }
}

struct GuiSubscriber {
    queue: RefreshQueue,
}

impl MutationSubscriber for GuiSubscriber {
    fn origin(&self) -> Origin {
        Origin::LocalGui
    }

    fn on_mutations(&self, _store: &mut ProjectStore, mutations: &[MutationReceipt]) {
        self.queue
            .receipts
            .borrow_mut()
            .extend(mutations.iter().cloned());
    }
}

/// 把界面登记为 `store` 的订阅者，返回排队句柄。
pub(crate) fn register(store: &mut ProjectStore) -> RefreshQueue {
    let queue = RefreshQueue::default();
    let subscriber: SubscriberRef = Rc::new(GuiSubscriber {
        queue: queue.clone(),
    });
    store.subscribe(subscriber);
    queue
}

/// 把一批回执换成刷新 tag：每行取最后一次写入的起止值，各打一条 start/end tag。
pub(crate) fn refresh_tags(store: &AppStore, receipts: &[MutationReceipt]) -> Vec<Tag> {
    let mut order: Vec<ItemId> = Vec::new();
    let mut latest: HashMap<ItemId, (i64, i64)> = HashMap::new();
    for receipt in receipts {
        let MutationKind::EventTimeChanged(detail) = &receipt.kind;
        if !latest.contains_key(&detail.item) {
            order.push(detail.item);
        }
        latest.insert(detail.item, (detail.start_ms, detail.end_ms));
    }
    order
        .into_iter()
        .flat_map(|id| {
            let (start_ms, end_ms) = latest[&id];
            [
                store.event_timecode_tag(id, true, TimeDelta::milliseconds(start_ms)),
                store.event_timecode_tag(id, false, TimeDelta::milliseconds(end_ms)),
            ]
        })
        .collect()
}
