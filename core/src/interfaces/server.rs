use std::sync::Arc;

use bellscoin::hashes::sha256;
use dutils::wait_token::WaitToken;

use crate::{
    db::tables::DB,
    types::{
        holders::Holders,
        server::{RawServerEvent, ServerEvent},
        structs::{AddressTokenId, HistoryValue},
    },
};
use std::collections::HashMap;

use crate::types::full_hash::FullHash;

pub trait DBPort {
    fn get_db(&self) -> Arc<DB>;
}

pub trait HoldersPort {
    fn get_holders(&self) -> Arc<Holders>;
}

pub trait LastIndexedAddressPort {
    fn get_last_indexed_address_height(&self) -> Arc<tokio::sync::RwLock<u32>>;
}

pub trait EventSenderPort {
    fn get_event_sender(&self) -> tokio::sync::broadcast::Sender<ServerEvent>;
    fn get_raw_event_sender(&self) -> kanal::Sender<RawServerEvent>;
}

pub trait TokenPort {
    fn get_token(&self) -> WaitToken;
}

pub trait ClientPort<T> {
    fn get_client(&self) -> T;
}

pub trait AddressesLoader {
    fn load_addresses(
        &self,
        keys: impl IntoIterator<Item = FullHash>,
        height: u32,
    ) -> impl std::future::Future<Output = anyhow::Result<HashMap<FullHash, String>>>;
}

pub trait HistoryHashGenerator {
    fn generate_history_hash(
        &self,
        prev_history_hash: sha256::Hash,
        history: &[(AddressTokenId, HistoryValue)],
        addresses: &HashMap<FullHash, String>,
    ) -> anyhow::Result<sha256::Hash>;
}
