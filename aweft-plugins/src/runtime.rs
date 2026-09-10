use anyhow::{bail, Context, Result};
use aweft_model::ids::ItemId;
use aweft_project::ProjectStore;
use serde::Deserialize;
use wasmi::{Caller, Engine, Extern, Instance, Linker, Memory, Module, Store};

use crate::ctx::{HostCtx, RowSnap};

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

    /// Guest 经 `log_text` 写出的诊断行。
    pub fn log_lines(&self) -> Vec<String> {
        self.store.data().log_lines.clone()
    }

    /// 从 WASM 字节码装载插件并读出描述。
    /// 描述要求的导出（`aweft_on_command`、订阅时的 `aweft_on_event_time`）
    /// 缺失时在这里就报错，运行期不再检查。
    pub fn load(wasm_bytes: &[u8]) -> Result<Self> {
        let engine = Engine::default();
        let mut store = Store::new(&engine, HostCtx::default());
        let module = Module::new(&engine, wasm_bytes).context("插件 WASM 模块解析失败")?;
        let mut linker = Linker::<HostCtx>::new(&engine);

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
            .func_wrap(
                HOST_MODULE,
                "log_text",
                |mut caller: Caller<HostCtx>, ptr: i32, len: i32| match read_caller_str(
                    &caller, ptr, len,
                ) {
                    Ok(text) => caller.data_mut().log_lines.push(text),
                    Err(e) => log::warn!("插件日志读取失败：{e:?}"),
                },
            )
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
            store,
            instance,
            descriptor,
        };
        plugin.check_exports()?;
        Ok(plugin)
    }

    /// 从存储重建行快照。每次进入 Guest 前调一次。
    pub fn sync_from_store(&mut self, store: &ProjectStore) {
        self.store.data_mut().sync_from_store(store);
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
