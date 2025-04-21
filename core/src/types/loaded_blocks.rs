use std::collections::VecDeque;

use super::{structs::BlockHeader, token_history::TokenHistoryData};

#[derive(Default)]
pub struct LoadedBlocks {
    pub from_block_number: u32,
    pub to_block_number: u32,
    pub blocks: VecDeque<Blocks>,
}

impl LoadedBlocks {
    pub fn take_blocks(&mut self) -> Option<Blocks> {
        self.blocks.pop_front()
    }
}

pub struct Blocks {
    pub from: BlockHeader,
    pub to: BlockHeader,
    pub blocks: Vec<electrs_client::Update<TokenHistoryData>>,
}
