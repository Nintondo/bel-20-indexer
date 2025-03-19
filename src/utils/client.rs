use std::time::Duration;

use dutils::{error::ContextWrapper, wait_token::WaitToken};
use jsonrpc_async::Client;
use nintondo_dogecoin::{
    Block, BlockHash,
    consensus::{Decodable, ReadExt},
};
use serde::de::DeserializeOwned;
use serde_json::{Value, value::RawValue};

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
                    tokio::time::sleep(Duration::from_secs(3)).await;
                    error!("Node is not replying, retrying: {}", e);
                    continue;
                }
            };
        }
    }

    pub async fn get_block_hash(&self, height: u32) -> anyhow::Result<BlockHash> {
        self.request("getblockhash", &[height.into()]).await
    }

    pub async fn best_block_hash(&self) -> anyhow::Result<BlockHash> {
        self.request("getbestblockhash", &[]).await
    }

    // todo add for doge coin
    // pub async fn get_block_info(
    //     &self,
    //     hash: &BlockHash,
    // ) -> anyhow::Result<bellscoincore_rpc::json::GetBlockResult> {
    //     self.request("getblock", &[serde_json::to_value(hash)?, 1.into()])
    //         .await
    // }

    pub async fn get_block(&self, hash: &BlockHash) -> anyhow::Result<Block> {
        let hex_result: String = self
            .request("getblock", &[serde_json::to_value(hash)?, 0.into()])
            .await?;
        deserialize_hex(&hex_result)
    }
}

fn deserialize_hex<T: Decodable>(hex: &str) -> anyhow::Result<T> {
    let mut reader = nintondo_dogecoin::hashes::hex::HexIterator::new(hex)?;
    let object = Decodable::consensus_decode(&mut reader)?;
    if reader.read_u8().is_ok() {
        anyhow::bail!("data not consumed entirely when explicitly deserializing")
    } else {
        Ok(object)
    }
}
