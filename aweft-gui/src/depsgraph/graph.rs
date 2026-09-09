use std::collections::{HashMap, HashSet, VecDeque};
use std::sync::atomic::{AtomicU32, Ordering};

use crate::store::AppStore;

use super::{DataId, OperationCode, Tag, TagKind};

/// 投影组件唯一标识。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct ComponentId(u32);

impl ComponentId {
    /// 创建一个新的唯一组件 ID。
    pub fn new() -> Self {
        static NEXT: AtomicU32 = AtomicU32::new(1);
        Self(NEXT.fetch_add(1, Ordering::Relaxed))
    }
}

/// 操作节点唯一标识。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct OperationId(u32);

impl OperationId {
    /// 创建一个新的唯一操作 ID。
    pub fn new() -> Self {
        static NEXT: AtomicU32 = AtomicU32::new(1);
        Self(NEXT.fetch_add(1, Ordering::Relaxed))
    }
}
/// 操作执行函数：投影的一个原子更新动作。
pub type OperationExec = fn(&mut AppStore, &[Tag]);

/// 一个原子更新操作。
///
/// - `code`：操作唯一标识（编译期枚举，可无限扩展）。
/// - `handles`：声明处理哪些 `TagKind`。tag 变化时作为**入口操作**被命中，
///   然后沿出边 flush 到下游操作。
/// - `depends_on`：**显式依赖边**（Blender 的 Relation）。声明本操作的结果是
///   哪些操作的输入（`OperationCode` 编译期引用），注册时解析为 `OperationId`
///   出边。跨组件和组件内的顺序依赖都必须显式声明，不自动连边。
///   一次 flush 沿这些边算出完整闭包，联动在构建期建图、单轮求值
///   （无需"打新 tag 下一轮"）。
#[derive(Clone)]
pub struct Operation {
    pub id: OperationId,
    /// 操作唯一标识（编译期检查的枚举）。
    pub code: OperationCode,
    /// 处理的操作语义分类（入口）。
    pub handles: &'static [TagKind],
    /// 依赖的其他操作（本操作求值前这些操作必须先求值）。
    /// 用 `OperationCode` 编译期引用，`register` 后由 `resolve_relations`
    /// 解析为 OperationId。
    pub depends_on: &'static [OperationCode],
    pub exec: OperationExec,
}

/// 投影组件：一组操作 + 声明依赖的数据域。
/// 注册后组件仅保留元信息；操作本体移入 `Depsgraph::op_nodes`（图节点）。
pub struct Component {
    pub id: ComponentId,
    /// 依赖的数据域。任一被 tag，组件即被调度。
    pub depends_on: &'static [DataId],
    /// 组件内操作（注册时移入 op_nodes，声明顺序即组件内执行顺序）。
    pub operations: Vec<Operation>,
}

/// UI 依赖图。
pub struct Depsgraph {
    /// 图节点：操作本体（自持 exec，Blender 的 OperationNode）。
    /// 注册时从 `Component.operations` 移入，求值 O(1) 取 exec。
    op_nodes: HashMap<OperationId, Operation>,
    /// **入口操作索引**（Blender 的 entry tags）：(数据域, 操作语义) → 入口操作。
    /// 注册时按组件 depends_on × 操作 handles 预建，flush 时 O(1) 查表
    /// 直接拿到入口，无需运行时线性扫描。
    entry_ops: HashMap<(DataId, TagKind), Vec<OperationId>>,
    /// 操作查找表：OperationCode → OperationId。用于解析跨组件 depends_on。
    op_refs: HashMap<OperationCode, OperationId>,
    /// 操作 → 依赖它的操作（出边：本操作变化后受影响的下游）。
    /// 全部来自显式 `Operation::depends_on` 解析（跨组件 + 组件内显式声明）。
    outlinks: HashMap<OperationId, Vec<OperationId>>,
    /// 待解析的跨组件依赖边：(本操作, OperationCode)。
    pending_relations: Vec<(OperationId, OperationCode)>,
    /// 本次被 tag 的变更（evaluate 后清空）。
    pending_tags: Vec<Tag>,
}

impl Depsgraph {
    pub fn new() -> Self {
        Self {
            op_nodes: HashMap::new(),
            entry_ops: HashMap::new(),
            op_refs: HashMap::new(),
            outlinks: HashMap::new(),
            pending_relations: Vec::new(),
            pending_tags: Vec::new(),
        }
    }

    /// 注册一个组件：收集操作、建立数据依赖边、暂存跨组件 Relation。
    ///
    /// 两阶段：
    /// 1. 收集所有操作到 `op_refs`（供 depends_on 引用）；
    /// 2. `resolve_relations()` 解析 `depends_on` 的 OperationCode 为 OperationId 出边
    ///    （Blender 构建期建图）。
    ///
    /// 依赖只来自显式 `Operation::depends_on`——不按声明顺序自动连边。组件内
    /// 操作如需顺序依赖（如结构操作的结果供选中操作使用），也必须显式声明。
    pub fn register(&mut self, component: Component) {
        for op in &component.operations {
            self.op_refs.insert(op.code, op.id);
            for dep in op.depends_on {
                self.pending_relations.push((op.id, *dep));
            }
            // entry_ops 索引：组件依赖的每个数据域 × 操作处理的每个 kind →
            // 该操作是入口（Blender 的 entry tags，构建期预建）。
            for d in component.depends_on {
                for h in op.handles {
                    self.entry_ops.entry((*d, *h)).or_default().push(op.id);
                }
            }
            // 操作本体移入 op_nodes（图节点自持 exec，求值 O(1) 取用）。
            self.op_nodes.insert(op.id, op.clone());
        }
    }

    /// 解析跨组件依赖边（Blender 构建期建图）。
    ///
    /// `Operation::depends_on` 声明"本操作依赖哪个上游操作"，边方向为
    /// **上游 → 下游**（被依赖方 → 依赖方）：上游变化时 flush 沿此传播到
    /// 依赖它的下游。遍历 `pending_relations`，把 `OperationCode` 解析为
    /// `OperationId`，建立 `outlinks[上游].push(下游)`。
    /// 引用不存在的操作会 panic（声明错误应立即暴露）。
    pub fn resolve_relations(&mut self) {
        let relations = std::mem::take(&mut self.pending_relations);
        for (dependent, dep_code) in relations {
            // dependent 是依赖方（下游），dep_code 是被依赖方（上游）
            let upstream = self.op_refs.get(&dep_code).unwrap_or_else(|| {
                panic!(
                    "depsgraph: 操作 {:?} 依赖未注册的操作 {:?}",
                    dependent, dep_code
                )
            });
            self.outlinks.entry(*upstream).or_default().push(dependent);
        }
    }

    /// 标记一个数据变更。同一帧内可多次调用，evaluate 时合并处理。
    pub fn tag(&mut self, tag: Tag) {
        self.pending_tags.push(tag);
    }

    /// 本次调度是否有待处理的标记。
    pub fn has_pending(&self) -> bool {
        !self.pending_tags.is_empty()
    }

    /// 从 pending tags 计算受影响操作闭包（Blender flush）。
    ///
    /// 1. 每个 tag 的 `(data_id, kind)` → `entry_ops` **O(1) 查表**拿到入口操作
    ///    （构建期预建的 entry tags，无需运行时线性扫描）；
    /// 2. 从入口沿出边 BFS 扩散，收集所有受影响操作（闭包）。
    fn flush_affected(&self, tags: &[Tag]) -> HashSet<OperationId> {
        let mut affected: HashSet<OperationId> = HashSet::new();
        let mut queue: VecDeque<OperationId> = VecDeque::new();
        for tag in tags {
            if let Some(entries) = self.entry_ops.get(&(tag.data_id(), tag.kind())) {
                for &e in entries {
                    if affected.insert(e) {
                        queue.push_back(e);
                    }
                }
            }
        }
        // BFS 沿出边扩散（Blender flush_schedule_children）
        while let Some(op) = queue.pop_front() {
            if let Some(nexts) = self.outlinks.get(&op) {
                for &n in nexts {
                    if affected.insert(n) {
                        queue.push_back(n);
                    }
                }
            }
        }
        affected
    }

    /// 对一批操作做拓扑排序（依赖在前的先执行）。返回执行顺序。
    /// 忽略未注册的依赖与环（保留相对顺序）。
    fn topo_sort(&self, ops: &HashSet<OperationId>) -> Vec<OperationId> {
        let mut indeg: HashMap<OperationId, usize> = ops.iter().map(|&o| (o, 0)).collect();
        let mut adj: HashMap<OperationId, Vec<OperationId>> = HashMap::new();
        for &op in ops {
            if let Some(nexts) = self.outlinks.get(&op) {
                for &n in nexts {
                    if ops.contains(&n) {
                        adj.entry(op).or_default().push(n);
                        *indeg.entry(n).or_default() += 1;
                    }
                }
            }
        }
        // Kahn
        let queue: VecDeque<OperationId> = indeg
            .iter()
            .filter(|(_, d)| **d == 0)
            .map(|(&o, _)| o)
            .collect();
        // 稳定：按 id 排序保证确定性
        let mut qvec: Vec<OperationId> = queue.into_iter().collect();
        qvec.sort();
        let mut queue: VecDeque<OperationId> = qvec.into_iter().collect();
        let mut order = Vec::new();
        while let Some(op) = queue.pop_front() {
            order.push(op);
            if let Some(nexts) = adj.get(&op) {
                let mut nv: Vec<OperationId> = nexts.clone();
                nv.sort();
                for n in nv {
                    let e = indeg.get_mut(&n).unwrap();
                    *e -= 1;
                    if *e == 0 {
                        queue.push_back(n);
                    }
                }
            }
        }
        order
    }

    /// 执行调度：tag → 入口操作 → flush 闭包（含跨组件边）→ 拓扑执行。
    ///
    /// 单轮求值：所有受影响操作（含跨组件依赖）在一次 flush 中闭包完成，
    /// 一次拓扑执行。跨组件联动由构建期 `resolve_relations` 建的边驱动，
    /// 无需"操作打新 tag 下一轮"（Blender 的构建期建图 + 单轮求值）。
    pub fn evaluate(&mut self, store: &mut AppStore) {
        if self.pending_tags.is_empty() {
            return;
        }
        let tags = std::mem::take(&mut self.pending_tags);

        // 1. flush：data_id → 依赖组件 → handles 入口 → BFS 闭包（含跨组件边）
        let affected = self.flush_affected(&tags);
        if affected.is_empty() {
            self.reuse_tag_buffer(tags);
            return;
        }

        // 2. 拓扑排序（含跨组件依赖边）
        let order = self.topo_sort(&affected);

        // 3. 执行（从 op_nodes O(1) 取 exec，避免线性扫描）
        for op_id in order {
            if let Some(op) = self.op_nodes.get(&op_id) {
                (op.exec)(store, &tags);
            }
        }
        self.reuse_tag_buffer(tags);
    }

    /// 复用 tag 缓冲：逐帧 tick 的 tag 列表不再每轮堆分配。
    /// 投影按设计约束不产生新 tag；若执行中确有新 tag 到达则保留它们，
    /// 旧缓冲丢弃（正确性优先于复用）。
    fn reuse_tag_buffer(&mut self, mut tags: Vec<Tag>) {
        tags.clear();
        if self.pending_tags.is_empty() {
            self.pending_tags = tags;
        }
    }
}

impl Default for Depsgraph {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use aweft_model::ids::ItemId;
    use chrono::TimeDelta;

    fn make_component(
        data: &'static [DataId],
        handles: &'static [TagKind],
        exec: OperationExec,
    ) -> Component {
        let op_id = OperationId::new();
        Component {
            id: ComponentId::new(),
            depends_on: data,
            operations: vec![Operation {
                id: op_id,
                code: OperationCode::SequenceStructure,
                handles,
                depends_on: &[],
                exec,
            }],
        }
    }

    fn noop(_store: &mut AppStore, _tags: &[Tag]) {}

    /// 测试用的跨组件引用常量（&'static 数组）。
    const DEP_A: &[OperationCode] = &[OperationCode::SequenceStructure];
    const DEP_MISSING: &[OperationCode] = &[OperationCode::SequenceCollapse];

    /// 提取组件内第一个操作的 id。
    fn op_id(comp: &Component) -> OperationId {
        comp.operations[0].id
    }

    #[test]
    fn tag_routes_to_dependent_operations() {
        let mut g = Depsgraph::new();
        let a = make_component(&[DataId::Sequence], &[TagKind::SequenceTimecode], noop);
        let b = make_component(&[DataId::Selection], &[TagKind::Selection], noop);
        let id_a = op_id(&a);
        let id_b = op_id(&b);
        g.register(a);
        g.register(b);

        g.tag(Tag::EventTimecodeChanged {
            event_id: ItemId::new(),
            is_start_time: true,
            time: TimeDelta::seconds(1),
            ancestors: vec![],
        });

        let affected = g.flush_affected(&g.pending_tags);
        assert!(affected.contains(&id_a));
        assert!(!affected.contains(&id_b));
    }

    #[test]
    fn operation_deduplicated_across_multiple_tags() {
        let mut g = Depsgraph::new();
        let a = make_component(&[DataId::Sequence], &[TagKind::SequenceStructure], noop);
        let id_a = op_id(&a);
        g.register(a);

        g.tag(Tag::StructureChanged);
        g.tag(Tag::ItemsRemoved { item_ids: vec![] });

        let affected = g.flush_affected(&g.pending_tags);
        assert_eq!(affected.len(), 1);
        assert!(affected.contains(&id_a));
    }

    #[test]
    fn unrelated_data_does_not_trigger() {
        let mut g = Depsgraph::new();
        let a = make_component(&[DataId::Sequence], &[TagKind::SequenceStructure], noop);
        g.register(a);

        g.tag(Tag::ScreenInitialized);
        assert!(g.flush_affected(&g.pending_tags).is_empty());
    }

    #[test]
    fn no_implicit_propagation_within_component() {
        // 组件内依赖只来自显式 depends_on，声明顺序不产生隐式边。
        // a 与 b 同组件且 a 在前，但 b 不 handles 该 tag、无 depends_on → 不受影响。
        let mut g = Depsgraph::new();
        let a_op = OperationId::new();
        let b_op = OperationId::new();
        let comp = Component {
            id: ComponentId::new(),
            depends_on: &[DataId::Sequence],
            operations: vec![
                Operation {
                    id: a_op,
                    code: OperationCode::SequenceStructure,
                    handles: &[TagKind::SequenceStructure],
                    depends_on: &[],
                    exec: noop,
                },
                Operation {
                    id: b_op,
                    code: OperationCode::SequenceTimecode,
                    handles: &[TagKind::SequenceTimecode],
                    depends_on: &[],
                    exec: noop,
                },
            ],
        };
        g.register(comp);

        g.tag(Tag::StructureChanged);
        let affected = g.flush_affected(&g.pending_tags);
        assert!(affected.contains(&a_op));
        assert!(
            !affected.contains(&b_op),
            "组件内声明顺序不自动产生依赖边——b 未被 tag 命中、无 depends_on"
        );
    }

    #[test]
    fn explicit_depends_on_propagates_within_component() {
        // 组件内显式 depends_on：b 声明依赖 a，即使 b 不 handles 该 tag，
        // 也会沿显式边被传播到（与跨组件同一机制）。
        let mut g = Depsgraph::new();
        let a_op = OperationId::new();
        let b_op = OperationId::new();
        g.register(Component {
            id: ComponentId::new(),
            depends_on: &[DataId::Sequence],
            operations: vec![
                Operation {
                    id: a_op,
                    code: OperationCode::SequenceStructure,
                    handles: &[TagKind::SequenceStructure],
                    depends_on: &[],
                    exec: noop,
                },
                Operation {
                    id: b_op,
                    code: OperationCode::SequenceCollapse,
                    handles: &[],
                    depends_on: DEP_A,
                    exec: noop,
                },
            ],
        });
        g.resolve_relations();

        g.tag(Tag::StructureChanged);
        let affected = g.flush_affected(&g.pending_tags);
        assert!(affected.contains(&a_op));
        assert!(affected.contains(&b_op), "显式 depends_on 应把 b 拉入闭包");
    }

    #[test]
    fn cross_component_relation_flushes_to_dependent_operation() {
        // 跨组件显式依赖边（Blender Relation）：组件 B 的 op_b 声明依赖
        // 组件 A 的 op_a（OperationCode 引用）。op_b 不直接响应 tag（无 handles 命中），
        // 只应通过依赖边从 op_a 传播到——验证单轮 flush 沿跨组件边扩散。
        let mut g = Depsgraph::new();
        let a_op = OperationId::new();
        let b_op = OperationId::new();
        g.register(Component {
            id: ComponentId::new(),
            depends_on: &[DataId::Sequence],
            operations: vec![Operation {
                id: a_op,
                code: OperationCode::SequenceStructure,
                handles: &[TagKind::SequenceStructure],
                depends_on: &[],
                exec: noop,
            }],
        });
        g.register(Component {
            id: ComponentId::new(),
            depends_on: &[DataId::Sequence],
            operations: vec![Operation {
                id: b_op,
                code: OperationCode::SequenceCollapse,
                // 不 handles SequenceStructure：op_b 只应通过依赖边被传播到
                handles: &[],
                // 显式依赖 A 的操作（跨组件边）
                depends_on: DEP_A,
                exec: noop,
            }],
        });
        g.resolve_relations();

        // tag 命中 SequenceStructure → 入口为 op_a；跨组件边应把 op_b 拉入闭包
        g.tag(Tag::StructureChanged);
        let affected = g.flush_affected(&g.pending_tags);
        assert!(affected.contains(&a_op));
        assert!(
            affected.contains(&b_op),
            "跨组件 Relation 应使不直接响应的 op_b 进入闭包"
        );
    }

    #[test]
    fn cross_component_relation_schedules_only_dependent() {
        // 跨组件边只影响声明的依赖：op_b 依赖 op_a（会被传播到），
        // 而 op_c（同组件但无 handles 命中、无 depends_on）不被调度。
        let mut g = Depsgraph::new();
        let a_op = OperationId::new();
        let b_op = OperationId::new();
        let c_op = OperationId::new();
        g.register(Component {
            id: ComponentId::new(),
            depends_on: &[DataId::Sequence],
            operations: vec![Operation {
                id: a_op,
                code: OperationCode::SequenceStructure,
                handles: &[TagKind::SequenceStructure],
                depends_on: &[],
                exec: noop,
            }],
        });
        g.register(Component {
            id: ComponentId::new(),
            depends_on: &[DataId::Sequence],
            operations: vec![
                Operation {
                    id: b_op,
                    code: OperationCode::SequenceCollapse,
                    // 不 handles SequenceStructure：只应通过依赖边被传播到
                    handles: &[],
                    depends_on: DEP_A,
                    exec: noop,
                },
                Operation {
                    id: c_op,
                    code: OperationCode::SequenceTimecode,
                    // 不 handles、无 depends_on：不应被调度
                    handles: &[],
                    depends_on: &[],
                    exec: noop,
                },
            ],
        });
        g.resolve_relations();

        g.tag(Tag::StructureChanged);
        let affected = g.flush_affected(&g.pending_tags);
        assert!(affected.contains(&a_op));
        assert!(affected.contains(&b_op));
        assert!(
            !affected.contains(&c_op),
            "无关操作（无 handles 命中、无 depends_on）不应被调度"
        );
    }

    #[test]
    fn resolve_relations_panics_on_unknown_name() {
        // 依赖未注册的操作名应立即暴露（构建期错误，不是运行时静默）。
        let mut g = Depsgraph::new();
        let a_op = OperationId::new();
        g.register(Component {
            id: ComponentId::new(),
            depends_on: &[DataId::Sequence],
            operations: vec![Operation {
                id: a_op,
                code: OperationCode::SequenceStructure,
                handles: &[TagKind::SequenceStructure],
                depends_on: DEP_MISSING,
                exec: noop,
            }],
        });
        let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
            g.resolve_relations();
        }));
        assert!(result.is_err(), "未注册的操作名应 panic");
    }

    #[test]
    fn cross_component_topo_order_upstream_first() {
        // 跨组件依赖的执行顺序：op_b 依赖 op_a（上游），op_a 必须先执行。
        // Blender 语义：下游依赖上游的输出，拓扑保证上游在前。
        let mut g = Depsgraph::new();
        let a_op = OperationId::new();
        let b_op = OperationId::new();
        g.register(Component {
            id: ComponentId::new(),
            depends_on: &[DataId::Sequence],
            operations: vec![Operation {
                id: a_op,
                code: OperationCode::SequenceStructure,
                handles: &[TagKind::SequenceStructure],
                depends_on: &[],
                exec: noop,
            }],
        });
        g.register(Component {
            id: ComponentId::new(),
            depends_on: &[DataId::Sequence],
            operations: vec![Operation {
                id: b_op,
                code: OperationCode::SequenceCollapse,
                handles: &[TagKind::SequenceStructure],
                depends_on: DEP_A,
                exec: noop,
            }],
        });
        g.resolve_relations();

        g.tag(Tag::StructureChanged);
        let affected = g.flush_affected(&g.pending_tags);
        let order = g.topo_sort(&affected);
        let pos_a = order.iter().position(|&o| o == a_op).unwrap();
        let pos_b = order.iter().position(|&o| o == b_op).unwrap();
        assert!(pos_a < pos_b, "op_a 应先于 op_b 执行，顺序: {order:?}");
    }

    #[test]
    fn evaluate_executes_upstream_before_downstream() {
        // 最终验证：evaluate 真正按拓扑顺序执行操作（而非仅 topo_sort 输出）。
        // 用全局日志记录 exec 实际调用顺序，断言 op_a 在 op_b 前。
        use std::sync::Mutex;
        static EXEC_LOG: Mutex<Vec<&'static str>> = Mutex::new(Vec::new());

        fn exec_a(store: &mut AppStore, _tags: &[Tag]) {
            EXEC_LOG.lock().unwrap().push("op_a");
            let _ = store;
        }
        fn exec_b(store: &mut AppStore, _tags: &[Tag]) {
            EXEC_LOG.lock().unwrap().push("op_b");
            let _ = store;
        }

        let mut g = Depsgraph::new();
        let a_op = OperationId::new();
        let b_op = OperationId::new();
        g.register(Component {
            id: ComponentId::new(),
            depends_on: &[DataId::Sequence],
            operations: vec![Operation {
                id: a_op,
                code: OperationCode::SequenceStructure,
                handles: &[TagKind::SequenceStructure],
                depends_on: &[],
                exec: exec_a,
            }],
        });
        g.register(Component {
            id: ComponentId::new(),
            depends_on: &[DataId::Sequence],
            operations: vec![Operation {
                id: b_op,
                code: OperationCode::SequenceCollapse,
                handles: &[TagKind::SequenceStructure],
                depends_on: DEP_A,
                exec: exec_b,
            }],
        });
        g.resolve_relations();
        g.tag(Tag::StructureChanged);

        // 构造 AppStore（需要 MainWindow/testing 后端），替换 depsgraph 为测试图
        i_slint_backend_testing::init_no_event_loop();
        let instance = crate::bridge::types::MainWindow::new().unwrap();
        let mut store = crate::store::AppStore::new(&instance);
        store.depsgraph = g;

        store.evaluate_depsgraph();

        let log = EXEC_LOG.lock().unwrap();
        assert_eq!(
            log.as_slice(),
            &["op_a", "op_b"],
            "evaluate 应按拓扑顺序执行：op_a（上游）先，op_b（下游）后，实际: {log:?}"
        );
    }

    #[test]
    fn evaluate_reuses_tag_buffer() {
        // 逐帧 tick 不应每轮重建 pending Vec：求值后缓冲容量保留，供下一轮复用。
        // 组件 handles 该 tag，走完整 flush+topo+exec 路径。
        let mut g = Depsgraph::new();
        let a = make_component(&[DataId::Sequence], &[TagKind::SequenceStructure], noop);
        g.register(a);

        i_slint_backend_testing::init_no_event_loop();
        let instance = crate::bridge::types::MainWindow::new().unwrap();
        let mut store = crate::store::AppStore::new(&instance);
        store.depsgraph = g;

        store.depsgraph.tag(Tag::StructureChanged);
        store.evaluate_depsgraph();
        assert!(
            store.depsgraph.pending_tags.capacity() > 0,
            "求值后应保留 tag 缓冲容量"
        );

        // 第二轮复用同一缓冲：容量不收缩。
        let capacity = store.depsgraph.pending_tags.capacity();
        store.depsgraph.tag(Tag::StructureChanged);
        store.evaluate_depsgraph();
        assert_eq!(
            store.depsgraph.pending_tags.capacity(),
            capacity,
            "复用的缓冲容量应保持稳定"
        );
    }

    #[test]
    fn shared_data_id_drives_multiple_components() {
        // 数据依赖边：两个组件依赖同一数据域，一次 tag 同时驱动两者。
        // （Blender 中多组件依赖同一 IDNode 的语义）
        let mut g = Depsgraph::new();
        let a = make_component(&[DataId::Sequence], &[TagKind::SequenceStructure], noop);
        let b = make_component(&[DataId::Sequence], &[TagKind::SequenceStructure], noop);
        let id_a = op_id(&a);
        let id_b = op_id(&b);
        g.register(a);
        g.register(b);

        g.tag(Tag::StructureChanged);
        let affected = g.flush_affected(&g.pending_tags);
        assert!(affected.contains(&id_a));
        assert!(affected.contains(&id_b), "依赖同一数据域的组件都应被调度");
    }

    #[test]
    fn component_not_depending_on_data_id_skipped() {
        // 组件 depends_on 不含 tag.data_id 时，即使 kind 匹配也不调度。
        // （数据依赖边是调度的必要条件）
        let mut g = Depsgraph::new();
        // 组件只依赖 Selection，但 tag 是 Sequence 数据域
        let a = make_component(&[DataId::Selection], &[TagKind::SequenceStructure], noop);
        let id_a = op_id(&a);
        g.register(a);

        g.tag(Tag::StructureChanged);
        let affected = g.flush_affected(&g.pending_tags);
        assert!(
            affected.is_empty(),
            "不依赖 Sequence 的组件不应被 SequenceStructure tag 调度"
        );
        assert!(!affected.contains(&id_a));
    }
}
