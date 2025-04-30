use std::collections::HashSet;

use crate::NETWORK;
use bellscoin::ScriptBuf;
use core_utils::types::full_hash::ComputeScriptHash;

use crate::inscriptions::ScriptToAddr;

use super::*;

#[derive(Clone)]
pub struct AddressHasher {
    pub server: Arc<Server>,
    pub addr_rx: kanal::Receiver<AddressesToLoad>,
    pub token: WaitToken,
}

pub struct AddressesToLoad {
    pub height: u32,
    pub addresses: HashSet<ScriptBuf>,
}

impl Handler for AddressHasher {
    async fn run(&mut self) -> anyhow::Result<()> {
        'outer: loop {
            let mut res = vec![];

            loop {
                match self.addr_rx.try_recv() {
                    Ok(Some(v)) => {
                        res.push(v);
                    }
                    Ok(None) => {
                        if res.is_empty() {
                            if self.token.is_cancelled() {
                                break 'outer;
                            }

                            tokio::time::sleep(tokio::time::Duration::from_millis(100)).await;
                            continue;
                        }
                        break;
                    }
                    Err(_) => {
                        if res.is_empty() {
                            break 'outer;
                        }
                    }
                }
            }

            let height = res.last().unwrap().height;

            let data = res
                .into_iter()
                .flat_map(|x| x.addresses)
                .unique()
                .filter_map(|x| {
                    x.to_address_str(*NETWORK)
                        .map(|v| (x.compute_script_hash(), v))
                });

            let db = self.server.db.clone();

            tokio::task::spawn_blocking(move || db.fullhash_to_address.extend(data))
                .await
                .anyhow()?;

            *self.server.last_indexed_address_height.write().await = height;
        }
        Ok(())
    }
}
