//! 一份带版本戳的 JSON 文件的读、写与版本门。

use anyhow::{bail, Context, Result};
use serde::de::DeserializeOwned;
use serde::Serialize;
use std::ffi::OsString;
use std::fmt::Display;
use std::io::Write;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};

/// 一个自带格式版本的 JSON 数据：文件与网络字节共用这一份规矩。
/// 实现者给出当前版本、版本字段的读与盖写，以及把旧数据补写到当前版本这一步。
///
/// 政策只有一条，仓库里所有 JSON 文件共用：
///
/// - 加字段不 bump [`Self::FORMAT`]：结构体上挂容器级 `#[serde(default)]`，旧文件缺的
///   字段自动取默认值，读出来就是一份合法的当前版本数据。
/// - 只有「旧值必须改写才有意义」的改动才 bump，并在 [`Self::migrate`] 里按版本顺序补。
/// - 读到的版本比本程序新时拒绝改写，不猜也不改写：旧程序静默改写新文件会把新程序
///   写进去的信息抹平。
pub trait JsonFile: Sized {
    /// 本程序写出的文件用的格式版本。
    const FORMAT: u32;

    /// 文件里的版本字段。
    fn format(&self) -> u32;

    /// 读入后把版本盖成当前值，写盘前也必须已是当前值。
    fn set_format(&mut self, format: u32);

    /// 把 `from` 版补写到 [`Self::FORMAT`] 版。只在 `from < Self::FORMAT` 时被调用；
    /// 旧值不需要改写的文件类型留默认空实现。
    fn migrate(&mut self, _from: u32) {}
}

/// 读一份 JSON 文件。文件不存在、读不出、解析失败、版本比本程序新，一律报错。
/// 工程文件用它：打不开的工程不能假装是一份空工程。
pub fn read_json<T: JsonFile>(path: &Path) -> Result<T>
where
    T: DeserializeOwned,
{
    let text = std::fs::read_to_string(path)
        .with_context(|| format!("文件读取失败：{}", path.display()))?;
    decode_with_format_gate(&text, &path.display())
}

/// 读一份 JSON 文件，文件不存在时按 [`Default`] 处理——首次启动就是这个状态。
/// 实现方的 `Default` 必须把 `format` 填成当前版本，否则这份默认值一写回去就变成旧版文件。
/// 解析失败与版本过新仍然报错：坏文件不能被静默抹平。
pub fn read_json_or_default<T: JsonFile + Default>(path: &Path) -> Result<T>
where
    T: DeserializeOwned,
{
    match std::fs::read_to_string(path) {
        Ok(text) => decode_with_format_gate(&text, &path.display()),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(T::default()),
        Err(error) => Err(error).with_context(|| format!("文件读取失败：{}", path.display())),
    }
}

/// 从字节解析并过版本门，规则与 [`read_json`] 一致。用于只走内存或网络的 JSON 文本，
/// 例如协作快照：不落盘，但同样不能拿旧版结构或未来版结构当当前版用。
/// `source` 只出现在错误里，用来点名这份字节从哪来（快照作者、连接名）。
pub fn decode_json<T, S>(bytes: &[u8], source: &S) -> Result<T>
where
    T: JsonFile + DeserializeOwned,
    S: Display + ?Sized,
{
    let text = std::str::from_utf8(bytes).with_context(|| format!("不是 UTF-8 文本：{source}"))?;
    decode_with_format_gate(text, source)
}

/// 解析文本并过版本门。`source` 只用于错误里点名来源：文件路径，或内存里的字节是谁给的。
fn decode_with_format_gate<T, S>(text: &str, source: &S) -> Result<T>
where
    T: JsonFile + DeserializeOwned,
    S: Display + ?Sized,
{
    let mut value: T = serde_json::from_str(text).with_context(|| format!("解析失败：{source}"))?;
    apply_format_gate(&mut value, source)?;
    Ok(value)
}

/// 版本门：比本程序新就拒绝，比本程序旧就补写后盖成当前版本。
/// 调用方负责用 `Context` 补上「这是哪份数据」，这里只说清版本差。
fn apply_format_gate<T, S>(value: &mut T, source: &S) -> Result<()>
where
    T: JsonFile,
    S: Display + ?Sized,
{
    let from = value.format();
    if from > T::FORMAT {
        bail!(
            "版本 {from} 比本程序支持的 {} 新，拒绝改写：{source}",
            T::FORMAT
        );
    }
    if from < T::FORMAT {
        value.migrate(from);
        value.set_format(T::FORMAT);
    }
    Ok(())
}

/// 序列化成紧凑字节，与 [`decode_json`] 成对。用于只走内存或网络的 JSON 文本。
/// 写文件不用它——文件的缩进由 [`write_json`] 负责。
pub fn encode_json<T: Serialize + ?Sized>(value: &T) -> Result<Vec<u8>> {
    serde_json::to_vec(value).context("序列化失败")
}

/// 写一份 JSON 文件（美化文本，人手可改）。原子性见 [`write_text_atomic`]。
pub fn write_json<T: Serialize + ?Sized>(path: &Path, value: &T) -> Result<()> {
    let text = serde_json::to_string_pretty(value)
        .with_context(|| format!("序列化失败：{}", path.display()))?;
    write_text_atomic(path, &text)
}

/// 原子写一段文本：在目标文件的同目录建临时文件写完全文，刷盘，再 `rename` 盖到
/// 正式名字上。这样任何时刻读这个路径得到的都是完整的旧文件或完整的新文件。
/// 目标的父目录不存在时先创建。
pub fn write_text_atomic(path: &Path, text: &str) -> Result<()> {
    if let Some(parent) = path
        .parent()
        .filter(|parent| !parent.as_os_str().is_empty())
    {
        std::fs::create_dir_all(parent)
            .with_context(|| format!("目录创建失败：{}", parent.display()))?;
    }
    let tmp = temp_path(path);
    let result = (|| -> Result<()> {
        let mut file = std::fs::File::create(&tmp)
            .with_context(|| format!("临时文件创建失败：{}", tmp.display()))?;
        file.write_all(text.as_bytes())
            .with_context(|| format!("临时文件写入失败：{}", tmp.display()))?;
        // sync 必须在 rename 之前：没刷盘的临时文件被 rename 上去之后，进程被杀或掉电
        // 会留下一个 0 字节的正式文件，下次启动就读不到内容了。
        file.sync_all()
            .with_context(|| format!("临时文件刷盘失败：{}", tmp.display()))?;
        std::fs::rename(&tmp, path)
            .with_context(|| format!("文件替换失败：{} -> {}", tmp.display(), path.display()))
    })();
    if result.is_err() {
        // 失败路径上清掉半成品，别让目录里堆垃圾；清理本身失败不值得盖掉原始错误。
        let _ = std::fs::remove_file(&tmp);
    }
    result
}

static TEMP_SEQ: AtomicU64 = AtomicU64::new(0);

/// 临时文件名带进程号与进程内序号：命令行与界面可能同时在写同一份文件，同一进程内
/// 的多个线程也不能共用一个临时名（测试就是多线程跑的同一目录）。
fn temp_path(path: &Path) -> PathBuf {
    let mut name: OsString = path.file_name().map(OsString::from).unwrap_or_default();
    name.push(format!(
        ".tmp-{}-{}",
        std::process::id(),
        TEMP_SEQ.fetch_add(1, Ordering::Relaxed)
    ));
    path.with_file_name(name)
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde::Deserialize;

    /// 测试夹具：`count` 在 2 版才有意义，所以从 1 版补写时要加一。
    #[derive(Debug, Default, PartialEq, Serialize, Deserialize)]
    #[serde(default)]
    struct Note {
        format: u32,
        text: String,
        count: u32,
    }

    impl JsonFile for Note {
        const FORMAT: u32 = 2;

        fn format(&self) -> u32 {
            self.format
        }

        fn set_format(&mut self, format: u32) {
            self.format = format;
        }

        fn migrate(&mut self, from: u32) {
            if from < 2 {
                self.count += 1;
            }
        }
    }

    impl Note {
        fn sample() -> Note {
            Note {
                format: Note::FORMAT,
                text: "hello".into(),
                count: 3,
            }
        }
    }

    #[test]
    fn round_trip_reads_back_what_was_written() {
        let dir = tempfile::tempdir().expect("临时目录");
        let path = dir.path().join("note.json");
        let note = Note::sample();

        write_json(&path, &note).expect("写入");
        let loaded: Note = read_json(&path).expect("读回");

        assert_eq!(loaded, note);
    }

    #[test]
    fn missing_file_is_an_error_for_read_json() {
        let dir = tempfile::tempdir().expect("临时目录");
        let error = read_json::<Note>(&dir.path().join("absent.json")).expect_err("缺文件必须报错");
        assert!(
            error.to_string().contains("absent.json"),
            "错误里要点名路径：{error}"
        );
    }

    #[test]
    fn missing_file_falls_back_to_default() {
        let dir = tempfile::tempdir().expect("临时目录");
        let loaded: Note =
            read_json_or_default(&dir.path().join("absent.json")).expect("缺文件不是错误");
        assert_eq!(loaded, Note::default());
    }

    #[test]
    fn newer_format_is_rejected() {
        let dir = tempfile::tempdir().expect("临时目录");
        let path = dir.path().join("note.json");
        std::fs::write(&path, r#"{"format": 999, "text": "x", "count": 0}"#).expect("写入测试文件");

        let error = read_json::<Note>(&path).expect_err("更新的版本必须被拒");
        assert!(
            error.to_string().contains("999"),
            "错误里要点名版本：{error}"
        );
    }

    #[test]
    fn bytes_round_trip_through_the_gate() {
        let note = Note::sample();
        let bytes = encode_json(&note).expect("序列化");
        let loaded: Note = decode_json(&bytes, "快照测试").expect("反序列化");
        assert_eq!(loaded, note);
    }

    #[test]
    fn newer_format_in_bytes_is_rejected_by_name() {
        let bytes = r#"{"format": 999}"#.as_bytes();
        let error = decode_json::<Note, _>(bytes, "member-42").expect_err("新版必须被拒");
        let message = error.to_string();
        assert!(message.contains("999"), "错误里要点名版本：{message}");
        assert!(message.contains("member-42"), "错误里要点名来源：{message}");
    }

    #[test]
    fn older_format_is_migrated_and_stamped() {
        let dir = tempfile::tempdir().expect("临时目录");
        let path = dir.path().join("note.json");
        std::fs::write(&path, r#"{"format": 1, "text": "x", "count": 0}"#).expect("写入测试文件");

        let loaded: Note = read_json(&path).expect("旧版本要能读");
        assert_eq!(
            loaded,
            Note {
                format: Note::FORMAT,
                text: "x".into(),
                count: 1
            }
        );
    }

    #[test]
    fn absent_fields_take_defaults_and_get_stamped() {
        let dir = tempfile::tempdir().expect("临时目录");
        let path = dir.path().join("note.json");
        std::fs::write(&path, "{}").expect("写入测试文件");

        let loaded: Note = read_json(&path).expect("缺字段要能读");
        assert_eq!(loaded.format, Note::FORMAT, "缺 format 的旧文件按 0 补写");
        assert_eq!(loaded.count, 1, "补写这一步要对缺字段的文件生效");
    }

    #[test]
    fn write_leaves_no_temp_file() {
        let dir = tempfile::tempdir().expect("临时目录");
        let path = dir.path().join("note.json");
        write_json(&path, &Note::sample()).expect("写入");

        let names: Vec<String> = std::fs::read_dir(dir.path())
            .expect("列目录")
            .filter_map(|entry| entry.ok())
            .map(|entry| entry.file_name().to_string_lossy().into_owned())
            .collect();
        assert_eq!(names, vec!["note.json".to_string()]);
    }

    #[test]
    fn write_creates_missing_parent_dirs() {
        let dir = tempfile::tempdir().expect("临时目录");
        let path = dir.path().join("nested/deeper").join("note.json");
        write_json(&path, &Note::sample()).expect("写入");
        assert!(path.is_file(), "父目录应当被一并创建");
    }
}
