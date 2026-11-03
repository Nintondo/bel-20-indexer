use nintondo_holdings_protocol::{SyncEvent, SyncStatus};

#[test]
fn identities_are_forward_compatible_and_notifications_roundtrip() {
    let status: SyncStatus =
        serde_json::from_str(r#"{"protocol_version":2,"chain":"futurechain","network":"regtest","tip":{"height":10,"hash":"abc"},"replay_start":5,"catching_up":false}"#).unwrap();
    assert_eq!(status.chain, "futurechain");
    for event in [
        SyncEvent::Ready,
        SyncEvent::NewBlock {
            height: 10,
            blockhash: "abc".into(),
        },
        SyncEvent::Reorg { blocks_count: 2, new_height: 8 },
    ] {
        let encoded = serde_json::to_vec(&event).unwrap();
        let _: SyncEvent = serde_json::from_slice(&encoded).unwrap();
    }
}
