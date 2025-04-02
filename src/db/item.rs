use super::*;
use nintondo_dogecoin::consensus;

pub trait Pebble {
    type Inner;
    fn get_bytes(v: &Self::Inner) -> Cow<[u8]>;
    fn from_bytes(v: Cow<[u8]>) -> anyhow::Result<Self::Inner>;
}

impl Pebble for () {
    type Inner = Self;
    fn get_bytes(_: &Self::Inner) -> Cow<[u8]> {
        Cow::Borrowed(&[])
    }

    fn from_bytes(_: Cow<[u8]>) -> anyhow::Result<Self::Inner> {
        Ok(())
    }
}

impl Pebble for Cow<'_, [u8]> {
    type Inner = Self;
    fn get_bytes(v: &Self::Inner) -> Cow<[u8]> {
        v.clone()
    }

    fn from_bytes(v: Cow<[u8]>) -> anyhow::Result<Self::Inner> {
        Ok(Cow::Owned(v.into_owned())) //todo: lifetime shit
    }
}

impl Pebble for Vec<u8> {
    type Inner = Self;
    fn get_bytes(v: &Self::Inner) -> Cow<[u8]> {
        Cow::Borrowed(v)
    }

    fn from_bytes(v: Cow<[u8]>) -> anyhow::Result<Self::Inner> {
        Ok(v.into_owned())
    }
}

impl Pebble for String {
    type Inner = Self;
    fn get_bytes(v: &Self::Inner) -> Cow<[u8]> {
        Cow::Borrowed(v.as_bytes())
    }

    fn from_bytes(v: Cow<[u8]>) -> anyhow::Result<Self::Inner> {
        String::from_utf8(v.into_owned()).anyhow()
    }
}

pub struct UsingSerde<T>(PhantomData<T>)
where
    T: serde::Serialize + for<'de> serde::Deserialize<'de>;

impl<T> Pebble for UsingSerde<T>
where
    T: serde::Serialize + for<'de> serde::Deserialize<'de>,
{
    type Inner = T;
    fn get_bytes(v: &T) -> Cow<[u8]> {
        Cow::Owned(postcard::to_allocvec(v).unwrap())
    }

    fn from_bytes(v: Cow<[u8]>) -> anyhow::Result<T> {
        postcard::from_bytes(&v).anyhow()
    }
}

impl<const N: usize> Pebble for [u8; N] {
    type Inner = Self;

    fn get_bytes(v: &Self::Inner) -> Cow<[u8]> {
        Cow::Borrowed(v)
    }

    fn from_bytes(v: Cow<[u8]>) -> anyhow::Result<Self::Inner> {
        Ok(v.into_owned()
            .try_into()
            .expect("Failed to deserlize slice"))
    }
}

pub struct UsingConsensus<T>(PhantomData<T>)
where
    T: consensus::Decodable + consensus::Encodable;

impl<T> Pebble for UsingConsensus<T>
where
    T: consensus::Decodable + consensus::Encodable,
{
    type Inner = T;

    fn get_bytes(v: &Self::Inner) -> Cow<[u8]> {
        let mut result = Vec::new();
        v.consensus_encode(&mut result).unwrap();
        Cow::Owned(result)
    }

    fn from_bytes(v: Cow<[u8]>) -> anyhow::Result<Self::Inner> {
        consensus::Decodable::consensus_decode(&mut std::io::Cursor::new(&v)).anyhow()
    }
}

#[macro_export]
macro_rules! impl_pebble {
    (int $T:ty) => {
        impl $crate::db::Pebble for $T {
            type Inner = Self;

            fn get_bytes(v: &Self::Inner) -> std::borrow::Cow<[u8]> {
                Cow::Owned(v.to_be_bytes().to_vec())
            }

            fn from_bytes(v: Cow<[u8]>) -> anyhow::Result<Self::Inner> {
                Ok(Self::from_be_bytes((&*v).try_into().anyhow()?))
            }
        }
    };

    ($WRAPPER:ty = $INNER:ty) => {
        impl $crate::db::Pebble for $WRAPPER {
            type Inner = Self;

            fn get_bytes(v: &Self::Inner) -> std::borrow::Cow<[u8]> {
                <$INNER>::get_bytes(&v.0)
            }

            fn from_bytes(v: std::borrow::Cow<[u8]>) -> anyhow::Result<Self::Inner> {
                <$INNER>::from_bytes(v).map(Self)
            }
        }
    };

    ($WRAPPER:ty as $INNER:ty) => {
        impl $crate::db::Pebble for $WRAPPER {
            type Inner = Self;

            fn get_bytes(v: &Self::Inner) -> std::borrow::Cow<[u8]> {
                let x = <$INNER>::from(v);
                let x = <$INNER>::get_bytes(&x);
                std::borrow::Cow::Owned(x.into_owned())
            }

            fn from_bytes(v: std::borrow::Cow<[u8]>) -> anyhow::Result<Self::Inner> {
                <$INNER>::from_bytes(v).map(Self::from)
            }
        }
    };
}

impl_pebble!(int i8);
impl_pebble!(int u8);
impl_pebble!(int i16);
impl_pebble!(int u16);
impl_pebble!(int i32);
impl_pebble!(int u32);
impl_pebble!(int i64);
impl_pebble!(int u64);
impl_pebble!(int i128);
impl_pebble!(int u128);
