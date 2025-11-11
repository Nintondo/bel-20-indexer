use bel_20_node::server::Server;
use std::sync::Arc;

pub struct TestServer {
    pub raw_rx: kanal::Receiver<bel_20_node::server::RawServerEvent>,
    pub tx: tokio::sync::broadcast::Sender<bel_20_node::server::ServerEvent>,
    pub server: Arc<Server>,
    pub _tmp: tempfile::TempDir,
}

pub fn temp_server() -> anyhow::Result<TestServer> {
    let tmp = tempfile::tempdir()?;
    let path = tmp.path().to_str().unwrap();
    let (raw_rx, tx, server) = Server::new(path)?;
    Ok(TestServer { raw_rx, tx, server: Arc::new(server), _tmp: tmp })
}

pub async fn drain_events(rx: &mut tokio::sync::broadcast::Receiver<bel_20_node::server::ServerEvent>) -> Vec<bel_20_node::server::ServerEvent> {
    let mut out = Vec::new();
    loop {
        match rx.try_recv() {
            Ok(ev) => out.push(ev),
            Err(tokio::sync::broadcast::error::TryRecvError::Empty) => break,
            Err(_) => break,
        }
    }
    out
}

