use std::{env, fs::File, io, io::Write, path::PathBuf};

use nom::{character::complete::digit1, combinator::map_res, IResult, Parser};

pub fn create_test_file(content: &str) -> io::Result<PathBuf> {
    // 获取系统临时目录
    let mut test_path = env::temp_dir();
    // 使用时间戳创建唯一文件名
    let timestamp = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap()
        .as_nanos();
    test_path.push(format!("test_subtitle_{}.ass", timestamp));

    let mut file = File::create(&test_path)?;
    file.write_all(content.as_bytes())?;
    file.flush()?;
    Ok(test_path)
}
