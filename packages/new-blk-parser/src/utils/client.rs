use crate::blockchain::parser::BlockchainRead;
use crate::blockchain::proto::block::Block;

use super::*;

/// The different authentication methods for the client.
#[derive(Clone, Debug, Hash, Eq, PartialEq, Ord, PartialOrd)]
pub enum Auth {
    None,
    UserPass(String, String),
    CookieFile(PathBuf),
}

#[derive(thiserror::Error, Debug)]
pub enum Error {
    #[error("Cookie file doesn't exists: {0}")]
    FileDoesntExists(#[from] std::io::Error),
    #[error("Invalid cookie file")]
    InvalidCookieFile,
    #[error("Failed to create client: {0}")]
    Client(#[from] jsonrpc::http::simple_http::Error),
    #[error("Failed to serialize params: {0}")]
    InvalidParams(#[from] serde_json::Error),
    #[error("Failed sending request: {0}")]
    FailedRequest(#[from] jsonrpc::Error),
    #[error("Failed deserialize block: {0}")]
    InvalidBlockHex(#[from] hex::FromHexError),
    #[error("Failed deserialize block: {0}")]
    DeserializeBlock(#[from] anyhow::Error),
    #[error("Token cancelled")]
    Cancelled,
}

type Result<T> = std::result::Result<T, Error>;

impl Auth {
    /// Convert into the arguments that jsonrpc::Client needs.
    pub fn get_user_pass(self) -> Result<(Option<String>, Option<String>)> {
        match self {
            Auth::None => Ok((None, None)),
            Auth::UserPass(u, p) => Ok((Some(u), Some(p))),
            Auth::CookieFile(path) => {
                let line = BufReader::new(File::open(path)?).lines().next().ok_or(Error::InvalidCookieFile)??;
                let colon = line.find(':').ok_or(Error::InvalidCookieFile)?;
                Ok((Some(line[..colon].into()), Some(line[colon + 1..].into())))
            }
        }
    }
}

/// Client for the Bitcoin Core daemon or compatible APIs.
pub struct Client {
    client: jsonrpc::client::Client,
    coin: CoinType,
    token: WaitToken,
}

impl fmt::Debug for Client {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        write!(f, "bitcoincore_rpc::Client({:?})", self.client)
    }
}

impl Client {
    /// Creates a client to a bitcoind JSON-RPC server.
    ///
    /// Can only return [Err] when using cookie authentication.
    pub fn new(url: &str, auth: Auth, coin: CoinType, token: WaitToken) -> Result<Self> {
        let (user, pass) = auth.get_user_pass()?;
        jsonrpc::client::Client::simple_http(url, user, pass)
            .map(|client| Client { client, coin, token })
            .map_err(|e| e.into())
    }

    /// Call an `cmd` rpc with given `args` list
    fn call<T: serde::de::DeserializeOwned>(&self, cmd: &str, args: &[serde_json::Value]) -> Result<T> {
        let raw = serde_json::value::to_raw_value(args).unwrap();

        for _ in 0..10 {
            let req = self.client.build_request(cmd, Some(&*raw));
            let resp = self.client.send_request(req);

            match resp {
                Ok(resp) => match resp.result() {
                    Ok(v) => return Ok(v),
                    Err(err) => {
                        tracing::error!("{:?}", err);
                        std::thread::sleep(Duration::from_secs(1));
                        continue;
                    }
                },
                Err(err) => {
                    tracing::error!("{:?}", err);
                    std::thread::sleep(Duration::from_secs(1));
                    continue;
                }
            }
        }

        self.token.cancel();

        Err(Error::Cancelled)
    }

    pub fn get_block(&self, hash: &sha256d::Hash) -> Result<Block> {
        let block_hex: String = self.call("getblock", &[serde_json::to_value(hash)?, 0.into()])?;
        let block_bytes = hex::decode(block_hex)?;
        let mut block_cursor = std::io::Cursor::new(block_bytes);
        block_cursor.read_block(0, self.coin).map_err(|err| err.into())
    }

    pub fn get_block_info(&self, hash: &sha256d::Hash) -> Result<GetBlockResult> {
        self.call("getblock", &[serde_json::to_value(hash)?, 1.into()])
    }

    /// Get block hash at a given height
    pub fn get_block_hash(&self, height: u64) -> Result<sha256d::Hash> {
        self.call("getblockhash", &[height.into()])
    }

    pub fn get_best_block_hash(&self) -> Result<sha256d::Hash> {
        self.call("getbestblockhash", &[])
    }
}

#[derive(Clone, PartialEq, Debug, serde::Deserialize, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct GetBlockResult {
    pub hash: sha256d::Hash,
    pub confirmations: i32,
    pub size: usize,
    pub strippedsize: Option<usize>,
    pub weight: usize,
    pub height: usize,
    pub version: i32,
    pub time: usize,
    pub mediantime: Option<usize>,
    pub nonce: u32,
    pub bits: String,
    pub difficulty: f64,
    pub previousblockhash: Option<sha256d::Hash>,
    pub nextblockhash: Option<sha256d::Hash>,
}
