use std::collections::HashMap;

use crate::types::full_hash::FullHash;

pub trait AddressesLoader {
    fn load_addresses(
        &self,
        keys: impl IntoIterator<Item = FullHash>,
        height: u32,
    ) -> impl std::future::Future<Output = anyhow::Result<HashMap<FullHash, String>>>;
}
