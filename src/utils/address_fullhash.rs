use super::*;

#[derive(Default)]
pub struct AddressesFullHash(HashMap<FullHash, String>);

impl AddressesFullHash {
    pub fn new(v: HashMap<FullHash, String>) -> Self {
        Self(v)
    }

    pub fn get(&self, hash: &FullHash) -> String {
        fullhash_to_address_str(hash, self.0.get(hash).cloned())
    }
}

impl From<HashMap<FullHash, String>> for AddressesFullHash {
    fn from(value: HashMap<FullHash, String>) -> Self {
        Self(value)
    }
}

pub fn fullhash_to_address_str(hash: &FullHash, value: Option<String>) -> String {
    if let Some(value) = value {
        return value;
    }

    if hash.is_op_return_hash() {
        OP_RETURN_ADDRESS.to_string()
    } else {
        NON_STANDARD_ADDRESS.to_string()
    }
}
