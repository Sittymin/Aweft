use anyhow::{bail, Context, Result};
// slint 的 `slint::ComponentFactory` 就是这一份的重导出（同一 rev），
// 交给宿主 `ComponentContainer` 时两边必须是同一个类型。
use i_slint_core::component_factory::ComponentFactory;
use i_slint_core::model::{ModelRc, VecModel};
use slint_interpreter::{
    CompilationResult, Compiler, ComponentDefinition, ComponentHandle, ComponentInstance,
    SharedString, Value, Weak,
};
use std::cell::RefCell;
use std::path::{Path, PathBuf};
use std::rc::Rc;

/// 面板必须提供的属性与回调名。宿主与插件片段之间只有这几项约定，
/// 宿主不知道任何具体功能，插件面板自己决定这些值渲染成什么控件。
/// `command-busy` 是可选的第 4 项：老面板没有它也能工作（此处 best-effort，
/// 属性缺失就忽略）；新面板用它置灰按钮——忙时点不点得由插件作者自己决定。
pub const COMMAND_LABELS_PROP: &str = "command-labels";
pub const STATUS_LINES_PROP: &str = "status-lines";
pub const COMMAND_CLICKED_CALLBACK: &str = "command-clicked";
pub const COMMAND_BUSY_PROP: &str = "command-busy";

fn result_to_definition(result: &CompilationResult) -> Result<ComponentDefinition> {
    if result.has_errors() {
        let messages: Vec<String> = result
            .diagnostics()
            .map(|diagnostic| diagnostic.to_string())
            .collect();
        bail!("插件面板 Slint 编译失败：{}", messages.join("\n"));
    }
    let mut components = result.components();
    let definition = components.next().context("插件面板 Slint 未产出任何组件")?;
    if components.next().is_some() {
        bail!("插件面板 Slint 必须只导出一个根组件");
    }
    Ok(definition)
}

/// 编译插件提供的面板 Slint 片段，返回组件定义。
/// 只做编译检查不实例化，因此单元测试无需显示服务器即可运行。
pub fn compile_panel(source: &str) -> Result<ComponentDefinition> {
    let compiler = Compiler::new();
    let result = spin_on::spin_on(
        compiler.build_from_source(source.to_string(), PathBuf::from("plugin-panel.slint")),
    );
    result_to_definition(&result)
}

/// 从文件编译插件面板。
pub fn compile_panel_file(path: &Path) -> Result<ComponentDefinition> {
    let source = std::fs::read_to_string(path)
        .with_context(|| format!("插件面板读取失败：{}", path.display()))?;
    compile_panel(&source)
}

fn string_list_value(items: &[String]) -> Value {
    let values: Vec<Value> = items
        .iter()
        .map(|item| Value::String(SharedString::from(item.as_str())))
        .collect();
    Value::Model(ModelRc::new(VecModel::from(values)))
}

/// 面板的值快照：新实例按它写属性，`sync` 更新它并扇出到活动实例。
struct PanelValues {
    labels: Vec<String>,
    lines: Vec<String>,
    busy: bool,
}

/// 一个插件面板的嵌入工厂，连同它的活动实例登记。
///
/// 面板不是独立窗口：工厂交给宿主的 `ComponentContainer`，容器每次实例化时回调
/// 进来现取现填——来回切 Area 会销毁重建容器，捕获创建时的快照会让切回来的
/// Area 显示旧状态。同一插件可以在两个 Area 同时打开，所以推新状态要扇出到
/// 它的所有活动实例（弱句柄，销毁掉的在这里顺手清掉）。
pub struct PanelHandle {
    values: Rc<RefCell<PanelValues>>,
    instances: Rc<RefCell<Vec<Weak<ComponentInstance>>>>,
    factory: ComponentFactory,
}

impl PanelHandle {
    /// 只做登记，不实例化：真正的面板实例由容器按需向 `factory` 索取。
    pub fn new(
        definition: &ComponentDefinition,
        labels: &[String],
        lines: &[String],
        busy: bool,
        on_command: impl Fn(i32) + 'static,
    ) -> Self {
        let values = Rc::new(RefCell::new(PanelValues {
            labels: labels.to_vec(),
            lines: lines.to_vec(),
            busy,
        }));
        let instances = Rc::new(RefCell::new(Vec::new()));
        let on_command = Rc::new(on_command);
        let factory = {
            let values = Rc::clone(&values);
            let instances = Rc::clone(&instances);
            let definition = definition.clone();
            ComponentFactory::new(move |ctx| {
                let Ok(instance) = definition.create_embedded(ctx) else {
                    log::warn!("插件面板嵌入实例化失败");
                    return None;
                };
                {
                    let snapshot = values.borrow();
                    let _ = instance
                        .set_property(COMMAND_LABELS_PROP, string_list_value(&snapshot.labels));
                    let _ = instance
                        .set_property(STATUS_LINES_PROP, string_list_value(&snapshot.lines));
                    // 老面板没有这一项，属性不存在时 set 报错，忽略即可。
                    let _ = instance.set_property(COMMAND_BUSY_PROP, Value::Bool(snapshot.busy));
                }
                let sink = Rc::clone(&on_command);
                if let Err(error) = instance.set_callback(COMMAND_CLICKED_CALLBACK, move |args| {
                    let command_id = match args.first() {
                        Some(Value::Number(index)) => *index as i32,
                        other => {
                            log::warn!("面板回调 {COMMAND_CLICKED_CALLBACK} 参数异常：{other:?}");
                            return Value::Void;
                        }
                    };
                    sink(command_id);
                    Value::Void
                }) {
                    log::warn!("面板回调 {COMMAND_CLICKED_CALLBACK} 接线失败：{error:?}");
                }
                instances.borrow_mut().push(instance.as_weak());
                Some(instance)
            })
        };
        Self {
            values,
            instances,
            factory,
        }
    }

    /// 交给 UI 的那一份工厂。克隆同一个值指针相等，Slint 侧据此判断工厂没变，
    /// 不会因为投影重算而重建容器里的面板。
    pub fn factory(&self) -> ComponentFactory {
        self.factory.clone()
    }

    /// 推新状态：先更新快照（下次实例化就是新值），再扇出到现存实例。
    pub fn sync(&self, lines: &[String], busy: bool) {
        {
            let mut snapshot = self.values.borrow_mut();
            snapshot.lines = lines.to_vec();
            snapshot.busy = busy;
        }
        self.instances
            .borrow_mut()
            .retain_mut(|instance| match instance.upgrade() {
                None => false,
                Some(instance) => {
                    let _ = instance.set_property(STATUS_LINES_PROP, string_list_value(lines));
                    let _ = instance.set_property(COMMAND_BUSY_PROP, Value::Bool(busy));
                    true
                }
            });
    }

    /// 还活着的面板实例数（登记里的死句柄顺手清掉）。
    /// 宿主用它确认"切走 Area 即释放面板"。
    pub fn live_instances(&self) -> usize {
        let mut instances = self.instances.borrow_mut();
        instances.retain(|instance| instance.upgrade().is_some());
        instances.len()
    }
}
