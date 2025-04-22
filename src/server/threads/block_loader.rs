use kanal::Sender;
use parking_lot::Mutex;
use std::path::PathBuf;

use super::*;

#[derive(Clone)]
pub struct BlockRpcLoader {
    pub server: Arc<Server>,
    pub tx: Sender<(u32, bellscoin::Block, bellscoin::hashes::sha256d::Hash)>,
}

impl Handler for BlockRpcLoader {
    async fn run(&mut self) -> anyhow::Result<()> {
        loop {
            let current_block_height = self.server.db.last_block.get(()).unwrap_or(0);
            let current_block_hash = self.server.db.block_hashes.get(current_block_height);
            let mut next_block_height = self
                .server
                .db
                .last_block
                .get(())
                .map(|x| x + 1)
                .unwrap_or(1);

            let tip_hash = self.server.client.best_block_hash().await?;
            let tip_height = self.server.client.get_block_info(&tip_hash).await?.height as u32;

            if tip_height == current_block_height {
                tokio::time::sleep(Duration::from_secs(5)).await;
                continue;
            }

            warn!("Blocks to sync: {}", tip_height - current_block_height);

            while next_block_height < tip_height && !self.server.token.is_cancelled() {
                let hash = self.server.client.get_block_hash(next_block_height).await?;
                let block = self.server.client.get_block(&hash).await?;

                let Some(current_block_hash) = current_block_hash else {
                    self.tx
                        .send((next_block_height, block, *hash.as_raw_hash()))?;
                    next_block_height += 1;
                    continue;
                };

                if current_block_hash == block.header.prev_blockhash {
                    continue;
                }

                let mut blocks = vec![(next_block_height, block)];
                let mut prev_height = next_block_height - 1;
                loop {
                    let db_prev_hash = self
                        .server
                        .db
                        .block_hashes
                        .get(prev_height - 1)
                        .expect("Block must exist");

                    let prev_block_hash = self.server.client.get_block_hash(prev_height).await?;

                    let prev_block = self.server.client.get_block(&prev_block_hash).await?;
                    if db_prev_hash == prev_block.header.prev_blockhash {
                        for (height, block) in blocks.into_iter().rev() {
                            let hash = block.block_hash();
                            self.tx.send((height, block, *hash.as_raw_hash()))?;
                        }
                        next_block_height += 1;
                        break;
                    }

                    blocks.push((prev_height, prev_block));
                    prev_height -= 1;
                }
            }
        }
    }
}

#[derive(Clone)]
pub struct BlockBlkLoader {
    pub magic: [u8; 4],
    pub blk_dir: PathBuf,
    pub from_block: Option<u32>,
    pub to_block: Option<u32>,
}

impl BlockBlkLoader {
    fn main_loop(
        blk_dir: PathBuf,
        magic: [u8; 4],
        from: Option<u32>,
        to: Option<u32>,
        tx: Sender<(u32, bellscoin::Block, bellscoin::hashes::sha256d::Hash)>,
    ) -> anyhow::Result<()> {
        let auth = bellscoincore_rpc::Auth::UserPass(USER.to_string(), PASS.to_string());
        let client = bellscoincore_rpc::Client::new(&URL, auth)?;

        let parser: utils::blk::Parser<bellscoin::Block, bellscoincore_rpc::Client> =
            utils::blk::Parser::new(blk_dir, client, magic);

        parser.parse(tx, from, to);

        Ok(())
    }

    pub fn run(
        this: Arc<Mutex<Self>>,
        tx: Sender<(u32, bellscoin::Block, bellscoin::hashes::sha256d::Hash)>,
    ) {
        std::thread::spawn(move || loop {
            let lock = this.lock();
            let blk_dir = lock.blk_dir.clone();
            let magic = lock.magic;
            let from = lock.from_block;
            let to = lock.to_block;
            drop(lock);

            let Err(e) = Self::main_loop(blk_dir, magic, from, to, tx.clone()) else {
                break;
            };

            error!("Blk loader got error: {e}");
            std::thread::sleep(Duration::from_secs(5));
        });
    }
}
