//! 设计验证 2：`Meadowcap` 自有命名空间的轨道级转授范围在接收端可验算。
//!
//! 对应设计草稿“权限”节。项目即自有命名空间，所有者持命名空间密钥签发
//! 覆盖全区域的创世凭证，再用自己的子空间密钥向编辑者转授 `/tracks/t1`
//! 子范围。断言范围内坐标被含、范围外坐标被拒、错钥转授失败。
//! 命名空间字节到 `p2panda` 主题的派生关系一并在此断言（一一对应）。

use p2panda::Topic;
use willow25::authorisation::WriteCapability;
use willow25::groupings::Grouping;
use willow25::prelude::{
    Area, NamespaceSecret, Path, SubspaceId, SubspaceSecret, TimeRange, Timestamp,
};

// 首字节经搜索确定：该种子派生的命名空间标识末字节为奇数，即自有命名空间。
const NAMESPACE_SEED: [u8; 32] = [
    1, 9, 9, 9, 9, 9, 9, 9, 9, 9, 9, 9, 9, 9, 9, 9, 9, 9, 9, 9, 9, 9, 9, 9, 9, 9, 9, 9, 9, 9, 9, 9,
];
const OWNER_SEED: [u8; 32] = [11; 32];
const EDITOR_SEED: [u8; 32] = [22; 32];

fn track_area(track: &str) -> Area {
    Area::new(
        None,
        Path::from_slices(&[b"tracks", track.as_bytes()]).expect("测试路径合法"),
        TimeRange::full(),
    )
}

fn row_coord(track: &str, row: &str, author: &SubspaceId) -> (SubspaceId, Path, Timestamp) {
    (
        author.clone(),
        Path::from_slices(&[b"tracks", track.as_bytes(), row.as_bytes()])
            .expect("测试路径合法"),
        Timestamp::from(1),
    )
}

#[test]
fn delegated_track_scope_is_enforced() {
    // 项目即自有命名空间：命名空间标识由项目密钥派生，末字节为奇数即自有。
    let namespace_secret = NamespaceSecret::from_bytes(&NAMESPACE_SEED);
    let namespace = namespace_secret.corresponding_namespace_id();
    assert!(namespace.is_owned());

    let owner_secret = SubspaceSecret::from_bytes(&OWNER_SEED);
    let owner_id = owner_secret.corresponding_subspace_id();
    let editor_secret = SubspaceSecret::from_bytes(&EDITOR_SEED);
    let editor_id = editor_secret.corresponding_subspace_id();

    // 所有者用命名空间密钥签发覆盖全区域的创世凭证，接收者为所有者自己。
    let owner_cap = WriteCapability::new_owned(&namespace_secret, owner_id.clone());
    assert_eq!(owner_cap.granted_namespace(), &namespace);
    assert_eq!(owner_cap.granted_area(), Area::full());

    // 所有者用自己的子空间密钥把 `/tracks/t1` 子范围转授给编辑者。
    let mut editor_cap = owner_cap.clone();
    editor_cap
        .try_delegate(&owner_secret, track_area("t1"), editor_id.clone())
        .expect("所有者转授应当成功");
    assert_eq!(editor_cap.receiver(), &editor_id);

    // 范围内坐标被含：编辑者在 t1 轨道写任意行。
    let granted = editor_cap.granted_area();
    assert!(granted.includes(&row_coord("t1", "row1", &editor_id)));

    // 范围外坐标被拒：编辑者越界写 t2 轨道。
    assert!(!granted.includes(&row_coord("t2", "row1", &editor_id)));

    // 错钥转授失败：编辑者不能冒充所有者继续转授。
    let mut forged = owner_cap.clone();
    assert!(
        forged
            .try_delegate(&editor_secret, track_area("t1"), editor_id.clone())
            .is_err()
    );
}

#[test]
fn namespace_bytes_derive_topic_one_to_one() {
    // 命名空间字节即主题字节，派生关系是一一对应。
    let namespace = NamespaceSecret::from_bytes(&NAMESPACE_SEED).corresponding_namespace_id();
    let topic = Topic::from_bytes(namespace.as_bytes()).expect("32 字节可建主题");
    assert_eq!(topic.as_bytes(), namespace.as_bytes());
}
