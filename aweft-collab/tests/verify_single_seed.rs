//! 设计验证 1：单一种子同时驱动 `p2panda` 签名密钥与 `willow` 子空间密钥。
//!
//! 对应设计草稿“身份与密钥”节。断言同一组 32 字节种子在两边导出恒等的公钥字节，
//! 因此队友只需交换一次公钥，不存在两套身份。

use p2panda::SigningKey;
use willow25::prelude::{SubspaceId, SubspaceSecret};

#[test]
fn same_seed_drives_both_key_types() {
    let seed = [7u8; 32];

    let panda_sk = SigningKey::from_bytes(&seed);
    let willow_sk = SubspaceSecret::from_bytes(&seed);

    let panda_pk = panda_sk.verifying_key();
    let willow_id = willow_sk.corresponding_subspace_id();

    // 两边公钥字节恒等：`p2panda` 成员标识即 `willow` 子空间标识。
    assert_eq!(panda_pk.as_bytes(), &willow_id.to_bytes());

    // `p2panda` 公钥字节可以直接重建出同一 `SubspaceId`。
    let rebuilt = SubspaceId::from_bytes(panda_pk.as_bytes());
    assert_eq!(rebuilt, willow_id);
}
