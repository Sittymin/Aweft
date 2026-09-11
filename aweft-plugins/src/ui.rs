use anyhow::{Context, Result, bail};
use i_slint_core::model::{ModelRc, VecModel};
use slint_interpreter::{
    CompilationResult, ComponentDefinition, ComponentHandle, ComponentInstance, Compiler,
    SharedString, Value,
};
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

/// 已打开的插件窗口守卫：持有实例即保持窗口存活，丢弃则关闭。
pub struct PluginWindow {
    instance: Rc<ComponentInstance>,
}

impl PluginWindow {
    /// 实例化面板、接线并显示。`on_command` 收到面板按钮点击时的命令编号，
    /// 在 Slint 事件循环线程执行，必须是快速同步逻辑。
    pub fn open(
        definition: &ComponentDefinition,
        command_labels: &[String],
        status_lines: &[String],
        on_command: impl Fn(i32) + 'static,
    ) -> Result<Self> {
        let instance = Rc::new(definition.create().context("插件面板实例化失败")?);
        let window = Self { instance };
        window.set_command_labels(command_labels);
        window.set_status_lines(status_lines);

        window
            .instance
            .set_callback(COMMAND_CLICKED_CALLBACK, move |args| {
                let command_id = match args.first() {
                    Some(Value::Number(index)) => *index as i32,
                    other => {
                        log::warn!("面板回调 {COMMAND_CLICKED_CALLBACK} 参数异常：{other:?}");
                        return Value::Void;
                    }
                };
                on_command(command_id);
                Value::Void
            })
            .with_context(|| format!("面板回调 {COMMAND_CLICKED_CALLBACK} 接线失败"))?;

        window.instance.show().context("插件面板显示失败")?;
        Ok(window)
    }

    pub fn set_command_labels(&self, labels: &[String]) {
        let _ = self
            .instance
            .set_property(COMMAND_LABELS_PROP, string_list_value(labels));
    }

    pub fn set_status_lines(&self, lines: &[String]) {
        let _ = self
            .instance
            .set_property(STATUS_LINES_PROP, string_list_value(lines));
    }

    /// 设置面板忙态（命令执行中）。老面板没有该属性时静默忽略，
    /// 因此宿主侧升级不强制插件作者改面板。
    pub fn set_busy(&self, busy: bool) {
        let _ = self
            .instance
            .set_property(COMMAND_BUSY_PROP, Value::Bool(busy));
    }

    /// 用户关掉窗口后为 false：图形界面据此决定重新显示还是新建。
    pub fn is_visible(&self) -> bool {
        self.instance.window().is_visible()
    }
}
