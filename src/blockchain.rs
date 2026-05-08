use std::str::FromStr;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Blockchain {
    Dogecoin,
    /// DogecoinEV (DEV) chain
    DogecoinEv,
    Bellscoin,
    Pepecoin,
    Litecoin,
}

#[derive(Debug, thiserror::Error)]
pub enum BlockchainParseError {
    #[error("Unknown blockchain")]
    UnknownBlockchain,
}

impl FromStr for Blockchain {
    type Err = BlockchainParseError;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        match s.to_ascii_lowercase().as_str() {
            "dogecoin" | "doge" => Ok(Blockchain::Dogecoin),
            // DogecoinEV (DEV) aliases
            "dogecoinev" | "dev" | "dev20" | "dev-20" | "doge-ev" => Ok(Blockchain::DogecoinEv),
            "bellscoin" | "bells" => Ok(Blockchain::Bellscoin),
            "pepecoin" | "pepe" => Ok(Blockchain::Pepecoin),
            "litecoin" => Ok(Blockchain::Litecoin),
            _ => Err(BlockchainParseError::UnknownBlockchain),
        }
    }
}
