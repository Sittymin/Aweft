//! 设计验证 5：协作身份、会话记录与邀请文本的落盘往返。
//!
//! 对应设计草稿「文件改动清单」第 5 条与「权限」节的邀请两段式。
//! 断言：本机身份第二次读还是同一个；所有者建工程 → 签邀请 → 对方接受 → 拼出引擎会话
//! 这条链闭合；成员可以向下转授自己持有的范围但不能越界；收紧写回记录后旧轨道连签发都
//! 签不出来；三处误用（邀请不是写给自己的、越界转授、命名空间字段是路径形状）都被挡下。
//!
//! 只用一个 `#[test]`：配置目录由进程内一次性锁决定，多个测试并行会互相抢。

use std::path::PathBuf;

use aweft_collab::cap::Member;
use aweft_collab::config::{
    accept, collab_dir, create_project, invite, list_records, load_or_create_member, load_record,
    save_capability, session, Invite, Scope,
};
use aweft_model::ids::ItemId;
use uuid::Uuid;

fn scratch_config_dir() -> PathBuf {
    // 测试产物落在仓库内的 target/scratch：这台机器上 /tmp 是 tmpfs，且随会话回收。
    let root = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .expect("crate 目录的上一层是仓库根")
        .join("target/scratch");
    let dir = root.join(format!("collab-config-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).expect("临时配置目录应当能创建");
    dir
}

/// 把一份合法邀请的命名空间字段换成路径形状，其余原样保留。
fn tamper_namespace(invite: &str) -> String {
    let mut value: serde_json::Value =
        serde_json::from_slice(&hex::decode(invite).expect("邀请串应当是合法的十六进制"))
            .expect("邀请串应当可解析");
    value["namespace"] = serde_json::Value::String("../evil".into());
    hex::encode(serde_json::to_vec(&value).expect("邀请应当可序列化"))
}

#[test]
fn identity_invitation_and_session_round_trip() {
    let dir = scratch_config_dir();
    // 环境变量优先于代码提交的目录（那是给用户留的配置入口），测试里要把它让开：
    // 否则临时目录不生效，这份记录会写进开发者真实的配置目录。
    std::env::remove_var(aweft_storage::USER_CONFIG_ENV);
    aweft_storage::set_config_dir(dir.clone());

    let owner = load_or_create_member().expect("首次读取应当生成并落盘本机身份");
    let again = load_or_create_member().expect("身份文件应当可复读");
    assert_eq!(owner.hex_id(), again.hex_id(), "第二次读必须用同一份密钥");

    let mut record = create_project("测试工程", &owner).expect("所有者建工程应当成功");
    assert_eq!(record.name, "测试工程");
    assert!(
        record.namespace_secret.is_some(),
        "所有者必须带着命名空间密钥，否则不能再签发创世凭证"
    );
    assert_eq!(
        load_record(&record.namespace).expect("记录应当可读回"),
        record
    );
    assert!(
        list_records()
            .iter()
            .any(|found| found.namespace == record.namespace),
        "工程应当出现在已加入列表里"
    );

    // 邀请：把某条轨道的写权限转授给对方。
    let invitee = Member::generate();
    let track = ItemId(Uuid::from_bytes([7u8; 16]));
    let text = invite(&mut record, &owner, &invitee.hex_id(), Scope::Track(track))
        .expect("所有者签邀请应当成功");
    assert_eq!(
        load_record(&record.namespace)
            .expect("记录应当可读回")
            .grants,
        record.grants,
        "签出去的凭证必须落盘：通告靠引擎发，而这一步可能没有引擎在场"
    );
    assert_eq!(
        list_records()
            .iter()
            .find(|found| found.namespace == record.namespace)
            .expect("工程应当出现在已加入列表里")
            .name,
        "测试工程"
    );
    let mut accepted = accept(&text, &invitee).expect("对方接受自己的邀请应当成功");
    assert_eq!(accepted.namespace, record.namespace);
    assert_eq!(accepted.name, "测试工程");
    assert!(
        accepted.namespace_secret.is_none(),
        "被邀请者拿不到命名空间密钥"
    );
    assert!(
        !accepted.capability.is_empty(),
        "接受邀请后本机应当已经持有一份凭证"
    );

    // 成员侧往下转授：相等范围可以，越界的不行——判据是密码学的，不看谁是所有者。
    let third = Member::generate();
    assert!(
        invite(&mut accepted, &invitee, &third.hex_id(), Scope::All).is_err(),
        "只持有单轨道凭证的成员不能给出全工程范围"
    );
    let other = ItemId(Uuid::from_bytes([8u8; 16]));
    assert!(
        invite(
            &mut accepted,
            &invitee,
            &third.hex_id(),
            Scope::Track(other)
        )
        .is_err(),
        "别的轨道不在这张凭证的范围内，同样不能转授"
    );
    assert!(
        invite(
            &mut accepted,
            &invitee,
            &third.hex_id(),
            Scope::Track(track)
        )
        .is_ok(),
        "转授自己持有的那条轨道应当成功"
    );
    assert_eq!(accepted.grants.len(), 1, "失败的转授不该留下通告记录");

    // 收紧：所有者凭自己那张重签一张窄的，成员侧写回记录后旧轨道就连签发都签不出了。
    let narrowed = ItemId(Uuid::from_bytes([9u8; 16]));
    let tightened = invite(
        &mut record,
        &owner,
        &invitee.hex_id(),
        Scope::Track(narrowed),
    )
    .expect("所有者收紧应当成功");
    assert_eq!(record.grants.len(), 2, "两次签发都要留补发通告的记录");
    let tightened: Invite =
        serde_json::from_slice(&hex::decode(&tightened).expect("邀请串应当是合法的十六进制"))
            .expect("邀请串应当可解析");
    save_capability(&mut accepted, &tightened.capability).expect("收紧后的凭证应当写得进记录");
    assert_eq!(
        load_record(&accepted.namespace)
            .expect("记录应当可读回")
            .capability,
        accepted.capability,
        "本机凭证换了必须落盘，否则重启后又拿旧的上岗"
    );
    assert!(
        save_capability(&mut accepted, "不再是十六进制").is_err(),
        "解不开的凭证不该写进记录"
    );
    assert!(
        invite(
            &mut accepted,
            &invitee,
            &third.hex_id(),
            Scope::Track(track)
        )
        .is_err(),
        "旧轨道已经不在他手里，再转授就签不出来"
    );

    // 引擎会话：凭证解得开、接收者是本机、日志库落在协作目录内、签发记录跟着上线。
    let engine_session = session(&accepted, &invitee).expect("会话应当能拼出来");
    assert_eq!(
        hex::encode(&engine_session.capability),
        tightened.capability,
        "会话用的应当是收紧后那张"
    );
    assert_eq!(
        engine_session.announcements.len(),
        1,
        "本机签出的凭证应当补发通告"
    );
    let expected_namespace: [u8; 32] = hex::decode(&record.namespace)
        .expect("命名空间标识是十六进制")
        .as_slice()
        .try_into()
        .expect("命名空间标识 32 字节");
    assert_eq!(engine_session.namespace.id_bytes(), expected_namespace);
    let database = engine_session.database.expect("落盘会话必然带日志库路径");
    assert!(database.starts_with(collab_dir().expect("协作目录")));
    assert_eq!(
        database.file_name().and_then(|name| name.to_str()),
        Some(format!("{}.sqlite", record.namespace).as_str())
    );
    assert!(engine_session.bootstrap.is_none(), "没配中继就不该有引导");

    // 误用一：邀请写给别人的，本机不接受。
    let stranger = Member::generate();
    assert!(
        accept(&text, &stranger).is_err(),
        "不是写给本机的邀请必须拒收"
    );

    // 误用二：命名空间字段来自网外，不能变成跳出会话目录的路径。
    assert!(
        accept(&tamper_namespace(&text), &invitee).is_err(),
        "非法命名空间形状必须被拒"
    );
    assert!(
        !dir.join("collab").join("evil.json").exists(),
        "非法命名空间不该在协作目录里留下文件"
    );
}
