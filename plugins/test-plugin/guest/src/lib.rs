//! 示例插件的 Guest：主程序在运行时用 wasmi 装载本 crate 编出的 `guest.wasm`。
//! 它同时是语言绑定的样例：插件作者只需要 `static COMMANDS` 与若干处理函数。

use aweft_plugin_sdk::{Api, Command};

/// 毫秒 → `HH:MM:SS.mmm`。显示文本由插件自己组装，宿主不做格式化与模板替换。
fn clock(ms: i64) -> String {
    let total_s = ms / 1000;
    format!(
        "{:02}:{:02}:{:02}.{:03}",
        total_s / 3600,
        (total_s % 3600) / 60,
        total_s % 60,
        ms % 1000
    )
}

/// 命令 0：找出最早的字幕行开始时间，写进面板第 0 行。
fn earliest(api: &Api, _id: i32) -> i32 {
    match (0..api.event_count()).map(|handle| api.start_ms(handle)).min() {
        Some(ms) => {
            api.set_status(0, &format!("最早：{}", clock(ms)));
            0
        }
        None => {
            api.set_status(0, "没有字幕行");
            1
        }
    }
}

/// 命令 1：全部字幕行后移 1 秒，写进面板第 1 行。
fn shift_all(api: &Api, _id: i32) -> i32 {
    let count = api.event_count();
    let mut moved = 0;
    for handle in 0..count {
        // 快照写入无顺序要求：落真相时宿主按移动方向自己选写入顺序，
        // 合法的成对目标在任一方向都不会误触发钳制。
        let end_ok = api.set_end_ms(handle, api.end_ms(handle) + 1000);
        let start_ok = api.set_start_ms(handle, api.start_ms(handle) + 1000);
        if end_ok && start_ok {
            moved += 1;
        }
    }
    api.set_status(1, &format!("已平移 {moved} 行 +1s"));
    0
}

/// 订阅行时间变更：宿主每提交一次时间写入就调到这里。
fn note_change(api: &Api, handle: i32, start_ms: i64, end_ms: i64) {
    api.set_status(
        2,
        &format!("最近通知：第 {handle} 行 {} ~ {}", clock(start_ms), clock(end_ms)),
    );
    api.log(&format!("第 {handle} 行时间已变更"));
}

/// 命令 2：倒计时 10 秒再把全部字幕行前移 10 秒，面板实时显示剩余时间。
/// 时间双原语的用法：`now_ms` 算剩余（deadline 写法，睡眠开销不累积漂移），
/// `sleep_ms` 分段等（每次至多 100ms，保证显示约 10Hz 刷新、超调有界）；
/// 占住的是 worker 线程，UI 线程照常响应。每次 `set_status` 都直发 Progress，
/// 倒计时能一格一格刷出来；期间面板按钮按 `command-busy` 置灰
/// （插件作者侧处理，宿主只给信号）。
fn delayed_shift(api: &Api, _id: i32) -> i32 {
    let deadline = api.now_ms() + 10_000;
    loop {
        let left = deadline - api.now_ms();
        if left <= 0 {
            break;
        }
        api.set_status(3, &format!("前移 10s 倒计时：{:.1}s", left as f64 / 1000.0));
        api.sleep_ms(left.min(100));
    }
    let count = api.event_count();
    let mut moved = 0;
    for handle in 0..count {
        // 快照写入无顺序要求（见命令 1）：落真相时宿主按移动方向自己选序。
        let start_ok = api.set_start_ms(handle, api.start_ms(handle) - 10_000);
        let end_ok = api.set_end_ms(handle, api.end_ms(handle) - 10_000);
        if start_ok && end_ok {
            moved += 1;
        }
    }
    api.set_status(3, &format!("已前移 {moved} 行 -10s"));
    0
}

static COMMANDS: &[Command] = &[
    Command::new("计算最早时间", earliest),
    Command::new("全部后移 1s", shift_all),
    Command::new("全部前移 10s（延迟 10s）", delayed_shift),
];

aweft_plugin_sdk::plugin! {
    commands: COMMANDS,
    on_time_changed: note_change,
}
