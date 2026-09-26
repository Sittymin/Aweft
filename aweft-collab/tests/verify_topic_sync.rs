//! 设计验证 3：`p2panda` 明文主题流在两节点之间同步消息。
//!
//! 对应设计草稿“同步语义”节。两 `Node` 以默认配置启动并订阅同一主题，
//! 一端发布后另一端断言收到 `StreamEvent::Processed`，且作者与内容正确。
//! 本测试不经过中继，同一进程内两节点经发现层直连。

use std::time::Duration;

use p2panda::streams::StreamEvent;
use p2panda::Topic;
use tokio_stream::StreamExt;

#[tokio::test]
async fn plain_topic_stream_syncs_between_two_nodes() {
    let topic = Topic::random();

    let alice = p2panda::builder().spawn().await.expect("节点 A 启动");
    let bob = p2panda::builder().spawn().await.expect("节点 B 启动");

    let (alice_tx, _alice_rx) = alice.stream::<String>(topic).await.expect("A 建流");
    alice_tx
        .publish("row42:1000-2000".to_string())
        .await
        .expect("A 发布");

    let (_bob_tx, mut bob_rx) = bob.stream::<String>(topic).await.expect("B 建流");

    let operation = tokio::time::timeout(Duration::from_secs(60), async {
        loop {
            match bob_rx.next().await {
                Some(StreamEvent::Processed { operation, .. }) => break operation,
                Some(_) => continue,
                None => panic!("B 的订阅流意外关闭"),
            }
        }
    })
    .await
    .expect("60 秒内应当同步完成");

    assert_eq!(operation.message(), "row42:1000-2000");
    assert_eq!(operation.author(), alice.id());
}
