//! 身份、路径与凭证：协作的授权层。
//!
//! 三件事在这里落地，互相咬合，缺一条整个权限模型就不成立：
//!
//! 1. 单一种子。32 字节种子同时构造 `p2panda` 的 `SigningKey` 和 `willow` 的
//!    `SubspaceSecret`（两边底层同为 `ed25519-dalek 2.2.0`），所以同一种子导出
//!    恒等的公钥字节。于是「谁签了这条网络消息」（`ProcessedOperation::author`）
//!    和「凭证授权给谁」（`WriteCapability::receiver`）是同一个 32 字节值，
//!    接收端不需要在消息里再验一层 `willow` 签名。
//! 2. 一工程一自有命名空间。命名空间密钥既是工程协作秘密，又是主题来源：
//!    `Topic` 直接取命名空间的 32 字节，一一对应（见 `topic_of`）。
//!    自有（`is_owned`）是转授的前提：共用命名空间的创世授权只覆盖创建者自己的
//!    子空间，不能向他人转授。
//! 3. 凭证随每条消息携带，接收端独立验算。`willow25` 的 `Decodable` 实现
//!    （`write_capability.rs:561`）在解码时就调 `is_valid()` 验完整条转授链的签名，
//!    所以「解得开」等于「链可信」，本模块只补三条链以外的判断（见 `authorize`）。

use anyhow::{bail, Context, Result};
use aweft_model::ids::ItemId;
use aweft_project::ProjectStore;
use p2panda::{SigningKey, Topic};
use ufotofu::codec_prelude::*;
use uuid::Uuid;
use willow25::authorisation::WriteCapability;
use willow25::groupings::Grouping;
use willow25::prelude::{
    Area, NamespaceId, NamespaceSecret, Path, SubspaceId, SubspaceSecret, TimeRange, Timestamp,
};

/// 成员种子与命名空间种子的长度：都是 ed25519 私钥种子。
pub const SEED_LEN: usize = 32;

/// 路径组件的长度：轨道组标识与行标识都是 `uuid` 的 16 字节裸值。
pub const ID_LEN: usize = 16;

/// 没有父分组的行挂在它下面：当作 0 号轨道组授权，与真实 `uuid` 不可能相撞。
pub const ROOT_TRACK: ItemId = ItemId(Uuid::nil());

/// 验算坐标里的时间戳。首版签发的凭证一律 `TimeRange::full()`，这个取值不参与判断；
/// 将来若按时间窗转授，这里要换成墙钟 `Timestamp::now()`。
fn coord_time() -> Timestamp {
    Timestamp::from(0)
}

/// 本机成员身份：一个种子同时是 `p2panda` 签名密钥和 `willow` 子空间密钥。
pub struct Member {
    seed: [u8; SEED_LEN],
    signing_key: SigningKey,
    subspace_secret: SubspaceSecret,
    subspace_id: SubspaceId,
}

impl Member {
    pub fn from_seed(seed: &[u8; SEED_LEN]) -> Self {
        let signing_key = SigningKey::from_bytes(seed);
        let subspace_secret = SubspaceSecret::from_bytes(seed);
        let subspace_id = subspace_secret.corresponding_subspace_id();
        Self {
            seed: *seed,
            signing_key,
            subspace_secret,
            subspace_id,
        }
    }

    /// 随机种子。`p2panda` 的 `SigningKey::generate` 取的是操作系统熵源，
    /// 不必为密钥生成再引一个随机数库。
    pub fn generate() -> Self {
        Self::from_seed(SigningKey::generate().as_bytes())
    }

    pub fn seed(&self) -> &[u8; SEED_LEN] {
        &self.seed
    }

    /// 成员标识：网络层的 `NodeId` 与授权层的 `SubspaceId` 是同一个值。
    pub fn id(&self) -> &SubspaceId {
        &self.subspace_id
    }

    pub fn signing_key(&self) -> SigningKey {
        self.signing_key.clone()
    }

    /// 十六进制的成员标识，用作 `Origin::Collab` 的标签和界面展示。
    pub fn hex_id(&self) -> String {
        hex::encode(self.subspace_id.to_bytes())
    }
}

/// 从 `p2panda` 的消息作者字节还原出 `willow` 成员标识。
/// 单一种子方案下这两者恒等，所以只需按字节重建；重建失败即消息来自非协作节点。
pub fn member_id_of_bytes(bytes: &[u8]) -> Result<SubspaceId> {
    let raw: [u8; SEED_LEN] = bytes
        .try_into()
        .map_err(|_| anyhow::anyhow!("作者标识长度不是 {SEED_LEN} 字节"))?;
    Ok(SubspaceId::from_bytes(&raw))
}

/// 一个工程的协作秘密：自有命名空间。
///
/// 所有者持有 `secret` 才能继续向新成员转授；被邀请者只持有 `id`，
/// 够用来验算别人发来的凭证，不够签发新的。
pub struct Namespace {
    secret: Option<NamespaceSecret>,
    id: NamespaceId,
}

impl Namespace {
    /// 所有者侧新建：滚动随机种子直到命名空间标识自有（末字节为奇数，约一半概率）。
    pub fn generate() -> Result<Self> {
        for _ in 0..64 {
            let candidate = Self::from_seed(SigningKey::generate().as_bytes());
            if candidate.id.is_owned() {
                return Ok(candidate);
            }
        }
        bail!("64 次都没能生成自有的命名空间标识，熵源可疑")
    }

    /// 从命名空间密钥种子构造（所有者）。
    pub fn from_seed(seed: &[u8; SEED_LEN]) -> Self {
        let secret = NamespaceSecret::from_bytes(seed);
        let id = secret.corresponding_namespace_id();
        Self {
            secret: Some(secret),
            id,
        }
    }

    /// 从命名空间标识构造（成员）：没有密钥，不能再转授。
    pub fn from_id_bytes(bytes: &[u8; SEED_LEN]) -> Self {
        Self {
            secret: None,
            id: NamespaceId::from_bytes(bytes),
        }
    }

    pub fn id(&self) -> &NamespaceId {
        &self.id
    }

    pub fn id_bytes(&self) -> [u8; SEED_LEN] {
        *self.id.as_bytes()
    }

    pub fn is_owned(&self) -> bool {
        self.id.is_owned()
    }

    /// 命名空间密钥种子，仅所有者持有。
    pub fn secret_bytes(&self) -> Option<[u8; SEED_LEN]> {
        self.secret.as_ref().map(|secret| *secret.as_bytes())
    }

    /// 主题即命名空间字节：一工程一流，派生关系一一对应。
    pub fn topic(&self) -> Topic {
        Topic::from_bytes(self.id.as_bytes()).expect("命名空间标识恰为主题长度")
    }

    /// 所有者为自己的全区域签发创世凭证。
    pub fn owner_capability(&self, owner: &Member) -> Result<WriteCapability> {
        let secret = self
            .secret
            .as_ref()
            .context("只有持命名空间密钥的所有者能签发创世凭证")?;
        Ok(WriteCapability::new_owned(secret, owner.id().clone()))
    }

    /// 把 `area` 范围的写权限转授给 `to`。签名用的是转授者自己的子空间密钥，
    /// 所以 `from` 必须是本成员持有、且 `area` 落在其已授权范围内的凭证。
    pub fn delegate(
        &self,
        from: &WriteCapability,
        by: &Member,
        area: Area,
        to: &SubspaceId,
    ) -> Result<WriteCapability> {
        let mut cap = from.clone();
        cap.try_delegate(&by.subspace_secret, area, to.clone())
            .map_err(|_| anyhow::anyhow!("转授失败：范围越界或签名密钥与凭证接收者不符"))?;
        Ok(cap)
    }
}

/// 一条轨道的授权区域：路径前缀 `/tracks/<轨道组标识>`，全时间。
pub fn track_area(track: ItemId) -> Area {
    Area::new(
        // 不绑定子空间：谁拿到这条凭证，谁就能在该轨道下写自己的行。
        None,
        track_path(track),
        TimeRange::full(),
    )
}

/// 单行的授权区域：路径 `/tracks/<轨道组标识>/<行标识>`，全时间。
pub fn row_area(track: ItemId, row: ItemId) -> Area {
    Area::new(None, row_path(track, row), TimeRange::full())
}

/// 轨道组标识 → 路径前缀。
pub fn track_path(track: ItemId) -> Path {
    Path::from_slices(&[b"tracks", id_bytes(track).as_slice()])
        .expect("固定前缀加 16 字节组件不会超出路径长度上限")
}

/// 行标识 → 完整路径：`/tracks/<轨道组标识>/<行标识>`。
pub fn row_path(track: ItemId, row: ItemId) -> Path {
    Path::from_slices(&[
        b"tracks",
        id_bytes(track).as_slice(),
        id_bytes(row).as_slice(),
    ])
    .expect("三段定长组件不会超出路径长度上限")
}

/// 从存储里取某行的两级标识（轨道组 + 行）。行不存在或不是字幕行时 `None`。
pub fn row_location(store: &ProjectStore, row: ItemId) -> Option<(ItemId, ItemId)> {
    let item = store.sequence().get(row)?;
    Some((item.parent.unwrap_or(ROOT_TRACK), row))
}

/// 线上两个路径组件 → 完整路径与行标识。
/// 组件长度不对即非本协议消息，调用方按 `Err` 丢弃并记日志。
pub fn path_from_wire(track: &[u8], row: &[u8]) -> Result<(Path, ItemId)> {
    if track.len() != ID_LEN || row.len() != ID_LEN {
        bail!("路径组件长度不是 {ID_LEN} 字节");
    }
    let path = Path::from_slices(&[b"tracks", track, row]).context("路径组件不合法")?;
    let mut bytes = [0u8; ID_LEN];
    bytes.copy_from_slice(row);
    Ok((path, ItemId(Uuid::from_bytes(bytes))))
}

fn id_bytes(id: ItemId) -> [u8; ID_LEN] {
    id.0.into_bytes()
}

/// 凭证 → 线上字节。
pub fn encode_capability(cap: &WriteCapability) -> Vec<u8> {
    // `new_vec_storing_encoding` 只在内存 consumer 上跑，不会挂起。
    pollster::block_on(cap.new_vec_storing_encoding())
}

/// 线上字节 → 凭证。解码过程即验转授链签名，签名不符在这里就失败。
pub fn decode_capability(bytes: &[u8]) -> Result<WriteCapability> {
    // 必须是 `Vec` 的 producer：`&[u8]` 的 producer 产出 `&u8`，
    // 不满足 `willow25` 要求的 `BulkProducer<Item = u8>`。
    let mut producer = bytes.to_vec().into_producer();
    let cap: WriteCapability = pollster::block_on(producer.produce_decoded())
        .map_err(|e| anyhow::anyhow!("凭证解码或验签失败：{e:?}"))?;
    Ok(cap)
}

/// 接收端验算：这条消息允许落成这次写入吗。
///
/// 解码（[`decode_capability`]）已经验完整条转授链的签名，本函数补三条链以外的判断：
/// 1. 凭证属于本工程（防跨工程重放：别的工程发来的合法凭证不能在这里生效）。
/// 2. 凭证的接收者就是签名者（防「拿别人的凭证冒充写入」）。
/// 3. 凭证覆盖目标坐标（防越界写入）。
///
/// 这三条只说明「这张凭证有资格写这一行」，不说明「它还是这个人的现行凭证」——
/// 收紧权限靠后者，见 `roster`。凭证在这里解码一次，调用方把结果同时交给名册比对。
pub fn authorize(
    namespace: &Namespace,
    author: &SubspaceId,
    cap: &WriteCapability,
    path: &Path,
) -> Result<()> {
    if cap.granted_namespace() != namespace.id() {
        bail!("凭证不属于本工程命名空间，丢弃");
    }
    if cap.receiver() != author {
        bail!("凭证的接收者与消息签名者不符，丢弃");
    }
    let coord = (author.clone(), path.clone(), coord_time());
    if !cap.granted_area().includes(&coord) {
        bail!("凭证范围不覆盖目标路径，丢弃");
    }
    Ok(())
}

/// 这条凭证最后一次转授是谁签的。
///
/// 转授链上每一段的签名者都是上一段的接收者，所以「最后一段的签名者」= 倒数第二段
/// 的接收者；只有一段的链由命名空间所有者在创世授权里签，签名者就是 `genesis.user_key`；
/// 零段的链（所有者自己的凭证）没有转授，署名人同样是创世接收者。
/// 通告一条新凭证的人必须正是他，否则任何人都能替别人宣布权限变更。
pub fn last_delegator(cap: &WriteCapability) -> &SubspaceId {
    let delegations = cap.delegations();
    match delegations.len() {
        0 | 1 => cap.genesis().user_key(),
        count => &delegations[count - 2].user,
    }
}

/// `base` 的转授链是否正好是 `longer` 去掉最后一段之后的那条链。
///
/// 转授只能在手里那张凭证上追加一段，所以这个判断等价于
/// 「`longer` 是从 `base` 这张现行凭证派生出来的」。`base` 零段时它要求 `longer` 恰好一段，
/// 也就是所有者直接签给某人的那张——所有者的创世凭证是全区域，重签一张即为收紧或撤销。
pub fn extends(base: &WriteCapability, longer: &WriteCapability) -> bool {
    if base.genesis() != longer.genesis() {
        return false;
    }
    let delegations = longer.delegations();
    let head = match delegations.len() {
        0 => return false,
        count => &delegations[..count - 1],
    };
    base.delegations() == head
}

/// `shorter` 的链是否为 `longer` 的严格前缀（`longer` 由它派生且更长）。
/// 收紧某成员时，凭它把该成员旧凭证派生出的下游一并清掉。
pub fn derived_from(shorter: &WriteCapability, longer: &WriteCapability) -> bool {
    if shorter.genesis() != longer.genesis() {
        return false;
    }
    let (mine, theirs) = (shorter.delegations(), longer.delegations());
    mine.len() < theirs.len() && theirs.starts_with(mine)
}

/// 凭证是否为 All 范围（全工程）。快照要求 All，否则 Track 级成员也能覆盖整库。
pub fn is_full_area(cap: &WriteCapability) -> bool {
    cap.granted_area() == willow25::prelude::Area::full()
}
