use futures::FutureExt;

use super::*;
use std::collections::VecDeque;

#[derive(Default)]
pub struct LoadedBlocks {
    pub from_block_number: u32,
    pub to_block_number: u32,
    pub blocks: VecDeque<Blocks>,
}

pub struct Blocks {
    pub from: BlockHeader,
    pub to: BlockHeader,
    pub blocks: Vec<electrs_client::Update<inscriptions::types::TokenHistoryData>>,
}

impl LoadedBlocks {
    pub fn take_blocks(&mut self) -> Option<Blocks> {
        self.blocks.pop_front()
    }
}

#[derive(Clone)]
pub struct BlocksLoader {
    pub storage: Arc<tokio::sync::Mutex<LoadedBlocks>>,
    pub client: Arc<electrs_client::Client<inscriptions::types::TokenHistoryData>>,
}

impl BlocksLoader {
    async fn fetch_block_metas(
        &self,
        block_numbers: Vec<u32>,
    ) -> Result<Vec<electrs_client::BlockMeta>, electrs_client::ClientError> {
        let futures: Vec<_> = block_numbers
            .into_iter()
            .map(|block_number| self.client.get_electrs_block_meta(block_number))
            .collect();

        let result: Result<Vec<_>, electrs_client::ClientError> =
            join_all(futures).await.into_iter().collect();

        result
    }

    async fn fetch_blocks(
        client: &electrs_client::Client<inscriptions::types::TokenHistoryData>,
        blocks: Vec<electrs_client::BlockMeta>,
    ) -> Result<Vec<Blocks>, electrs_client::ClientError> {
        let features = blocks.into_iter().map(|from| async move {
            client
                .fetch_updates::<inscriptions::types::InscriptionsTokenHistory>(&[from])
                .map(|result| {
                    result.map(|blocks| Blocks {
                        from: blocks
                            .first()
                            .map(|block| match block {
                                electrs_client::Update::AddBlock { block, .. } => BlockHeader {
                                    number: block.block_info.height,
                                    hash: block.block_info.block_hash.into(),
                                    prev_hash: block.block_info.prev_block_hash.into(),
                                },
                                _ => unimplemented!(),
                            })
                            .expect("Must exist"),
                        to: blocks
                            .iter()
                            .next_back()
                            .map(|block| match block {
                                electrs_client::Update::AddBlock { block, .. } => BlockHeader {
                                    number: block.block_info.height,
                                    hash: block.block_info.block_hash.into(),
                                    prev_hash: block.block_info.prev_block_hash.into(),
                                },
                                _ => unimplemented!(),
                            })
                            .expect("Must exist"),
                        blocks,
                    })
                })
                .await
        });

        let result: Result<Vec<_>, electrs_client::ClientError> =
            join_all(features).await.into_iter().collect();

        result
    }

    fn generate_blocks_batch_sequence(
        mut from: u32,
        to: u32,
        batch_size: u32,
        mut batch_count: u32,
    ) -> Vec<u32> {
        let mut result = Vec::with_capacity(batch_count as _);
        while from <= to && batch_count != 0 {
            result.push(from);
            from += batch_size;
            batch_count -= 1;
        }
        result
    }
}

impl Handler for BlocksLoader {
    async fn run(&mut self) -> anyhow::Result<()> {
        let block_numbers = {
            let lock = self.storage.lock().await;
            if lock.from_block_number >= lock.to_block_number {
                return Ok(());
            }

            if lock.blocks.len() > 2 {
                return Ok(());
            }

            if lock.from_block_number == 0 {
                vec![0]
            } else {
                Self::generate_blocks_batch_sequence(
                    lock.from_block_number,
                    lock.to_block_number,
                    self.client.config.limit.unwrap_or(1000),
                    5,
                )
            }
        };

        let now = Instant::now();
        let block_metas = self.fetch_block_metas(block_numbers).await.anyhow()?;

        let blocks = Self::fetch_blocks(&self.client, block_metas)
            .await
            .anyhow()?;

        if blocks.is_empty() {
            return Ok(());
        }

        let from = blocks
            .first()
            .map(|x| x.from.number)
            .expect("Validate above");
        let to = blocks.last().map(|x| x.to.number).expect("Validate above");

        info!(
            "BlocksLoader fetch batch of blocks {} from #{} to #{}, {}s",
            blocks.iter().map(|x| x.blocks.len()).sum::<usize>(),
            from,
            to,
            now.elapsed().as_secs_f32()
        );

        let mut storage = self.storage.lock().await;
        storage.from_block_number = to;
        blocks
            .into_iter()
            .for_each(|blocks| storage.blocks.push_back(blocks));

        Ok(())
    }
}
