//! `aweft collab ...`：协作身份与会话的命令行入口。
//!
//! 首版没有协作面板，这七个动词就是「网外手工通道」的落地处：
//! 谁都能先报出自己的成员标识，所有者据此签出邀请串，串经聊天/二维码传给对方，
//! 对方接受后即把该会话设为活跃——下次启动 GUI 就会带着它上线。
//!
//! 面板做出来之后本文件应当删掉：这些动词的每一步都只是
//! `aweft_collab::config` 的几个函数，界面直接调同一层即可。

use std::io::{self, Write};

use aweft_collab::config::{self, Scope};

pub fn run(mut args: impl Iterator<Item = String>) -> io::Result<()> {
    let Some(verb) = args.next() else {
        return usage();
    };
    let rest: Vec<String> = args.collect();
    let result = match verb.as_str() {
        "who" => who(),
        "create" => create(&rest),
        "invite" => invite(&rest),
        "accept" => accept(&rest),
        "list" => list(),
        "join" => join(&rest),
        "leave" => leave(),
        "help" | "--help" | "-h" => return usage(),
        other => {
            eprintln!("未知的协作子命令：{other}");
            return usage();
        }
    };
    match result {
        Ok(()) => Ok(()),
        Err(error) => {
            // `{:?}` 打的是 anyhow 的整条 cause 链，命令行上看得见失败原因。
            eprintln!("协作命令失败：{error:?}");
            Err(io::Error::other(error.to_string()))
        }
    }
}

fn usage() -> io::Result<()> {
    let text = "\
用法：aweft collab <子命令>

  who                    打印本机成员标识（要别人邀请你时，把这一串给对方）
  create <工程名>         以本机为所有者新建协作会话，并设为活跃
  invite [<会话>] <标识>  为成员标识 <标识> 签发出邀请串（整串发给对方）
                          <会话> 省略时用活跃会话；也可给工程名或标识前缀
  accept <邀请串>         接受邀请，落盘为会话记录并设为活跃
  list                   列出本机已加入的协作会话
  join <会话>             把某个会话设为活跃（工程名或标识前缀）
  leave                  取消活跃会话

活跃会话决定 GUI 启动时加入哪一场协作；没有活跃会话时协作不启用，主程序照常工作。
";
    let mut out = io::stdout();
    out.write_all(text.as_bytes())?;
    out.flush()
}

fn who() -> anyhow::Result<()> {
    let member = config::load_or_create_member()?;
    println!("{}", member.hex_id());
    Ok(())
}

fn create(args: &[String]) -> anyhow::Result<()> {
    let name = args
        .first()
        .filter(|name| !name.is_empty())
        .ok_or_else(|| anyhow::anyhow!("create 需要工程名：aweft collab create <工程名>"))?;
    let member = config::load_or_create_member()?;
    let record = config::create_project(name, &member)?;
    config::set_active(&record.namespace)?;
    println!("已创建协作工程「{name}」并设为活跃");
    println!("  命名空间 {}", record.namespace);
    println!("  本机身份 {}", member.hex_id());
    println!("下一步：队友先跑 `aweft collab who` 把它的标识发过来，再 `aweft collab invite <那个标识>`。");
    Ok(())
}

fn invite(args: &[String]) -> anyhow::Result<()> {
    let (mut session, to) = match (args.first(), args.get(1)) {
        (Some(to), None) => (active_record()?, to.clone()),
        (Some(name), Some(to)) => (find_record(name)?, to.clone()),
        _ => anyhow::bail!("用法：aweft collab invite [<会话>] <对方成员标识>"),
    };
    let member = config::load_or_create_member()?;
    // 本版按全工程范围转授：按轨道切分需要能填轨道标识的界面，留给面板。
    let text = config::invite(&mut session, &member, &to, Scope::All)?;
    println!("{text}");
    Ok(())
}

fn accept(args: &[String]) -> anyhow::Result<()> {
    let text = args
        .first()
        .ok_or_else(|| anyhow::anyhow!("用法：aweft collab accept <邀请串>"))?;
    let member = config::load_or_create_member()?;
    let record = config::accept(text.trim(), &member)?;
    config::set_active(&record.namespace)?;
    println!("已加入协作工程「{}」并设为活跃", record.name);
    Ok(())
}

fn list() -> anyhow::Result<()> {
    let active = config::active_namespace();
    let records = config::list_records();
    if records.is_empty() {
        println!("本机还没有加入任何协作工程");
        return Ok(());
    }
    for record in records {
        let mark = if active.as_deref() == Some(record.namespace.as_str()) {
            "*"
        } else {
            " "
        };
        let role = if record.namespace_secret.is_some() {
            "所有者"
        } else {
            "成员"
        };
        println!("{mark} {}  [{role}]  {}", record.name, record.namespace);
    }
    Ok(())
}

fn join(args: &[String]) -> anyhow::Result<()> {
    let needle = args
        .first()
        .ok_or_else(|| anyhow::anyhow!("用法：aweft collab join <工程名或标识前缀>"))?;
    let record = find_record(needle)?;
    config::set_active(&record.namespace)?;
    println!("活跃会话已切到「{}」", record.name);
    Ok(())
}

fn leave() -> anyhow::Result<()> {
    config::clear_active()?;
    println!("已取消活跃协作会话");
    Ok(())
}

fn active_record() -> anyhow::Result<config::ProjectRecord> {
    let namespace = config::active_namespace()
        .ok_or_else(|| anyhow::anyhow!("没有活跃会话：先 `aweft collab create` 或 `join`"))?;
    config::load_record(&namespace)
}

/// 按工程名或命名空间标识前缀找会话；歧义时报错，不猜。
fn find_record(needle: &str) -> anyhow::Result<config::ProjectRecord> {
    let matches: Vec<config::ProjectRecord> = config::list_records()
        .into_iter()
        .filter(|record| record.name == needle || record.namespace.starts_with(needle))
        .collect();
    match matches.as_slice() {
        [only] => Ok(only.clone()),
        [] => anyhow::bail!("没有匹配的协作会话：{needle}（`aweft collab list` 看有哪些）"),
        many => {
            let names: Vec<&str> = many.iter().map(|record| record.name.as_str()).collect();
            anyhow::bail!("「{needle}」匹配到多个会话：{}，请写全", names.join("、"))
        }
    }
}
