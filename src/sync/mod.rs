pub mod config;
mod error;
mod events;
pub mod limits;
mod replay;
pub mod routes;
pub mod snapshots;
pub mod storage;
#[cfg(test)]
mod tests;

use crate::{Blockchain, Network, Server};
use anyhow::Context;
use nintondo_holdings_protocol::{BlockRef, SyncStatus, PROTOCOL_VERSION};

pub fn identity(blockchain: Blockchain, network: Network) -> anyhow::Result<(String, String)> {
    let chain = match blockchain {
        Blockchain::Bellscoin => "bells",
        Blockchain::Dogecoin => "doge",
        Blockchain::Pepecoin => "pepe",
        Blockchain::Litecoin => "litecoin",
    };
    let network = match network {
        Network::Bellscoin => "mainnet",
        Network::Testnet => "testnet",
        Network::Regtest => "regtest",
        Network::Signet => "signet",
        _ => anyhow::bail!("Unsupported network"),
    };
    Ok((chain.into(), network.into()))
}

pub fn status(server: &Server) -> anyhow::Result<SyncStatus> {
    let storage::Identity::V1 { chain, network } = server.db.sync_identity.try_get(())?.context("Database identity missing")?;
    let height = server.db.last_block.try_get(())?.context("Indexer has no committed tip")?;
    let info = server.db.block_info.try_get(height)?.context("Indexer has no committed block")?;
    let start = server.db.sync_state.try_get(())?.context("Replay state missing")?.start();
    Ok(SyncStatus {
        protocol_version: PROTOCOL_VERSION,
        chain,
        network,
        tip: BlockRef {
            height,
            hash: info.hash.to_string(),
        },
        replay_start: start,
        catching_up: server.catching_up.load(std::sync::atomic::Ordering::Acquire),
    })
}
