//! 动画域 reducer（统一 `AnimationLibrary`：Action 曲线 + NLA 轨道）。
//!
//! 数据域 `DataId::Animation` 单一，但 GUI 保留两类编辑器表现（曲线/摄影表 vs NLA），
//! 故本域聚合两类子 reducer：`graph`（Action 曲线）与 `nla`（轨道/片断）。
//! 顶层 `dispatch_animation` 按 `AnimationAction::Graph/Nla` 路由，返回精确 tag
//! 由单一 `Component`（`DataId::Animation`）的投影更新派生层。

pub mod graph;
pub mod nla;

use anyhow::Result;

use crate::actions::animation::AnimationAction;
use crate::depsgraph::Tag;

/// 动画域统一分发：路由到曲线或 NLA 子 reducer。
impl crate::store::AppStore {
    pub(crate) fn dispatch_animation(&mut self, action: AnimationAction) -> Result<Vec<Tag>> {
        match action {
            AnimationAction::Graph(g) => self.dispatch_graph_action(g),
            AnimationAction::Nla(n) => self.dispatch_nla_action(n),
        }
    }
}
