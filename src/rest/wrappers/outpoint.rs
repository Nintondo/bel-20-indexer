use super::*;

use bellscoin::{consensus::Decodable, OutPoint as OutPointInner};

#[repr(transparent)]
#[derive(Clone, Debug, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
pub struct OutPoint(OutPointInner);

impl Display for OutPoint {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        self.0.fmt(f)
    }
}

impl From<OutPointInner> for OutPoint {
    fn from(value: OutPointInner) -> Self {
        Self(value)
    }
}

impl From<Outpoint> for OutPoint {
    fn from(value: Outpoint) -> Self {
        Self(value.into())
    }
}

impl std::ops::Deref for OutPoint {
    type Target = OutPointInner;

    fn deref(&self) -> &Self::Target {
        &self.0
    }
}

impl std::ops::DerefMut for OutPoint {
    fn deref_mut(&mut self) -> &mut Self::Target {
        &mut self.0
    }
}

impl bellscoin::consensus::Encodable for OutPoint {
    fn consensus_encode<W: std::io::Write + ?Sized>(&self, writer: &mut W) -> Result<usize, std::io::Error> {
        self.0.consensus_encode(writer)
    }
}

impl bellscoin::consensus::Decodable for OutPoint {
    fn consensus_decode<R: std::io::Read + ?Sized>(reader: &mut R) -> Result<Self, bellscoin::consensus::encode::Error> {
        Ok(Self(Decodable::consensus_decode(reader)?))
    }
}

impl schemars::JsonSchema for OutPoint {
    fn schema_name() -> std::borrow::Cow<'static, str> {
        std::borrow::Cow::Borrowed("OutPoint")
    }

    fn schema_id() -> std::borrow::Cow<'static, str> {
        concat!(module_path!(), "::OutPoint").into()
    }

    fn json_schema(_: &mut schemars::SchemaGenerator) -> schemars::Schema {
        schemars::json_schema!({
            "type": "string",
            "pattern": "^[0-9a-fA-F]{64}$i\\d+$",
            "description": "SHA-256 transaction hexadecimal hash"
        })
    }
}
