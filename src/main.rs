use saffron_ass::merge_ass_files;
use saffron_ass::save_ass;
use std::env;
use std::io::{self, Error, ErrorKind};
use std::path::PathBuf;

fn main() -> io::Result<()> {
    let mut args = env::args().skip(1).peekable();

    let mut files_with_offsets = Vec::new();
    let mut output_path = None;

    // TODO: 重构此解析
    while let Some(arg) = args.next() {
        println!("参数是: {}", arg);
        if arg == "-m" || arg == "--merge" {
            match parse_file_with_offset(&mut args) {
                Ok(fwo) => files_with_offsets.push(fwo),
                Err(e) => {
                    eprintln!("错误：{}", e);
                    return Err(e);
                }
            }
        } else if arg == "-o" || arg == "--output" {
            if output_path.is_some() {
                eprintln!("警告：已有输出路径，忽略{:#?}", output_path);
                continue;
            } else if let Some(output_arg) = args.next() {
                output_path = Some(PathBuf::from(output_arg));
                continue;
            }

            eprintln!("错误：输出路径解析出错");
        }
    }

    if !files_with_offsets.is_empty() && output_path.is_some() {
        match merge_ass_files(
            files_with_offsets
                .iter()
                .map(|&(ref path, h, m, s, cs)| (path.as_path(), h, m, s, cs))
                .collect(),
        ) {
            Ok(ass_file) => {
                println!("文件合并成功: {:#?}", output_path);
                save_ass(&ass_file, output_path.as_ref().unwrap())?;
            }
            Err(e) => eprintln!("合并文件时出错: {}", e),
        }
    } else {
        eprintln!("错误：'-m' 或 '--merge' 需要跟随至少一个文件路径、时间偏移量和一个输出路径");
        return Err(Error::new(
            ErrorKind::InvalidInput,
            "缺少 '-m' 或 '--merge' 的参数",
        ));
    }

    Ok(())
}

fn parse_file_with_offset(
    args_iter: &mut impl Iterator<Item = String>,
) -> Result<(PathBuf, i32, i32, i32, i32), io::Error> {
    // 获取文件路径
    let file_str = args_iter
        .next()
        .ok_or_else(|| io::Error::new(io::ErrorKind::InvalidInput, "缺少文件路径"))?;
    let file_path = PathBuf::from(&file_str);
    if !file_path.exists() || !file_path.is_file() {
        return Err(io::Error::new(
            io::ErrorKind::NotFound,
            format!("文件 '{}' 不存在或不是一个文件", file_str),
        ));
    }

    // 定义一个辅助函数来简化错误处理
    fn parse_i32(s: Option<String>, field: &str) -> Result<i32, io::Error> {
        s.ok_or_else(|| io::Error::new(io::ErrorKind::InvalidInput, format!("缺少 {}", field)))
            .and_then(|s| {
                s.parse::<i32>().map_err(|_| {
                    io::Error::new(
                        io::ErrorKind::InvalidInput,
                        format!("无效的 {}: '{}'", field, s),
                    )
                })
            })
    }

    // 获取时间偏移量
    let offset_hours = parse_i32(args_iter.next(), "小时数")?;
    let offset_minutes = parse_i32(args_iter.next(), "分钟数")?;
    let offset_seconds = parse_i32(args_iter.next(), "秒数")?;
    let offset_centiseconds = parse_i32(args_iter.next(), "百分秒数")?;

    Ok((
        file_path,
        offset_hours,
        offset_minutes,
        offset_seconds,
        offset_centiseconds,
    ))
}
