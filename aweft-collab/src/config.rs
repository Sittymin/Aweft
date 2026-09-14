//! 协作身份的落盘：本机成员密钥、每工程的会话记录、以及两者之间的邀请文本。
//!
//! 全部放在配置目录下的 `collab/`：
//!
//! ```text
//! collab/member.json                本机成员种子（签名密钥即子空间密钥，见 `cap`）
//! collab/projects/<命名空间>.json    一个工程一份：凭证 +（所有者才有）命名空间密钥
//! collab/db/<命名空间>.sqlite       该工程协作节点的日志库
//! collab/active                    启动时加入哪一份会话（一行命名空间标识）
//! ```
//!
//! 字段一律以十六进制文本存 JSON，与工程文件同一种可读格式：密钥文件要能被人
//! 备份、复制、手写，二进制只会添堵。写盘一律「临时文件 + 刷盘 + rename」，
//! 半途而废的密钥文件比没有密钥文件更糟。
//!
//! 邀请分两段：`Meadowcap` 的凭证必须写明接收者，所以所有者先拿到对方的成员标识
//! 才能 [`invite`]；被邀请者 [`accept`] 时核对串里的接收者是不是自己。

use anyhow::{Context, Result, bail};
use aweft_model::ids::ItemId;
use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};
use willow25::groupings::Area;

use crate::cap::{
    Member, Namespace, SEED_LEN, decode_capability, encode_capability, member_id_of_bytes,
    track_area,
};
use crate::engine::{Bootstrap, Session};

/// 文件格式版本：只用来拒绝比本程序更新的文件，不做静默兼容。
const FORMAT: u8 = 1;

/// 转授范围。
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Scope {
    /// 整个工程：所有轨道的所有行。
    All,
    /// 单条轨道：路径前缀 `/tracks/<轨道组标识>`。
    Track(ItemId),
}

#[derive(Serialize, Deserialize)]
struct MemberFile {
    format: u8,
    seed: String,
}

/// 一个工程的协作会话记录。落盘形状与内存形状一致，字段即 JSON 字段。
#[derive(Serialize, Deserialize, Debug, Clone, PartialEq, Eq)]
pub struct ProjectRecord {
    pub format: u8,
    /// 展示名，只在本地生效，不进网络。
    pub name: String,
    /// 命名空间标识：同时是主题，也是本记录的键。
    pub namespace: String,
    /// 本机在该工程持有的写凭证。
    pub capability: String,
    /// 命名空间密钥：只有所有者持有，凭它才能继续向新人转授。
    #[serde(default)]
    pub namespace_secret: Option<String>,
    /// 引导中继地址与引导节点标识：两端直连不上时才需要。
    #[serde(default)]
    pub relay_url: Option<String>,
    #[serde(default)]
    pub bootstrap_node: Option<String>,
    /// 本机签发出去的凭证：启动时逐条重新通告，让只经命令行完成的转授也进本机日志。
    #[serde(default)]
    pub grants: Vec<String>,
}

#[derive(Serialize, Deserialize, Debug, Clone, PartialEq, Eq)]
pub struct Invite {
    pub format: u8,
    pub name: String,
    pub namespace: String,
    pub capability: String,
    /// 这份邀请写给谁。只有提示作用，真正的强制力在凭证的接收者字段里。
    pub to: String,
}

/// `collab/` 目录，不存在时创建。
pub fn collab_dir() -> Result<PathBuf> {
    let dir = aweft_settings::config_dir()
        .context("没有可用的配置目录，协作身份无法落盘")?
        .join("collab");
    std::fs::create_dir_all(&dir).with_context(|| format!("协作目录创建失败：{}", dir.display()))?;
    restrict_dir(&dir);
    Ok(dir)
}

/// 读本机成员身份：第一次用时生成一份并落盘。
pub fn load_or_create_member() -> Result<Member> {
    let path = collab_dir()?.join("member.json");
    if path.is_file() {
        let text = std::fs::read_to_string(&path)
            .with_context(|| format!("成员密钥读取失败：{}", path.display()))?;
        let file: MemberFile =
            serde_json::from_str(&text).with_context(|| format!("成员密钥无效：{}", path.display()))?;
        check_format(file.format, &path)?;
        return Ok(Member::from_seed(&seed_of(&file.seed)?));
    }
    let member = Member::generate();
    let file = MemberFile {
        format: FORMAT,
        seed: hex::encode(member.seed()),
    };
    write_json(&path, &file)?;
    // 密钥文件只给本机用户读：默认 umask 下的 0644 在共享机器上等于泄露。
    restrict(&path);
    log::info!("已生成本机协作身份 {}", member.hex_id());
    Ok(member)
}

fn records_dir() -> Result<PathBuf> {
    let dir = collab_dir()?.join("projects");
    std::fs::create_dir_all(&dir).with_context(|| format!("会话目录创建失败：{}", dir.display()))?;
    Ok(dir)
}

fn record_path(namespace: &str) -> Result<PathBuf> {
    Ok(records_dir()?.join(format!("{}.json", namespace_key(namespace)?)))
}

/// 命名空间标识 → 规范化的文件名主干。
///
/// 邀请串里的这段文本来自网外通道，不能直接当路径用：先验长度再重编码，
/// `../` 之类的形状在这一关就进不来。
fn namespace_key(namespace: &str) -> Result<String> {
    Ok(hex::encode(seed_of(namespace)?))
}

/// 某工程的会话记录。不存在时报错，由调用方决定这是「未加入」还是异常。
pub fn load_record(namespace: &str) -> Result<ProjectRecord> {
    let path = record_path(namespace)?;
    let text = std::fs::read_to_string(&path)
        .with_context(|| format!("会话记录读取失败：{}", path.display()))?;
    let record: ProjectRecord = serde_json::from_str(&text)
        .with_context(|| format!("会话记录无效：{}", path.display()))?;
    check_format(record.format, &path)?;
    Ok(record)
}

/// 所有已加入的工程，按展示名排序。读不懂的记录只记日志跳过，不挡住启动。
pub fn list_records() -> Vec<ProjectRecord> {
    let mut records = Vec::new();
    let Ok(dir) = records_dir() else {
        // 没有配置目录（未初始化的命令行环境）按「没有工程」处理。
        return records;
    };
    let Ok(entries) = std::fs::read_dir(&dir) else {
        log::warn!("会话目录读取失败：{}", dir.display());
        return records;
    };
    for entry in entries.flatten() {
        let path = entry.path();
        if path.extension().and_then(|ext| ext.to_str()) != Some("json") {
            continue;
        }
        let Ok(text) = std::fs::read_to_string(&path) else {
            continue;
        };
        match serde_json::from_str::<ProjectRecord>(&text) {
            Ok(record) if record.format == FORMAT => records.push(record),
            Ok(_) => log::warn!("会话记录版本过新，跳过：{}", path.display()),
            Err(error) => log::warn!("会话记录无效，跳过（{}）：{error:?}", path.display()),
        }
    }
    records.sort_by(|a, b| a.name.cmp(&b.name));
    records
}

pub fn save_record(record: &ProjectRecord) -> Result<()> {
    let path = record_path(&record.namespace)?;
    write_json(&path, record)?;
    restrict(&path);
    Ok(())
}

/// 启动时自动加入的会话：`collab/active` 里一行命名空间标识。
/// 本版还没有选工程的界面，这个文件就是那扇门：手写、`set_active`，以后交给面板。
fn active_path() -> Result<PathBuf> {
    Ok(collab_dir()?.join("active"))
}

pub fn set_active(namespace: &str) -> Result<()> {
    // 先过 `namespace_key`：存进去的只能是规范化标识，不能是任意文本。
    let key = namespace_key(namespace)?;
    let path = active_path()?;
    write_atomic(&path, &format!("{key}\n"))?;
    restrict(&path);
    Ok(())
}

pub fn clear_active() -> Result<()> {
    let path = active_path()?;
    match std::fs::remove_file(&path) {
        Ok(()) => Ok(()),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(()),
        Err(error) => Err(error).with_context(|| format!("活跃会话清除失败：{}", path.display())),
    }
}

/// 活跃会话的命名空间标识。没有该文件（或内容为空）时 `None`，这是首启的正常状态。
pub fn active_namespace() -> Option<String> {
    let text = std::fs::read_to_string(active_path().ok()?).ok()?;
    let text = text.trim();
    (!text.is_empty()).then(|| text.to_string())
}

/// 所有者新建一个工程的协作会话：滚出自有命名空间，给自己签全区域凭证，落盘。
pub fn create_project(name: &str, member: &Member) -> Result<ProjectRecord> {
    let namespace = Namespace::generate()?;
    let record = ProjectRecord {
        format: FORMAT,
        name: name.to_string(),
        namespace: hex::encode(namespace.id_bytes()),
        capability: hex::encode(encode_capability(&namespace.owner_capability(member)?)),
        namespace_secret: namespace.secret_bytes().map(hex::encode),
        relay_url: None,
        bootstrap_node: None,
        grants: Vec::new(),
    };
    save_record(&record)?;
    Ok(record)
}

/// 记录里的命名空间：所有者带密钥（可再转授），被邀请者只有标识。
pub fn namespace_of(record: &ProjectRecord) -> Result<Namespace> {
    match &record.namespace_secret {
        Some(secret) => Ok(Namespace::from_seed(&seed_of(secret)?)),
        None => Ok(Namespace::from_id_bytes(&seed_of(&record.namespace)?)),
    }
}

/// 签发邀请：把 `scope` 范围的写权限转授给成员标识为 `to` 的对方。
///
/// 根是本机自己那张凭证（所以任何成员都能往下转授），签名用本机子空间密钥。
/// 「不能给出自己没有的权限」不由这里判断：`willow25` 的 `try_delegate` 要求
/// 新区域 ⊆ 当前凭证区域且签名者 == 当前凭证接收者，越界在签发时就失败，
/// 接收端解码时还会逐段重验（见 `cap::decode_capability`）。
/// 所有者能转授全工程也只是因为他的创世凭证就是全区域。
///
/// 签出去的凭证记进 `record.grants` 并落盘：通告要靠引擎发出去，而这一步可能
/// 发生在没有引擎的命令行里，不留下来启动时就无从补发。
pub fn invite(
    record: &mut ProjectRecord,
    member: &Member,
    to: &str,
    scope: Scope,
) -> Result<String> {
    let namespace = namespace_of(record)?;
    let own = decode_capability(&hex_of(&record.capability)?)?;
    if own.receiver() != member.id() {
        bail!("会话记录里的凭证不属于本机身份，请重新接受邀请");
    }
    let area = match scope {
        Scope::All => Area::full(),
        Scope::Track(track) => track_area(track),
    };
    let receiver = member_id_of_bytes(&seed_of(to)?)?;
    let cap = namespace.delegate(&own, member, area, &receiver)?;
    let encoded = hex::encode(encode_capability(&cap));
    let invite = Invite {
        format: FORMAT,
        name: record.name.clone(),
        namespace: record.namespace.clone(),
        capability: encoded.clone(),
        to: hex::encode(receiver.to_bytes()),
    };
    if !record.grants.iter().any(|granted| granted == &encoded) {
        record.grants.push(encoded);
        save_record(record)?;
    }
    Ok(hex::encode(
        serde_json::to_vec(&invite).context("邀请序列化失败")?,
    ))
}

/// 被邀请者接受邀请：核对接收者后落盘成会话记录。
/// 凭证本身不在这验（[`session`] 与接收端验算都会验），这里只挡明显的误用。
pub fn accept(invite: &str, member: &Member) -> Result<ProjectRecord> {
    let invite: Invite =
        serde_json::from_slice(&hex_of(invite)?).context("邀请串无法解析，确认整串复制完整")?;
    if invite.format != FORMAT {
        bail!("邀请版本过新，请升级后再试");
    }
    if invite.to != member.hex_id() {
        bail!(
            "这份邀请不是写给本机的（写给 {}，本机 {}）",
            invite.to,
            member.hex_id()
        );
    }
    let record = ProjectRecord {
        format: FORMAT,
        name: invite.name,
        namespace: invite.namespace,
        capability: invite.capability,
        namespace_secret: None,
        relay_url: None,
        bootstrap_node: None,
        grants: Vec::new(),
    };
    save_record(&record)?;
    Ok(record)
}

/// 会话记录 + 本机身份 → 引擎启动参数。日志库落在 `collab/db/<命名空间>.sqlite`。
pub fn session(record: &ProjectRecord, member: &Member) -> Result<Session> {
    let namespace = namespace_of(record)?;
    let capability = decode_capability(&hex_of(&record.capability)?)?;
    if capability.receiver() != member.id() {
        bail!("会话记录里的凭证不属于本机身份，请重新接受邀请");
    }
    let bootstrap = match (&record.relay_url, &record.bootstrap_node) {
        (Some(relay_url), Some(node_id)) => Some(Bootstrap {
            node_id: p2panda::NodeId::from_bytes(&seed_of(node_id)?)
                .map_err(|error| anyhow::anyhow!("引导节点标识不合法：{error:?}"))?,
            relay_url: relay_url.clone(),
        }),
        _ => None,
    };
    let mut announcements = Vec::with_capacity(record.grants.len());
    for granted in &record.grants {
        match hex_of(granted).and_then(|bytes| decode_capability(&bytes).map(|_| bytes)) {
            // 补发只影响别人那侧的名册，坏条目拦不住本机上线：记一条日志就够。
            Ok(bytes) => announcements.push(bytes),
            Err(error) => log::warn!("本机签发记录无效，不通告：{error:?}"),
        }
    }
    Ok(Session {
        member: Member::from_seed(member.seed()),
        namespace,
        capability: encode_capability(&capability),
        database: Some(database_path(&record.namespace)?),
        bootstrap,
        announcements,
    })
}

/// 本机凭证被换掉（收紧或撤销）后写回会话记录。
///
/// 必须写：名册里本机那一条在引擎收到通告时就已更新，记录里还留着旧的一张的话，
/// 重启后拿旧的上岗，第一笔写入就会被自己的名册判成过期凭证。
/// 只存解得开的串——解码即验整条转授链的签名。
pub fn save_capability(record: &mut ProjectRecord, capability: &str) -> Result<()> {
    let bytes = hex_of(capability)?;
    decode_capability(&bytes)?;
    record.capability = hex::encode(bytes);
    save_record(record)
}

fn database_path(namespace: &str) -> Result<PathBuf> {
    let dir = collab_dir()?.join("db");
    std::fs::create_dir_all(&dir).with_context(|| format!("数据库目录创建失败：{}", dir.display()))?;
    Ok(dir.join(format!("{}.sqlite", namespace_key(namespace)?)))
}

fn check_format(format: u8, path: &Path) -> Result<()> {
    if format > FORMAT {
        bail!(
            "文件版本 {format} 比本程序支持的 {FORMAT} 新，拒绝改写：{}",
            path.display()
        );
    }
    Ok(())
}

/// 定长密钥的十六进制。成员标识、命名空间标识与密钥种子同为 32 字节，共用一个解码。
fn seed_of(text: &str) -> Result<[u8; SEED_LEN]> {
    Ok(hex_of(text)?
        .as_slice()
        .try_into()
        .map_err(|_| anyhow::anyhow!("长度不是 {SEED_LEN} 字节"))?)
}

fn hex_of(text: &str) -> Result<Vec<u8>> {
    Ok(hex::decode(text.trim())?)
}

fn write_json<T: Serialize>(path: &Path, value: &T) -> Result<()> {
    let text =
        serde_json::to_string_pretty(value).with_context(|| format!("序列化失败：{}", path.display()))?;
    write_atomic(path, &text)
}

/// 原子写：临时文件 + 刷盘 + rename。半途而废的密钥文件比没有密钥文件更糟。
pub(crate) fn write_atomic(path: &Path, text: &str) -> Result<()> {
    let tmp = path.with_extension(format!("tmp-{}", std::process::id()));
    let result = (|| -> Result<()> {
        use std::io::Write;
        let mut file = std::fs::File::create(&tmp)?;
        file.write_all(text.as_bytes())?;
        // 没刷盘就 rename，掉电后会留下一个盖掉旧文件的 0 字节文件。
        file.sync_all()?;
        std::fs::rename(&tmp, path)?;
        Ok(())
    })();
    if result.is_err() {
        let _ = std::fs::remove_file(&tmp);
    }
    result.with_context(|| format!("写盘失败：{}", path.display()))
}

#[cfg(unix)]
fn restrict(path: &Path) {
    use std::os::unix::fs::PermissionsExt;
    let _ = std::fs::set_permissions(path, std::fs::Permissions::from_mode(0o600));
}

/// 目录要的是可穿透的 0700（0600 连自己进不去）。锁住 `collab/` 一层，
/// 下面的会话与数据库目录就都跟着只有本机用户可见。
#[cfg(unix)]
fn restrict_dir(path: &Path) {
    use std::os::unix::fs::PermissionsExt;
    let _ = std::fs::set_permissions(path, std::fs::Permissions::from_mode(0o700));
}

#[cfg(not(unix))]
fn restrict(_path: &Path) {}

#[cfg(not(unix))]
fn restrict_dir(_path: &Path) {}
