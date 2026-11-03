use crate::{db::*, inscriptions::process_data::ProcessedData, FullHash};
use anyhow::Context;
use nintondo_holdings_protocol::{Balance, BlockRef, SyncBlock};
use std::collections::HashMap;

pub fn balance(address: String, key: AddressToken, value: TokenBalance, meta: TokenMetaDB) -> Balance {
    Balance {
        address,
        tick: key.token.to_string(),
        token_id: meta.genesis.to_string(),
        decimals: meta.proto.dec,
        available: value.balance.to_string(),
        transferable: value.transferable_balance.to_string(),
        transfers_count: value.transfers_count,
    }
}

pub fn block(db: &DB, height: u32, header: &nint_blk::proto::Hashed<nint_blk::proto::header::BlockHeader>, processed: &[ProcessedData]) -> anyhow::Result<SyncBlock> {
    let mut addresses: HashMap<FullHash, String> = HashMap::new();
    let mut metas = HashMap::new();
    for item in processed {
        match item {
            ProcessedData::FullHash { addresses: items } => addresses.extend(items.iter().cloned()),
            ProcessedData::Tokens { metas: items, .. } => metas.extend(items.iter().cloned()),
            _ => {}
        }
    }
    let mut balances = Vec::new();
    for item in processed {
        if let ProcessedData::Tokens { balances: items, .. } = item {
            for (key, value) in items {
                if db.address_token_to_balance.get(key).as_ref() == Some(value) {
                    continue;
                }
                // Non-standard scripts and burned balances have no wallet address.
                let Some(address) = addresses.get(&key.address).cloned().or_else(|| db.fullhash_to_address.get(key.address)) else {
                    continue;
                };
                let token = key.token.into();
                let meta = metas
                    .get(&token)
                    .cloned()
                    .or_else(|| db.token_to_meta.get(&token))
                    .context("Balance token has no metadata")?;
                balances.push(balance(address, *key, value.clone(), meta));
            }
        }
    }
    balances.sort_by(|a, b| (&a.address, &a.token_id).cmp(&(&b.address, &b.token_id)));
    Ok(SyncBlock {
        block: BlockRef {
            height,
            hash: header.hash.to_string(),
        },
        prev_hash: header.value.prev_hash.to_string(),
        created: header.value.timestamp,
        balances,
    })
}
