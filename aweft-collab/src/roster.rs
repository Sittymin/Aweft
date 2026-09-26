//! 名册：每个成员**现行**凭证的登记表，收紧与撤销唯一真正生效的地方。
//!
//! Meadowcap 的凭证只证明「签发当时授权过」，证明不了「现在仍然授权」。规范给的两条
//! 出路在这里都不够用：时间窗到期只能冻住未来的写入（我们的凭证暂不带到期），
//! 「用更晚时间戳的条目盖掉旧条目」要求盖的人持有那个子空间的密钥，
//! 而本工程每个人的行都写在自己的子空间里（见 `cap::authorize` 的第二条）。
//! 所以「把 Bob 从全工程收紧到只改字幕行」这件事，只能靠这张表立刻反映出来：
//! 表里 Bob 只有窄的那张，他再拿旧的那张来写就不被接受。
//!
//! 两条规则撑起全部语义：
//!
//! 1. **换掉谁的凭证，他旧凭证派生的下游一并立成墓碑**（[`derived_from`]）。
//!    转授是在手里那张凭证后面追加一段，所以 Bob 被收紧后，`所有者→Bob→Carol` 这类
//!    以 Bob 旧凭证为前缀的条目全部作废，Carol 的权限不可能比 Bob 更宽。
//!    作废是留一条 `None` 而不是删键：删了键就落进下面「不认识即放行」那一支，
//!    被撤的人拿旧凭证照样写得进去。墓碑跟着名册一起落盘，重启后仍然作数。
//! 2. **新凭证要么由有权签它的人宣布，要么是本人认领自己那张**。分两种宣布者：
//!    - 宣布者是别人的转授者：他必须正是这条链最后一段的签名者；再看他在名册里的状态——
//!      从没见过他即只看签名（本机名册里通常没有所有者那一条，收紧就是靠这一支进来的），
//!      已是墓碑则一律不算，现行凭证要求新链去掉最后一段后正好等于他手里那张。
//!      凭此 Bob 只能转授自己还持有的权限，被收紧之后连这张也签不出。
//!    - 宣布者就是接收者本人：只允许认领名册里已有那张，或从自己那张再追加一段
//!      （自我收窄）。这条限制是必需的——否则收紧之后 Bob 只需把自己那张旧凭证
//!      重新宣布一遍（链的签名依然合法），名册就被打回宽的了。
//!
//! 挡旧通告反悔的是**顺序**而不是这张表：同一个人签出的收紧必然排在放宽之后，
//! 而单个作者的日志是因果有序的（`StreamFrom::Frontier` 又保证确认过的操作不再重放），
//! 所以「先收到宽、后收到窄」是唯一的到达顺序。别人替他转发的通告过不了第一段那条判断。
//!
//! 密码学部分不在这里：整条链的签名由 [`decode_capability`] 在解码时验完，
//! 「范围不越界」由 `willow` 的逐段包含判断保证。本模块只回答「还是不是现行那张」。
//!
//! 落盘是必需的而不是优化：重连时已确认过的操作不会再投喂给我们（`StreamFrom::Frontier`
//! 读的是库里的游标），内存里的名册会跟着重启清空，而清空等于回到「不认识即放行」。

use anyhow::{Context, Result};
use aweft_storage::{read_json, write_json, JsonFile};
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use willow25::authorisation::WriteCapability;
use willow25::prelude::SubspaceId;

use crate::cap::{
    decode_capability, derived_from, encode_capability, extends, last_delegator, SEED_LEN,
};

/// 一次通告的处置结果。
#[derive(Debug, PartialEq, Eq)]
pub enum Adoption {
    /// 名册未变动：重复通告，或宣布者给不起这张凭证。
    Ignored,
    /// 采纳为某个他人的现行凭证。
    Accepted,
    /// 采纳的正是本机的凭证：调用方要把它写回会话记录，
    /// 否则重启后又拿旧的一张上岗，而那张在本机名册里已经不是现行的了。
    Ours,
}

/// 一个成员：凭证的字节形式（落盘与比对用）与解码形式（派生判断用）。
#[derive(Clone)]
struct Entry {
    bytes: Vec<u8>,
    cap: WriteCapability,
}

/// 名册本体。键用成员标识的裸字节：排序稳定，落盘逐字节可 diff。
///
/// 值有三态：没有这个键 = 从没见过他；`None` = 见过但权限已被撤销（墓碑）；
/// `Some` = 现行凭证。**墓碑必须是独立的一态**——连带清掉下游如果只是删键，
/// 就退回了「不认识即放行」，被撤的人拿旧凭证照样写得进去。
pub struct Roster {
    entries: BTreeMap<[u8; SEED_LEN], Option<Entry>>,
    /// 本机现行凭证的字节：每条消息都附它。
    own: Vec<u8>,
    /// 本机成员标识：判定某张凭证是不是发给自己的。
    myself: SubspaceId,
    path: Option<PathBuf>,
}

impl Roster {
    /// 以本机凭证建名册。`path` 是名册文件，`None` 表示纯内存（进程内测试用）。
    ///
    /// 本机凭证解不开就直接失败：没有它整个会话无从谈起，而不是「先上线再等通告」。
    pub fn open(own: Vec<u8>, path: Option<PathBuf>) -> Result<Self> {
        let cap = decode_capability(&own)?;
        let myself = cap.receiver().clone();
        let mut roster = Self {
            entries: BTreeMap::new(),
            own,
            myself,
            path,
        };
        roster.insert_own_entry(cap);
        roster.load()?;
        Ok(roster)
    }

    pub fn own(&self) -> &[u8] {
        &self.own
    }

    /// 这条凭证是不是该成员的现行凭证。
    ///
    /// 名册里没有这个成员时放行：缺通告不等于已撤销。链的合法性解码时已验，
    /// 范围由 `cap::authorize` 验过覆盖目标行，此时拦下只会把「被转授者的写入先到、
    /// 转授通告后到」这种正常顺序变成白丢数据。撤销的强制力来自另一头——
    /// 收紧必然产生一张**不同**的现行凭证，被换掉的那张此后不再被接受；
    /// 被连带撤销的人落在墓碑那一态，比「不认识」更严。
    pub fn is_current(&self, member: &SubspaceId, cap: &WriteCapability) -> bool {
        match self.entries.get(&key(member)) {
            None => true,
            Some(None) => false,
            Some(Some(entry)) => &entry.cap == cap,
        }
    }

    /// 收到一条凭证通告：判定能否采纳，采纳则更新名册并连带撤销下游。
    ///
    /// 与命名空间有关的验算由调用方负责（见 `cap::authorize`）；这里只管名册那两条规则。
    /// 拒绝不报错：通告是尽力而为的消息，收下方给不起的通告是常态，返回值够调用方记日志。
    pub fn apply(&mut self, announcer: &SubspaceId, cap: &WriteCapability) -> Result<Adoption> {
        let receiver = cap.receiver();
        let current = self.entries.get(&key(receiver)).cloned().flatten();
        if current.as_ref().is_some_and(|entry| entry.cap == *cap) {
            return Ok(Adoption::Ignored);
        }

        // 本人认领只能在自己那张的基础上收窄；除此之外一律要求宣布者有权签这张凭证，
        // 墓碑状态也只有靠权威重签才能复活。
        let allowed = match &current {
            Some(entry) if announcer == receiver => extends(&entry.cap, cap),
            _ => self.authorises(announcer, cap),
        };
        if !allowed {
            return Ok(Adoption::Ignored);
        }

        // 先撤销下游再登记新的：自我收窄得到的新凭证正是被换掉那张的派生，
        // 反过来做会把自己刚放进去的那条判成墓碑。
        if let Some(previous) = &current {
            self.revoke_descendants(&previous.cap);
        }
        let bytes = encode_capability(cap);
        self.entries.insert(
            key(receiver),
            Some(Entry {
                bytes: bytes.clone(),
                cap: cap.clone(),
            }),
        );
        self.save()?;

        if receiver != &self.myself {
            return Ok(Adoption::Accepted);
        }
        self.own = bytes;
        Ok(Adoption::Ours)
    }

    /// 本机这一张登记进名册（建册时与读完文件后各一次，后一次覆盖前一次：
    /// 名册文件可能停在离线期间被收紧之前的状态，本机凭证以会话记录里那张为准）。
    fn insert_own_entry(&mut self, cap: WriteCapability) {
        let bytes = encode_capability(&cap);
        self.entries
            .insert(key(cap.receiver()), Some(Entry { bytes, cap }));
    }

    /// 宣布者有没有资格宣布这张凭证：必须正是最后一段的签名者，且他本人的状态允许。
    ///
    /// 没见过他时只看签名：解码已验完整条链，而「上一级的收紧」正是靠这一支才进得来
    /// ——本机名册里通常没有所有者那一条，他的收紧若按「不认识即拒绝」就会被挡在门外。
    /// 墓碑则反过来：他的授权已被收回，凭他签的链一律不算。
    fn authorises(&self, announcer: &SubspaceId, cap: &WriteCapability) -> bool {
        if last_delegator(cap) != announcer {
            return false;
        }
        match self.entries.get(&key(announcer)) {
            None => true,
            Some(None) => false,
            Some(Some(base)) => extends(&base.cap, cap),
        }
    }

    /// 把以 `superseded` 为严格前缀的条目立成墓碑：它们的授权来自那张已被换掉的凭证。
    /// 只立墓碑不删键，是为了让「被撤的人拿旧凭证来写」落到 [`Roster::is_current`]
    /// 里比「不认识」更严的那一态。
    fn revoke_descendants(&mut self, superseded: &WriteCapability) {
        let doomed: Vec<[u8; SEED_LEN]> = self
            .entries
            .iter()
            .filter_map(|(member, entry)| {
                entry
                    .as_ref()
                    .filter(|entry| derived_from(superseded, &entry.cap))
                    .map(|_| *member)
            })
            .collect();
        for member in doomed {
            self.entries.insert(member, None);
        }
    }

    fn load(&mut self) -> Result<()> {
        let Some(path) = &self.path else {
            return Ok(());
        };
        // 名册读不出就按空名册处理，不拦下上线：缺通告不等于已撤销（见 is_current）。
        // 只有确有文件时才读，首启没有文件是正常状态，不该记日志。
        if path.is_file() {
            match read_json::<RosterFile>(path) {
                Ok(file) => {
                    for (member, capability) in file.entries {
                        let record = match read_entry(&member, capability.as_deref()) {
                            Ok(record) => record,
                            Err(error) => {
                                log::warn!("名册条目无效，跳过（{member}）：{error:?}");
                                continue;
                            }
                        };
                        self.entries.insert(record.member, record.entry);
                    }
                }
                Err(error) => log::warn!("名册文件读不懂，按空名册处理：{error:?}"),
            }
        }
        let own = self.own.clone();
        let cap = decode_capability(&own)?;
        self.insert_own_entry(cap);
        Ok(())
    }

    fn save(&self) -> Result<()> {
        let Some(path) = &self.path else {
            return Ok(());
        };
        let file = RosterFile {
            format: RosterFile::FORMAT,
            entries: self
                .entries
                .iter()
                .map(|(member, entry)| {
                    (
                        hex::encode(member),
                        entry.as_ref().map(|entry| hex::encode(&entry.bytes)),
                    )
                })
                .collect(),
        };
        write_json(path, &file).context("名册写盘失败")
    }
}

/// 名册文件形状：成员标识 → 凭证字节（十六进制），值为 `null` 即墓碑（已撤销）。
#[derive(Serialize, Deserialize)]
#[serde(default)]
struct RosterFile {
    format: u32,
    entries: BTreeMap<String, Option<String>>,
}

impl Default for RosterFile {
    fn default() -> Self {
        Self {
            format: Self::FORMAT,
            entries: BTreeMap::new(),
        }
    }
}

impl JsonFile for RosterFile {
    const FORMAT: u32 = 1;

    fn format(&self) -> u32 {
        self.format
    }

    fn set_format(&mut self, format: u32) {
        self.format = format;
    }
}

/// 文件里一条记录解码后的样子。
struct Record {
    member: [u8; SEED_LEN],
    entry: Option<Entry>,
}

/// 一条名册记录：有凭证时验签并验接收者与键相符，防的是文件被手改或拷错工程。
/// 墓碑没有凭证可验，只认键。
fn read_entry(member: &str, capability: Option<&str>) -> Result<Record> {
    let raw = hex::decode(member.trim()).context("成员标识不是十六进制")?;
    let member: [u8; SEED_LEN] = raw
        .as_slice()
        .try_into()
        .map_err(|_| anyhow::anyhow!("成员标识长度不是 {SEED_LEN} 字节"))?;
    let Some(capability) = capability else {
        return Ok(Record {
            member,
            entry: None,
        });
    };
    let bytes = hex::decode(capability.trim()).context("凭证不是十六进制")?;
    let cap = decode_capability(&bytes)?;
    if member != *cap.receiver().as_bytes() {
        anyhow::bail!("名册条目的键与凭证接收者不符");
    }
    Ok(Record {
        member,
        entry: Some(Entry { bytes, cap }),
    })
}

fn key(member: &SubspaceId) -> [u8; SEED_LEN] {
    *member.as_bytes()
}

/// 名册文件的默认位置：与节点日志库并排，一个工程一份。
pub fn roster_path(database: Option<&Path>) -> Option<PathBuf> {
    database.map(|path| path.with_extension("roster.json"))
}
