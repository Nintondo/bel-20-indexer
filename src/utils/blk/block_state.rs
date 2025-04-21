use std::io::Cursor;

use super::InnerBlockHash;

pub enum BlockState<T: InnerBlockHash> {
    Raw(Vec<u8>),
    Decoded(T),
}

impl<T: InnerBlockHash> BlockState<T> {
    pub fn decode(&mut self) {
        let bytes = match self {
            BlockState::Raw(bytes) => bytes,
            _ => unreachable!(),
        };

        let mut cursor = Cursor::new(bytes);

        let block = T::consensus_decode(&mut cursor).unwrap();

        *self = BlockState::Decoded(block);
    }
}
