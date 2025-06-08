use super::*;

#[derive(Default)]
pub struct AddressesFullHash(HashMap<FullHash, String>);

impl AddressesFullHash {
    pub fn new(v: HashMap<FullHash, String>) -> Self {
        Self(v)
    }

    pub fn get(&self, hash: &FullHash) -> String {
        if hash.is_op_return_hash() {
            return OP_RETURN_ADDRESS.to_string();
        }

        self.0
            .get(hash)
            .cloned()
            .unwrap_or(NON_STANDARD_ADDRESS.to_string())
    }

    pub fn iter(&'_ self) -> impl Iterator<Item = String> + use<'_> {
        self.0.values().cloned()
    }
}

impl From<HashMap<FullHash, String>> for AddressesFullHash {
    fn from(value: HashMap<FullHash, String>) -> Self {
        Self(value)
    }
}
