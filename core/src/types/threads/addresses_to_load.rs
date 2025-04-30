use std::collections::HashSet;
use bellscoin::ScriptBuf;

pub struct AddressesToLoad {
    pub height: u32,
    pub addresses: HashSet<ScriptBuf>,
}
