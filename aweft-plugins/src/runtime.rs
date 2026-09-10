use anyhow::{Context, Result, bail};
use aweft_project::{Origin, ProjectStore};
use serde::Deserialize;
use wasmi::{Caller, Engine, Extern, Instance, Linker, Memory, Module, Store};

use crate::ctx::HostCtx;

/// Guest 导入的宿主模块名，与 `aweft_plugin_sdk` 的 `#[link]` 模块名一致。
pub const HOST_MODULE: &str = "aweft";

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
pub struct GuestPlugin {
    id: String,
    store: Store<HostCtx>,
    instance: Instance,
    descriptor: GuestDescriptor,
}

impl GuestPlugin {
    pub fn id(&self) -> &str {
        &self.id
    }

    pub fn descriptor(&self) -> &GuestDescriptor {
        &self.descriptor
    }

    /// 面板状态行（插件自己写的文本，按行号排列）。
    pub fn status_lines(&self) -> &[String] {
        &self.store.data().status_lines
    }

    /// Guest 经 `log_text` 写出的诊断行。
    pub fn log_lines(&self) -> &[String] {
        &self.store.data().log_lines
    }

    /// 从 WASM 字节码装载插件并读出描述。
    /// 描述要求的导出（`aweft_on_command`、订阅时的 `aweft_on_event_time`）
    /// 缺失时在这里就报错，运行期不再检查。
    pub fn load(id: &str, wasm_bytes: &[u8]) -> Result<Self> {
        let engine = Engine::default();
        let mut store = Store::new(&engine, HostCtx::default());
        let module = Module::new(&engine, wasm_bytes).context("插件 WASM 模块解析失败")?;
        let mut linker = Linker::<HostCtx>::new(&engine);

        linker
            .func_wrap(HOST_MODULE, "event_count", |caller: Caller<HostCtx>| {
                caller.data().rows.len() as i32
            })
            .context("宿主函数 event_count 注册失败")?;
        linker
            .func_wrap(
                HOST_MODULE,
                "event_start_ms",
                |caller: Caller<HostCtx>, handle: i32| -> i64 {
                    caller
                        .data()
                        .rows
                        .get(handle as usize)
                        .map(|row| row.start_ms)
                        .unwrap_or(-1)
                },
            )
            .context("宿主函数 event_start_ms 注册失败")?;
        linker
            .func_wrap(
                HOST_MODULE,
                "event_end_ms",
                |caller: Caller<HostCtx>, handle: i32| -> i64 {
                    caller
                        .data()
                        .rows
                        .get(handle as usize)
                        .map(|row| row.end_ms)
                        .unwrap_or(-1)
                },
            )
            .context("宿主函数 event_end_ms 注册失败")?;
        linker
            .func_wrap(
                HOST_MODULE,
                "set_event_start_ms",
                |mut caller: Caller<HostCtx>, handle: i32, ms: i64| -> i32 {
                    match caller.data_mut().rows.get_mut(handle as usize) {
                        Some(row) => {
                            row.start_ms = ms;
                            0
                        }
                        None => 1,
                    }
                },
            )
            .context("宿主函数 set_event_start_ms 注册失败")?;
        linker
            .func_wrap(
                HOST_MODULE,
                "set_event_end_ms",
                |mut caller: Caller<HostCtx>, handle: i32, ms: i64| -> i32 {
                    match caller.data_mut().rows.get_mut(handle as usize) {
                        Some(row) => {
                            row.end_ms = ms;
                            0
                        }
                        None => 1,
                    }
                },
            )
            .context("宿主函数 set_event_end_ms 注册失败")?;
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
                },
            )
            .context("宿主函数 set_status_line 注册失败")?;
        linker
            .func_wrap(HOST_MODULE, "log_text", |mut caller: Caller<HostCtx>, ptr: i32, len: i32| {
                match read_caller_str(&caller, ptr, len) {
                    Ok(text) => caller.data_mut().log_lines.push(text),
                    Err(e) => log::warn!("插件日志读取失败：{e:?}"),
                }
            })
            .context("宿主函数 log_text 注册失败")?;

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
            id: id.to_string(),
            store,
            instance,
            descriptor,
        };
        plugin.check_exports()?;
        Ok(plugin)
    }

    fn origin(&self) -> Origin {
        Origin::Plugin(self.id.clone())
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

    fn command_func(
        &self,
    ) -> Result<wasmi::TypedFunc<i32, i32>> {
        self.instance
            .get_typed_func::<i32, i32>(&self.store, "aweft_on_command")
            .context("插件声明了命令但缺少导出 aweft_on_command")
    }

    /// 调用命令。返回 Guest 的状态码（0 表示成功）。
    /// 调用前从存储重建快照，调用后把快照差异写回存储。
    pub fn invoke_command(&mut self, store: &mut ProjectStore, cmd_id: i32) -> Result<i32> {
        self.store.data_mut().sync_from_store(store);
        let code = self
            .command_func()?
            .call(&mut self.store, cmd_id)
            .with_context(|| format!("guest aweft_on_command({cmd_id}) 调用失败"))?;
        let origin = self.origin();
        self.store.data().flush_to_store(store, &origin)?;
        Ok(code)
    }

    /// 投递行时间变更。描述未订阅时直接返回，不碰存储。
    pub fn notify_time_changed(
        &mut self,
        store: &mut ProjectStore,
        handle: i32,
        start_ms: i64,
        end_ms: i64,
    ) -> Result<()> {
        if !self.descriptor.time_changed {
            return Ok(());
        }
        self.store.data_mut().sync_from_store(store);
        self.instance
            .get_typed_func::<(i32, i64, i64), ()>(&self.store, "aweft_on_event_time")
            .context("缺少导出 aweft_on_event_time")?
            .call(&mut self.store, (handle, start_ms, end_ms))
            .context("guest aweft_on_event_time 调用失败")?;
        let origin = self.origin();
        self.store.data().flush_to_store(store, &origin)
    }
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
