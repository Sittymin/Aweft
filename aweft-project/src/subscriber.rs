use crate::{MutationReceipt, Origin, ProjectStore};
use std::rc::Rc;

/// 工程存储的变更订阅者。分发点唯一在 `ProjectStore`，订阅者是谁、彼此什么关系，
/// 存储一概不认识：图形界面刷新、插件通知、以后的远端合流走同一入口。
///
/// 回调用 `&self` + 内部可变，是因为广播期间存储要把自己可变地交给回调，
/// 同时持有注册表；订阅者自己的状态由它用 `RefCell` 等自持。
///
/// 回调里可以继续写 `store`：写入进同一轮广播的队列，排完后陆续投给其余订阅者
/// （包括写它的那位自己），因此不会嵌套重入本回调。
pub trait MutationSubscriber {
    /// 本订阅者写入时用的来源标记。存储按它做回声过滤：与回执来源相同的订阅者
    /// 收不到那次写入，所以插件不会为「自己刚写的一行」被再通知一次。
    fn origin(&self) -> Origin;

    /// 一批变更。`store` 就是本订阅者注册到的那个存储。
    fn on_mutations(&self, store: &mut ProjectStore, mutations: &[MutationReceipt]);
}

/// 订阅者句柄。单线程模型，与 GUI 的 `Rc<RefCell<_>>` 一致。
pub type SubscriberRef = Rc<dyn MutationSubscriber>;
