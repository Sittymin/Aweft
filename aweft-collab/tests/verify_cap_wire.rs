//! 设计验证 4a：凭证的线上往返与接收端验算。
//!
//! 对应设计草稿“分层归属”的授权层与 `cap.rs`。断言四件事：
//! 编码解码后凭证不变、范围内坐标放行、越界路径被拒、冒充作者与跨工程重放被拒。

use aweft_collab::cap::{
    authorize, decode_capability, encode_capability, row_location, row_path, track_area, Namespace,
};
use aweft_model::ids::ItemId;
use uuid::Uuid;
use willow25::groupings::Grouping;
use willow25::prelude::SubspaceId;

// 抄自 `verify_meadowcap_scoping.rs`：该种子派生的命名空间标识末字节为奇数，即自有。
const NAMESPACE_SEED: [u8; 32] = [
    1, 9, 9, 9, 9, 9, 9, 9, 9, 9, 9, 9, 9, 9, 9, 9, 9, 9, 9, 9, 9, 9, 9, 9, 9, 9, 9, 9, 9, 9, 9, 9,
];

fn member(byte: u8) -> aweft_collab::cap::Member {
    aweft_collab::cap::Member::from_seed(&[byte; 32])
}

fn id(byte: u8) -> ItemId {
    ItemId(Uuid::from_bytes([byte; 16]))
}

fn subspace(byte: u8) -> SubspaceId {
    member(byte).id().clone()
}

#[test]
fn capability_survives_the_wire_unchanged() {
    let namespace = Namespace::from_seed(&NAMESPACE_SEED);
    let owner = member(11);
    let editor = member(22);

    let owner_cap = namespace
        .owner_capability(&owner)
        .expect("创世凭证签发失败");
    let granted = namespace
        .delegate(&owner_cap, &owner, track_area(id(1)), editor.id())
        .expect("轨道转授应当成功");

    let bytes = encode_capability(&granted);
    let back = decode_capability(&bytes).expect("凭证应当能解回来");

    assert_eq!(back.receiver(), granted.receiver());
    assert_eq!(back.granted_area(), granted.granted_area());
    assert_eq!(back.granted_namespace(), granted.granted_namespace());
    // 整体相等，而不只是字段相等：Meadowcap 的凭证里没有时间戳也没有随机量，
    // 同一张转授永远编出同一串字节。名册判「还是不是现行那张」就靠这条。
    assert_eq!(back, granted);
    // 路径映射与坐标包含关系随凭证一起成立：`/tracks/t1/row` 在 `/tracks/t1` 之下。
    assert!(back.granted_area().includes(&(
        editor.id().clone(),
        row_path(id(1), id(7)),
        0u64.into()
    )));
}

#[test]
fn authorize_accepts_only_the_granted_coordinate() {
    let namespace = Namespace::from_seed(&NAMESPACE_SEED);
    let owner = member(11);
    let editor = member(22);
    let outsider = member(33);

    let owner_cap = namespace
        .owner_capability(&owner)
        .expect("创世凭证签发失败");
    let cap = namespace
        .delegate(&owner_cap, &owner, track_area(id(1)), editor.id())
        .expect("轨道转授应当成功");
    let cap_bytes = encode_capability(&cap);

    // 范围内：编辑者在 1 号轨道写 7 号行。
    authorize(&namespace, &subspace(22), &cap, &row_path(id(1), id(7)))
        .expect("范围内的合法凭证应当放行");

    // 越界：同一凭证写 2 号轨道。
    assert!(authorize(&namespace, &subspace(22), &cap, &row_path(id(2), id(7))).is_err());

    // 冒充：凭证是给编辑者的，签名者却是别人。
    assert!(authorize(&namespace, outsider.id(), &cap, &row_path(id(1), id(7))).is_err());

    // 跨工程重放：凭证合法，但不属于这个命名空间。
    let other = Namespace::from_seed(&[3u8; 32]);
    assert!(authorize(&other, &subspace(22), &cap, &row_path(id(1), id(7))).is_err());

    // 篡改字节：解码即验整条转授链的签名，改一个字节就解不开。
    let mut broken = cap_bytes.clone();
    broken[0] ^= 0b1000_0000;
    assert!(
        decode_capability(&broken).is_err(),
        "被改过的凭证应当在解码时就失败，而不是留到验算"
    );
}

#[test]
fn row_location_falls_back_to_the_root_track() {
    use aweft_model::sequence::{EventData, ItemLocation};
    use aweft_project::{ProjectStore, SavedProject};
    use chrono::TimeDelta;

    let mut store = ProjectStore::new(SavedProject::new());
    let data = EventData {
        start_time: TimeDelta::milliseconds(0),
        end_time: TimeDelta::milliseconds(100),
        actor: None,
        text: String::new(),
        segment_timing: None,
        style: String::new(),
    };
    let group = store
        .sequence_mut()
        .create_group(
            aweft_model::sequence::GroupData {
                name: "轨道 1".into(),
                collapsed: false,
                color: None,
                children: vec![],
            },
            ItemLocation::Root(0),
        )
        .expect("分组创建失败");
    let row = store
        .sequence_mut()
        .create_event(data, ItemLocation::InGroup(group, 0))
        .expect("行创建失败");

    assert_eq!(row_location(&store, row), Some((group, row)));
    // 不存在的行没有路径可发：协作侧据此丢弃回执。
    assert_eq!(row_location(&store, id(200)), None);
}
