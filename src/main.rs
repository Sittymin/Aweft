use std::env;
use std::io;
mod cli;
use self::cli::parse_cli_commands;

fn main() -> io::Result<()> {
    // 必须在读设置之前定下配置目录：设置层首次访问就缓存，之后提交新目录不再生效。
    match aweft_settings::platform_config_dir() {
        Some(dir) => aweft_settings::set_config_dir(dir),
        None => eprintln!("无法从环境变量确定配置目录，设置按出厂值"),
    }

    let args = env::args().skip(1).peekable();
    if args.len() == 0 {
        if let Err(e) = aweft_gui::create_window() {
            eprint!("Error starting GUI: {}", e);
        };
    } else {
        parse_cli_commands(args)?;
    }

    Ok(())
}
