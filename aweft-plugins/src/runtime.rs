use anyhow::{bail, Context, Result};
use aweft_model::ids::ItemId;
use futures::channel::mpsc::UnboundedSender;
use serde::Deserialize;
use wasmi::{Caller, Engine, Extern, Instance, Linker, Memory, Module, Store};

use crate::ctx::HostCtx;
use crate::manifest::Permissions;
use crate::worker::{PluginEvent, RowSnap, WorkerEvent};

/// Guest 导入的宿主模块名，与 `aweft_plugin_sdk` 的 `#[link]` 模块名一致。
pub const HOST_MODULE: &str = "aweft";

/// 受清单 `permissions.project_write` 管制的宿主函数。
/// 未授予时宿主根本不链接它们：插件想改真相必须在装载期就被看见，
/// 而不是运行期某条命令里悄悄改掉。
const WRITE_HOST_FUNCS: [&str; 2] = ["set_event_start_ms", "set_event_end_ms"];

/// 描述 JSON 的 `abi` 字段必须等于此值，否则整个插件拒绝装载。
pub const ABI_VERSION: u32 = 1;

/// 描述 JSON 的 `events` 里表示「行时间变更」订阅的名字。
pub const EVENT_TIME_CHANGED: &str = "time_changed";

/// 插件描述：命令标签（下标即命令编号）与订阅表。
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct GuestDescriptor {
    pub commands: Vec<String>,
    pub time_changed: bool,
}

#[derive(Deserialize)]
struct RawDescriptor {
    abi: u32,
    #[serde(default)]
    commands: Vec<String>,
    #[serde(default)]
    events: Vec<String>,
}

/// 解析 Guest 的描述 JSON。`abi` 不匹配直接拒绝，不做静默兼容。
pub fn parse_descriptor(text: &str) -> Result<GuestDescriptor> {
    let raw: RawDescriptor = serde_json::from_str(text).context("插件描述 JSON 解析失败")?;
    if raw.abi != ABI_VERSION {
        bail!(
            "插件描述版本不受支持：文件 abi={}，当前支持 abi={ABI_VERSION}",
            raw.abi
        );
    }
    Ok(GuestDescriptor {
        commands: raw.commands,
        time_changed: raw.events.iter().any(|event| event == EVENT_TIME_CHANGED),
    })
}

/// 已装载的单个插件实例：WASM Guest + 宿主快照上下文 + 解析出的描述。
/// 插件身份由装载方（`LoadedPlugin`）持有，本类型只跑字节码。
/// 成员内部类型：快照与写回的行类型不对使用方公开。
pub(crate) struct GuestPlugin {
    store: Store<HostCtx>,
    instance: Instance,
    descriptor: GuestDescriptor,
}

impl GuestPlugin {
    pub fn descriptor(&self) -> &GuestDescriptor {
        &self.descriptor
    }

    /// 面板状态行（插件自己写的文本，按行号排列）。
    pub fn status_lines(&self) -> Vec<String> {
        self.store.data().status_lines.clone()
    }

    /// 从 WASM 字节码装载插件并读出描述。
    /// 宿主入口按清单授予装配：未申请 `project_write` 的插件根本没有写真相的入口，
    /// 它 import 了就在装载期失败（见 `check_grants`）。
    /// 描述要求的导出（`aweft_on_command`、订阅时的 `aweft_on_event_time`）
    /// 缺失时也在这里报错，运行期不再检查。
    pub fn load(wasm_bytes: &[u8], permissions: &Permissions) -> Result<Self> {
        let engine = Engine::default();
        let mut store = Store::new(&engine, HostCtx::default());
        let module = Module::new(&engine, wasm_bytes).context("插件 WASM 模块解析失败")?;
        check_grants(&module, permissions)?;
        let mut linker = Linker::<HostCtx>::new(&engine);

        // 读真相：不改任何数据，不必申请。
        linker
            .func_wrap(HOST_MODULE, "event_count", |caller: Caller<HostCtx>| {
                caller.data().row_count() as i32
            })
            .context("宿主函数 event_count 注册失败")?;
        linker
            .func_wrap(
                HOST_MODULE,
                "event_start_ms",
                |caller: Caller<HostCtx>, handle: i32| -> i64 {
                    caller.data().row_start(handle).unwrap_or(-1)
                },
            )
            .context("宿主函数 event_start_ms 注册失败")?;
        linker
            .func_wrap(
                HOST_MODULE,
                "event_end_ms",
                |caller: Caller<HostCtx>, handle: i32| -> i64 {
                    caller.data().row_end(handle).unwrap_or(-1)
                },
            )
            .context("宿主函数 event_end_ms 注册失败")?;

        // 工具：时间双原语，不进真相，无需申请。
        // 时钟：进程启动起的单调毫秒（`Instant` 差值，不受墙钟校时回拨影响）。
        linker
            .func_wrap(HOST_MODULE, "now_ms", |_: Caller<HostCtx>| -> i64 {
                mono_ms()
            })
            .context("宿主函数 now_ms 注册失败")?;
        // 睡眠：阻塞的是 worker 线程，UI 线程照常转；极长的睡眠会拖住该插件
        // 后续的命令与通知（v1 无抢占，失控 Guest 的强杀是后续加固的事）。
        linker
            .func_wrap(HOST_MODULE, "sleep_ms", |_: Caller<HostCtx>, ms: i64| {
                let ms = ms.max(0) as u64;
                if ms > 0 {
                    std::thread::sleep(std::time::Duration::from_millis(ms));
                }
            })
            .context("宿主函数 sleep_ms 注册失败")?;
        // 输出：只写插件自己的面板状态行与诊断日志，进不了真相。
        // 每次写完实时发 `Progress`：worker 线程的 Guest 调用中途面板就能刷新，
        // 不必等调用返回（`emit_progress` 内是无界发送，不阻塞 Guest）。
        linker
            .func_wrap(
                HOST_MODULE,
                "set_status_line",
                |mut caller: Caller<HostCtx>, line: i32, ptr: i32, len: i32| {
                    let text = match read_caller_str(&caller, ptr, len) {
                        Ok(text) => text,
                        Err(e) => {
                            log::warn!("插件写入状态行失败：{e:?}");
                            return;
                        }
                    };
                    caller.data_mut().set_status_line(line, text);
                    caller.data().emit_progress();
                },
            )
            .context("宿主函数 set_status_line 注册失败")?;
        linker
            .func_wrap(
                HOST_MODULE,
                "log_text",
                |mut caller: Caller<HostCtx>, ptr: i32, len: i32| {
                    match read_caller_str(&caller, ptr, len) {
                        Ok(text) => {
                            caller.data_mut().log_lines.push(text);
                            caller.data().emit_progress();
                        }
                        Err(e) => log::warn!("插件日志读取失败：{e:?}"),
                    }
                },
            )
            .context("宿主函数 log_text 注册失败")?;

        // 写真相：清单申请过才链接。没链接时实例化会因找不到 import 而失败，
        // 那条失败由上面的 check_grants 提前变成指名道姓的报错。
        if permissions.project_write {
            linker
                .func_wrap(
                    HOST_MODULE,
                    "set_event_start_ms",
                    |mut caller: Caller<HostCtx>, handle: i32, ms: i64| -> i32 {
                        i32::from(!caller.data_mut().set_row_start(handle, ms))
                    },
                )
                .context("宿主函数 set_event_start_ms 注册失败")?;
            linker
                .func_wrap(
                    HOST_MODULE,
                    "set_event_end_ms",
                    |mut caller: Caller<HostCtx>, handle: i32, ms: i64| -> i32 {
                        i32::from(!caller.data_mut().set_row_end(handle, ms))
                    },
                )
                .context("宿主函数 set_event_end_ms 注册失败")?;
        }

        let instance = linker
            .instantiate_and_start(&mut store, &module)
            .context("插件实例化失败")?;

        let descriptor_text = read_guest_str_from(
            &mut store,
            &instance,
            "aweft_describe_ptr",
            "aweft_describe_len",
        )
        .context("插件缺少描述导出 aweft_describe_ptr/aweft_describe_len")?;
        let descriptor = parse_descriptor(&descriptor_text)?;
        let plugin = Self {
            store,
            instance,
            descriptor,
        };
        plugin.check_exports()?;
        Ok(plugin)
    }

    /// 装好事件直发通道（`PluginWorker::spawn` 在把本实例搬上 worker 线程前调一次）。
    pub(crate) fn set_event_sink(
        &mut self,
        plugin_id: String,
        sink: UnboundedSender<PluginEvent>,
    ) {
        let ctx = self.store.data_mut();
        ctx.plugin_id = plugin_id;
        ctx.event_sink = Some(sink);
    }

    /// 装入调用方打包好的行快照（worker 侧替代 `sync_from_store` 的入口）。
    /// 脏标记随之清空：上一批未写回的脏行即丢弃，调用方须先 `take_dirty_rows`。
    pub(crate) fn install_snapshot(&mut self, rows: Vec<RowSnap>) {
        let ctx = self.store.data_mut();
        ctx.rows = rows;
        ctx.dirty.clear();
    }

    /// 经直发通道发出一个事件。接收端已关闭时返回 `Err`，调用方据此结束 worker。
    pub(crate) fn emit(&self, event: WorkerEvent) -> Result<()> {
        let ctx = self.store.data();
        let Some(sink) = ctx.event_sink.clone() else {
            return Ok(());
        };
        sink.unbounded_send((ctx.plugin_id.clone(), event))
            .map_err(|_| anyhow::anyhow!("插件事件通道已关闭（{}）", ctx.plugin_id))?;
        Ok(())
    }

    /// 行 id → Guest 句柄（快照下标）。按 `ordered_event_ids` 的顺序编号。
    pub fn handle_of(&self, id: ItemId) -> Option<i32> {
        self.store.data().handle_of(id)
    }

    /// 取走 Guest 本次写过的行，交给调用方写回工程存储。
    pub fn take_dirty_rows(&mut self) -> Vec<RowSnap> {
        self.store.data_mut().take_dirty_rows()
    }

    /// 调用命令导出，返回 Guest 的状态码（0 表示成功）。
    /// 前提是已 `sync_from_store`；写入只落在快照上。
    pub fn call_command(&mut self, cmd_id: i32) -> Result<i32> {
        let code = self
            .command_func()?
            .call(&mut self.store, cmd_id)
            .with_context(|| format!("guest aweft_on_command({cmd_id}) 调用失败"))?;
        Ok(code)
    }

    /// 投递一次行时间变更。前提是已 `sync_from_store`，且 `handle` 由 `handle_of` 取得。
    pub fn call_time_changed(&mut self, handle: i32, start_ms: i64, end_ms: i64) -> Result<()> {
        self.instance
            .get_typed_func::<(i32, i64, i64), ()>(&self.store, "aweft_on_event_time")
            .context("缺少导出 aweft_on_event_time")?
            .call(&mut self.store, (handle, start_ms, end_ms))
            .context("guest aweft_on_event_time 调用失败")
    }

    /// 描述声明的能力必须有对应导出：声明了命令却没有 `aweft_on_command`
    /// 属于插件自身的错误，装载期就拒绝。
    fn check_exports(&self) -> Result<()> {
        if !self.descriptor.commands.is_empty() {
            self.command_func()?;
        }
        if self.descriptor.time_changed {
            self.instance
                .get_typed_func::<(i32, i64, i64), ()>(&self.store, "aweft_on_event_time")
                .context("描述订阅了 time_changed 但缺少导出 aweft_on_event_time")?;
        }
        Ok(())
    }

    fn command_func(&self) -> Result<wasmi::TypedFunc<i32, i32>> {
        self.instance
            .get_typed_func::<i32, i32>(&self.store, "aweft_on_command")
            .context("插件声明了命令但缺少导出 aweft_on_command")
    }
}

/// 宿主进程启动起的单调毫秒，供 `now_ms` 宿主函数。`Instant` 差值只增不减，
/// 与墙钟无关；Guest 侧 `std::time` 不可用，这是它唯一能看到的时间。
fn mono_ms() -> i64 {
    static BASE: std::sync::OnceLock<std::time::Instant> = std::sync::OnceLock::new();
    BASE.get_or_init(std::time::Instant::now)
        .elapsed()
        .as_millis() as i64
}

/// 清单没批的写能力，插件 import 了就不给装载。
/// 只查本宿主模块的 import：其余模块（如 WASI）找不到自然由实例化报错。
/// 少了这一步，未授权的插件只会得到一条"实例化失败：找不到 import"，
/// 看不出是自己没申请权限。
fn check_grants(module: &Module, permissions: &Permissions) -> Result<()> {
    if permissions.project_write {
        return Ok(());
    }
    for import in module.imports() {
        if import.module() != HOST_MODULE || !WRITE_HOST_FUNCS.contains(&import.name()) {
            continue;
        }
        bail!(
            "插件 import 了未申请的宿主函数 {}::{}：写真相需要清单 permissions 里的 project_write",
            import.module(),
            import.name()
        );
    }
    Ok(())
}

/// 读取 Guest 线性内存里的一段 UTF-8，供宿主函数在调用内使用。
fn read_caller_str(caller: &Caller<HostCtx>, ptr: i32, len: i32) -> Result<String> {
    let memory = match caller.get_export("memory") {
        Some(Extern::Memory(memory)) => memory,
        _ => bail!("插件未导出 memory"),
    };
    read_memory(&memory, caller, ptr, len)
}

fn read_memory(
    memory: &Memory,
    store: &impl wasmi::AsContext,
    ptr: i32,
    len: i32,
) -> Result<String> {
    if ptr < 0 || len < 0 {
        bail!("Guest 字符串指针或长度非法：ptr={ptr} len={len}");
    }
    let mut bytes = vec![0u8; len as usize];
    memory
        .read(store, ptr as usize, &mut bytes)
        .map_err(|e| anyhow::anyhow!("Guest 内存读取越界：{e:?}"))?;
    String::from_utf8(bytes).context("Guest 文本不是合法 UTF-8")
}

/// 先调用取指针/长度的两个导出，再按描述读字符串。
fn read_guest_str_from(
    store: &mut Store<HostCtx>,
    instance: &Instance,
    ptr_export: &str,
    len_export: &str,
) -> Result<String> {
    let ptr = call_export(store, instance, ptr_export)?;
    let len = call_export(store, instance, len_export)?;
    let memory = instance
        .get_memory(&*store, "memory")
        .context("插件未导出 memory")?;
    read_memory(&memory, store, ptr, len)
}

fn call_export(store: &mut Store<HostCtx>, instance: &Instance, name: &str) -> Result<i32> {
    let func = instance
        .get_typed_func::<(), i32>(&*store, name)
        .with_context(|| format!("缺少导出 {name}"))?;
    func.call(&mut *store, ())
        .with_context(|| format!("导出 {name} 调用失败"))
}

#[cfg(test)]
mod tests {
    /// `GuestPlugin` 必须能搬上 worker 线程：`Store<HostCtx>` 的 `Send` 来自
    /// wasmi 内部的显式实现 + `HostCtx` 全是 owned 数据（含事件发送端）。
    /// 若某天 `HostCtx` 里混进 `!Send` 的东西，这里先炸而不是在 `spawn` 处。
    #[test]
    fn guest_plugin_is_send_for_worker_threads() {
        fn assert_send<T: Send>() {}
        assert_send::<super::GuestPlugin>();
    }
}
