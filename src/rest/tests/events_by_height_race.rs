#![allow(clippy::uninlined_format_args)]

use super::*;

#[cfg(feature = "failpoints")]
#[tokio::test(flavor = "current_thread"])
async fn events_by_height_race_panics_or_errors() {
    // This test simulates the transient window: block_events keys exist while
    // address_token_to_history values are not yet persisted. The handler currently
    // uses multi_get_kv(..., true) and will panic if values are missing.

    std::env::set_var("RPC_URL", "http://127.0.0.1:8332");
    std::env::set_var("RPC_USER", "user");
    std::env::set_var("RPC_PASS", "pass");
    std::env::set_var("BLOCKCHAIN", "bellscoin");

    let tmp = tempfile::tempdir().expect("tempdir");
    let (_raw_rx, _tx, server0) = Server::new(tmp.path().to_str().unwrap()).expect("server");
    let server = std::sync::Arc::new(server0);

    let height = *START_HEIGHT + 22;
    let tick = OriginalTokenTick::from_str("RACE").unwrap();
    let key = AddressTokenIdDB { address: FullHash::ZERO, token: tick, id: 1 };
    let hv = HistoryValue { height, action: TokenHistoryDB::Deploy { max: Fixed128::from(0), lim: Fixed128::from(0), dec: 0, txid: Txid::all_zeros(), vout: 0 } };

    let _scenario = fail::FailScenario::setup();
    fail::cfg("after_block_events_set", "sleep(500)").unwrap();

    // Spawn blocking writer; it will pause after block_events.set
    let srv2 = server.clone();
    let handle = std::thread::spawn(move || {
        let data = ProcessedData::History { block_number: height, last_history_id: 1, history: vec![(key, hv)] };
        data.write(&srv2, None);
    });

    // Give it time to hit the failpoint
    tokio::time::sleep(std::time::Duration::from_millis(100)).await;

    // Now call the handler concurrently; it should panic (or error) due to missing values
    let server3 = server.clone();
    let j = tokio::spawn(async move { history::events_by_height(State(server3), Path(height)).await });
    match j.await {
        Err(e) if e.is_panic() => { /* expected */ }
        Ok(Err(_)) => { /* acceptable error path if not panicking */ }
        Ok(Ok(_)) => panic!("unexpected success during race window"),
        Err(e) => panic!("unexpected join error: {e}"),
    }

    // Finish writer and ensure join
    handle.join().unwrap();
}

