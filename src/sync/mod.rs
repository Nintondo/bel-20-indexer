mod events;
pub mod export;
pub mod routes;
pub mod snapshots;
#[cfg(test)]
mod tests;

use crate::{Blockchain, Network, DB};
use anyhow::Context;
use nintondo_holdings_protocol::{BlockRef, Chain, SyncStatus, PROTOCOL_VERSION};

pub fn status(db: &DB) -> anyhow::Result<SyncStatus> {
    let chain = match *crate::BLOCKCHAIN {
        Blockchain::Bellscoin => Chain::Bells,
        Blockchain::Dogecoin => Chain::Doge,
        Blockchain::Pepecoin => Chain::Pepe,
        _ => anyhow::bail!("Holdings sync supports Bells, Doge and Pepe"),
    };
    let network = match *crate::NETWORK {
        Network::Testnet => nintondo_holdings_protocol::Network::Testnet,
        _ => nintondo_holdings_protocol::Network::Mainnet,
    };
    let height = db.last_block.get(()).context("Indexer has no committed tip")?;
    let info = db.block_info.get(height).context("Indexer has no committed block")?;
    Ok(SyncStatus {
        protocol_version: PROTOCOL_VERSION,
        chain,
        network,
        tip: BlockRef {
            height,
            hash: info.hash.to_string(),
        },
    })
}
