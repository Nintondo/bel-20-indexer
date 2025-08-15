use super::*;

use bellscoin::{consensus::Decodable, Txid as TxidInner};

#[repr(transparent)]
#[derive(Clone, Debug, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
pub struct Txid(pub TxidInner);

impl Display for Txid {
    fn fmt(&self, f: &mut Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}", self.0)
    }
}

impl std::ops::Deref for Txid {
    type Target = TxidInner;

    fn deref(&self) -> &Self::Target {
        &self.0
    }
}

impl std::ops::DerefMut for Txid {
    fn deref_mut(&mut self) -> &mut Self::Target {
        &mut self.0
    }
}

impl schemars::JsonSchema for Txid {
    fn schema_name() -> std::borrow::Cow<'static, str> {
        std::borrow::Cow::Borrowed("Txid")
    }

    fn schema_id() -> std::borrow::Cow<'static, str> {
        concat!(module_path!(), "::Txid").into()
    }

    fn json_schema(_: &mut schemars::SchemaGenerator) -> schemars::Schema {
        schemars::json_schema!({
            "type": "string",
            "pattern": "^[0-9a-fA-F]{64}$",
            "description": "SHA-256 transaction hexadecimal hash"
        })
    }
}

impl std::str::FromStr for Txid {
    type Err = <TxidInner as std::str::FromStr>::Err;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        TxidInner::from_str(s).map(Txid)
    }
}

impl bellscoin::consensus::Encodable for Txid {
    fn consensus_encode<W: std::io::Write + ?Sized>(&self, writer: &mut W) -> Result<usize, std::io::Error> {
        self.0.consensus_encode(writer)
    }
}

impl bellscoin::consensus::Decodable for Txid {
    fn consensus_decode<R: std::io::Read + ?Sized>(reader: &mut R) -> Result<Self, bellscoin::consensus::encode::Error> {
        Ok(Self(Decodable::consensus_decode(reader)?))
    }
}

impl std::fmt::LowerHex for Txid {
    fn fmt(&self, f: &mut Formatter<'_>) -> std::fmt::Result {
        <TxidInner as std::fmt::LowerHex>::fmt(self, f)
    }
}

impl std::borrow::Borrow<[u8]> for Txid {
    fn borrow(&self) -> &[u8] {
        self.0.borrow()
    }
}

impl std::ops::Index<std::ops::Range<usize>> for Txid {
    type Output = <TxidInner as std::ops::Index<std::ops::Range<usize>>>::Output;

    fn index(&self, index: std::ops::Range<usize>) -> &Self::Output {
        self.0.index(index)
    }
}

impl std::ops::Index<std::ops::RangeFrom<usize>> for Txid {
    type Output = <TxidInner as std::ops::Index<std::ops::RangeFrom<usize>>>::Output;

    fn index(&self, index: std::ops::RangeFrom<usize>) -> &Self::Output {
        self.0.index(index)
    }
}

impl std::ops::Index<std::ops::RangeTo<usize>> for Txid {
    type Output = <TxidInner as std::ops::Index<std::ops::RangeTo<usize>>>::Output;

    fn index(&self, index: std::ops::RangeTo<usize>) -> &Self::Output {
        self.0.index(index)
    }
}

impl std::ops::Index<std::ops::RangeFull> for Txid {
    type Output = <TxidInner as std::ops::Index<std::ops::RangeFull>>::Output;

    fn index(&self, index: std::ops::RangeFull) -> &Self::Output {
        self.0.index(index)
    }
}

impl std::ops::Index<usize> for Txid {
    type Output = <TxidInner as std::ops::Index<usize>>::Output;

    fn index(&self, index: usize) -> &Self::Output {
        self.0.index(index)
    }
}

impl Hash for Txid {
    type Engine = <TxidInner as Hash>::Engine;

    type Bytes = <TxidInner as Hash>::Bytes;

    fn engine() -> Self::Engine {
        <TxidInner as Hash>::engine()
    }

    fn from_engine(e: Self::Engine) -> Self {
        Self(TxidInner::from_engine(e))
    }

    const LEN: usize = <TxidInner as Hash>::LEN;
    const DISPLAY_BACKWARD: bool = <TxidInner as Hash>::DISPLAY_BACKWARD;

    #[inline]
    fn from_slice(sl: &[u8]) -> Result<Self, bitcoin_hashes::Error> {
        TxidInner::from_slice(sl).map(Self)
    }

    #[inline]
    fn to_byte_array(self) -> Self::Bytes {
        TxidInner::to_byte_array(self.0)
    }

    #[inline]
    fn as_byte_array(&self) -> &Self::Bytes {
        TxidInner::as_byte_array(&self.0)
    }

    #[inline]
    fn from_byte_array(bytes: Self::Bytes) -> Self {
        Self(TxidInner::from_byte_array(bytes))
    }

    #[inline]
    fn all_zeros() -> Self {
        Self(TxidInner::all_zeros())
    }
}

impl From<TxidInner> for Txid {
    fn from(value: TxidInner) -> Self {
        Self(value)
    }
}
