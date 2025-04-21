use std::time::Duration;

use bellscoin::{
    consensus::{Decodable, ReadExt},
    hashes::hex::HexIterator,
};
use dutils::{error::ContextWrapper, wait_token::WaitToken};
use jsonrpc_async::Client;
use serde::de::DeserializeOwned;
use serde_json::{value::RawValue, Value};

pub struct AsyncClient {
    client: Client,
    token: WaitToken,
}

impl AsyncClient {
    pub async fn new(
        url: &str,
        user: Option<String>,
        pass: Option<String>,
        token: WaitToken,
    ) -> anyhow::Result<Self> {
        let client = Client::simple_http(url, user, pass)
            .await
            .anyhow_with("Invalid URL for RPC client")?;

        Ok(Self { client, token })
    }

    async fn request<T: DeserializeOwned>(
        &self,
        method: &str,
        params: &[Value],
    ) -> anyhow::Result<T> {
        let params = params
            .iter()
            .map(|x| RawValue::from_string(x.to_string()).anyhow_with("Failed to serialize params"))
            .collect::<anyhow::Result<Vec<_>>>()?;
        loop {
            if self.token.is_cancelled() {
                anyhow::bail!("Cancelled");
            }

            match self.client.call::<T>(method, &params.clone()).await {
                Ok(res) => return Ok(res),
                Err(e) => {
                    tokio::time::sleep(Duration::from_secs(12)).await;
                    error!("Node is not for method {}, retrying: {}", method, e);
                    continue;
                }
            };
        }
    }

    pub async fn get_block_hash(&self, height: u32) -> anyhow::Result<bellscoin::BlockHash> {
        self.request("getblockhash", &[height.into()]).await
    }

    pub async fn best_block_hash(&self) -> anyhow::Result<bellscoin::BlockHash> {
        self.request("getbestblockhash", &[]).await
    }

    pub async fn get_block_info(
        &self,
        hash: &bellscoin::BlockHash,
    ) -> anyhow::Result<bellscoincore_rpc::json::GetBlockResult> {
        self.request("getblock", &[serde_json::to_value(hash)?, 1.into()])
            .await
    }

    pub async fn get_block(&self, hash: &bellscoin::BlockHash) -> anyhow::Result<bellscoin::Block> {
        let hex_result: String = self
            .request("getblock", &[serde_json::to_value(hash)?, 0.into()])
            .await?;
        deserialize_hex(&hex_result)
    }
}

fn deserialize_hex<T: Decodable>(hex: &str) -> anyhow::Result<T> {
    let mut reader = HexIterator::new(hex)?;
    let object = Decodable::consensus_decode(&mut reader)?;
    if reader.read_u8().is_ok() {
        anyhow::bail!("data not consumed entirely when explicitly deserializing")
    } else {
        Ok(object)
    }
}
