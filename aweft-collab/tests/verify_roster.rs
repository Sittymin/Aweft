//! 设计验证 6：名册——收紧、连带撤销与复活。
//!
//! 对应「场景验证」里创建者收紧 Bob 那几步，全程不碰网络，只把真凭证喂给 `roster.rs`
//! 那两条规则。断言四件事：收紧即刻让旧凭证不再现行并连坐其下游、被撤的人不能自我复活、
//! 收窄之后再转授仍然有效、墓碑跟着名册落盘。

use std::path::PathBuf;

use aweft_collab::cap::{encode_capability, row_area, track_area, Member, Namespace};
use aweft_collab::roster::{roster_path, Adoption, Roster};
use aweft_model::ids::ItemId;
use uuid::Uuid;
use willow25::authorisation::WriteCapability;
use willow25::groupings::Area;
use willow25::prelude::SubspaceId;

// 抄自 `verify_cap_wire.rs`：该种子派生的命名空间标识末字节为奇数，即自有。
const NAMESPACE_SEED: [u8; 32] = [
    1, 9, 9, 9, 9, 9, 9, 9, 9, 9, 9, 9, 9, 9, 9, 9, 9, 9, 9, 9, 9, 9, 9, 9, 9, 9, 9, 9, 9, 9, 9, 9,
];

fn member(byte: u8) -> Member {
    Member::from_seed(&[byte; 32])
}

fn id(byte: u8) -> ItemId {
    ItemId(Uuid::from_bytes([byte; 16]))
}

/// 处置结果单独取：`anyhow::Error` 没有 `PartialEq`，比 `Ok(..)` 编不过。
fn adopt(roster: &mut Roster, announcer: &SubspaceId, cap: &WriteCapability) -> Adoption {
    roster.apply(announcer, cap).expect("名册处置不该失败")
}

/// 三个角色与两份已签发的凭证：所有者把全工程签给 Bob，Bob 把字幕轨转授给 Carol。
struct Cast {
    namespace: Namespace,
    owner: Member,
    bob: Member,
    carol: Member,
    owner_cap: WriteCapability,
    bob_wide: WriteCapability,
    carol_from_bob: WriteCapability,
    subtitle: ItemId,
}

fn cast() -> Cast {
    let namespace = Namespace::from_seed(&NAMESPACE_SEED);
    let (owner, bob, carol) = (member(11), member(22), member(33));
    let subtitle = id(1);
    let owner_cap = namespace.owner_capability(&owner).expect("创世凭证");
    let bob_wide = namespace
        .delegate(&owner_cap, &owner, Area::full(), bob.id())
        .expect("根级转授");
    let carol_from_bob = namespace
        .delegate(&bob_wide, &bob, track_area(subtitle), carol.id())
        .expect("Bob 向下转授");
    Cast {
        namespace,
        owner,
        bob,
        carol,
        owner_cap,
        bob_wide,
        carol_from_bob,
        subtitle,
    }
}

impl Cast {
    /// 所有者把 Bob 收回「仅字幕轨」：凭的是他自己那张创世凭证，不是 Bob 那张。
    fn bob_narrow(&self) -> WriteCapability {
        self.namespace
            .delegate(
                &self.owner_cap,
                &self.owner,
                track_area(self.subtitle),
                self.bob.id(),
            )
            .expect("收紧签发")
    }
}

/// 名册文件的临时位置：`target/scratch/` 下（`/tmp` 是 tmpfs），每次先清掉旧文件。
fn scratch_roster(who: &str) -> PathBuf {
    let dir = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .expect("crate 应当在工程目录下")
        .join("target/scratch/roster");
    std::fs::create_dir_all(&dir).expect("scratch 目录应当能创建");
    let database = dir.join(format!("{who}.sqlite"));
    let path = roster_path(Some(&database)).expect("给了库路径就应当有名册文件位置");
    let _ = std::fs::remove_file(&path);
    path
}

#[test]
fn tightening_revokes_the_old_capability_and_everything_derived_from_it() {
    let cast = cast();
    let mut roster =
        Roster::open(encode_capability(&cast.owner_cap), None).expect("名册应当开得起来");

    assert_eq!(
        adopt(&mut roster, cast.owner.id(), &cast.bob_wide),
        Adoption::Accepted
    );
    assert_eq!(
        adopt(&mut roster, cast.bob.id(), &cast.carol_from_bob),
        Adoption::Accepted
    );
    assert!(roster.is_current(cast.bob.id(), &cast.bob_wide));
    assert!(roster.is_current(cast.carol.id(), &cast.carol_from_bob));

    let bob_narrow = cast.bob_narrow();
    assert_eq!(
        adopt(&mut roster, cast.owner.id(), &bob_narrow),
        Adoption::Accepted,
        "所有者重签一张窄的即为收紧"
    );
    assert!(
        !roster.is_current(cast.bob.id(), &cast.bob_wide),
        "收紧之后旧凭证不再现行，Bob 拿它写范围外的行也该被丢"
    );
    assert!(roster.is_current(cast.bob.id(), &bob_narrow));
    assert!(
        !roster.is_current(cast.carol.id(), &cast.carol_from_bob),
        "Bob 被换掉的那张派生出的下游一并作废，Carol 不可能比 Bob 更宽"
    );

    // 三条翻案的路都要堵死：本人认领旧那张、原转授者替他重发、陌生人替他重发。
    assert_eq!(
        adopt(&mut roster, cast.carol.id(), &cast.carol_from_bob),
        Adoption::Ignored,
        "墓碑只有权威重签才能复活"
    );
    assert_eq!(
        adopt(&mut roster, cast.bob.id(), &cast.carol_from_bob),
        Adoption::Ignored,
        "Bob 的现行已是窄的那张，旧链上的转授签不出来"
    );
    let stranger = member(77);
    assert_eq!(
        adopt(&mut roster, stranger.id(), &cast.carol_from_bob),
        Adoption::Ignored,
        "宣布者必须正是这条链最后一段的签名者"
    );
    assert!(!roster.is_current(cast.carol.id(), &cast.carol_from_bob));

    // 撤销不是放逐：收窄后的 Bob 重新往下转授仍然有效。
    let carol_narrow = cast
        .namespace
        .delegate(
            &bob_narrow,
            &cast.bob,
            row_area(cast.subtitle, id(3)),
            cast.carol.id(),
        )
        .expect("从窄凭证再转授");
    assert_eq!(
        adopt(&mut roster, cast.bob.id(), &carol_narrow),
        Adoption::Accepted
    );
    assert!(roster.is_current(cast.carol.id(), &carol_narrow));
}

#[test]
fn a_tightened_member_adopts_the_narrow_capability_as_his_own() {
    let cast = cast();
    let mut roster =
        Roster::open(encode_capability(&cast.bob_wide), None).expect("名册应当开得起来");
    adopt(&mut roster, cast.bob.id(), &cast.carol_from_bob);

    // 收紧的通告来自所有者，而本机名册里根本没有他这一条：这一支只看链上的签名，
    // 否则 Bob 永远收不到自己的收紧。
    let bob_narrow = cast.bob_narrow();
    assert_eq!(
        adopt(&mut roster, cast.owner.id(), &bob_narrow),
        Adoption::Ours,
        "接收者是本机，调用方要把这张写回会话记录"
    );
    let narrow_bytes = encode_capability(&bob_narrow);
    assert_eq!(
        roster.own(),
        narrow_bytes.as_slice(),
        "本机凭证得跟着换，下一条消息就用窄的这张"
    );
    assert_eq!(
        adopt(&mut roster, cast.bob.id(), &cast.bob_wide),
        Adoption::Ignored,
        "把自己那张旧凭证重发一遍不算翻案"
    );
    assert!(!roster.is_current(cast.bob.id(), &cast.bob_wide));
    assert!(
        roster.is_current(cast.bob.id(), &bob_narrow),
        "连坐下游时不该把刚采纳的自己判成墓碑"
    );
    assert!(
        !roster.is_current(cast.carol.id(), &cast.carol_from_bob),
        "自我收窄同样连坐下游"
    );
}

#[test]
fn a_grant_can_only_be_claimed_by_the_one_it_was_signed_for() {
    let cast = cast();
    let mut roster =
        Roster::open(encode_capability(&cast.owner_cap), None).expect("名册应当开得起来");
    let dave = member(44);
    let dave_cap = cast
        .namespace
        .delegate(&cast.owner_cap, &cast.owner, track_area(id(9)), dave.id())
        .expect("给 Dave 的转授");

    // Dave 自己认领：链是所有者签的，他不是最后一段的签名者，挡下。
    assert_eq!(adopt(&mut roster, dave.id(), &dave_cap), Adoption::Ignored);
    assert_eq!(
        adopt(&mut roster, cast.owner.id(), &dave_cap),
        Adoption::Accepted,
        "换了所有者宣布就成立"
    );
    // 名册里没有这一条时放行：缺通告不等于已撤销。
    let erin = member(55);
    let erin_cap = cast
        .namespace
        .delegate(&cast.owner_cap, &cast.owner, Area::full(), erin.id())
        .expect("给 Erin 的转授");
    assert!(roster.is_current(erin.id(), &erin_cap));
}

#[test]
fn tombstones_survive_a_restart() {
    let cast = cast();
    let path = scratch_roster("restart");
    let own = encode_capability(&cast.owner_cap);
    let bob_narrow = cast.bob_narrow();
    {
        let mut roster = Roster::open(own.clone(), Some(path.clone())).expect("名册应当开得起来");
        adopt(&mut roster, cast.owner.id(), &cast.bob_wide);
        adopt(&mut roster, cast.bob.id(), &cast.carol_from_bob);
        assert_eq!(
            adopt(&mut roster, cast.owner.id(), &bob_narrow),
            Adoption::Accepted
        );
        assert!(path.is_file(), "名册必须落盘，重连时已确认的通告不会再投喂");
    }

    let roster = Roster::open(own, Some(path)).expect("名册文件应当读得回来");
    assert!(roster.is_current(cast.bob.id(), &bob_narrow));
    assert!(!roster.is_current(cast.bob.id(), &cast.bob_wide));
    assert!(
        !roster.is_current(cast.carol.id(), &cast.carol_from_bob),
        "墓碑丢了就等于把 Carol 放回来"
    );
}
