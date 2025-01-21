use std::env;
use std::io;
mod cli;
use self::cli::parse_cli_commands;

fn main() -> io::Result<()> {
    let args = env::args().skip(1).peekable();
    if args.len() == 0 {
        println!("args is zero")
    } else {
        parse_cli_commands(args)?;
    }

    Ok(())
}
