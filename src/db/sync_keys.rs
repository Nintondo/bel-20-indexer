//! Namespaced keys in the existing default column family permit binary rollback.
use rocksdb_wrapper::Pebble;
use std::borrow::Cow;

pub struct SyncHeight;
impl Pebble for SyncHeight {
    type Inner = u32;
    const PREFIX: &'static [u8] = b"holdings:block:";
    fn get_bytes(value: &u32) -> Cow<'_, [u8]> {
        Cow::Owned([Self::PREFIX, &value.to_be_bytes()].concat())
    }
    fn from_bytes(bytes: Cow<'_, [u8]>) -> anyhow::Result<u32> {
        Ok(u32::from_be_bytes(
            bytes.strip_prefix(Self::PREFIX).ok_or_else(|| anyhow::anyhow!("Invalid export key"))?.try_into()?,
        ))
    }
}
macro_rules! singleton {
    ($name:ident, $key:literal) => {
        pub struct $name;
        impl Pebble for $name {
            type Inner = ();
            const PREFIX: &'static [u8] = $key;
            fn get_bytes(_: &()) -> Cow<'_, [u8]> {
                Cow::Borrowed(Self::PREFIX)
            }
            fn from_bytes(bytes: Cow<'_, [u8]>) -> anyhow::Result<()> {
                anyhow::ensure!(bytes.as_ref() == Self::PREFIX, "Invalid export metadata key");
                Ok(())
            }
        }
    };
}
singleton!(SyncStateKey, b"holdings:state");
singleton!(IdentityKey, b"holdings:identity");
