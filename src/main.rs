use std::env;
use std::io;
mod cli;
mod gui;
use self::cli::parse_cli_commands;
use self::gui::gui;

// 定义自己的错误类型，可以处理所有可能的错误
#[derive(Debug)]
enum AppError {
    Io(io::Error),
    Frame(eframe::Error),
}

// 实现从 io::Error 的转换
impl From<io::Error> for AppError {
    fn from(err: io::Error) -> Self {
        AppError::Io(err)
    }
}

// 实现从 eframe::Error 的转换
impl From<eframe::Error> for AppError {
    fn from(err: eframe::Error) -> Self {
        AppError::Frame(err)
    }
}

fn main() -> Result<(), AppError> {
    let args = env::args().skip(1).peekable();
    if args.len() == 0 {
        println!("args is zero");
        gui()?;
    } else {
        parse_cli_commands(args)?; // io::Error 会自动转换为 AppError
    }

    Ok(())
}
