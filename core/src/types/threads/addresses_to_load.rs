use std::collections::HashSet;
use std::sync::Arc;
use bellscoin::ScriptBuf;
use dutils::wait_token::WaitToken;

pub struct AddressesToLoad {
    pub height: u32,
    pub addresses: HashSet<ScriptBuf>,
}
