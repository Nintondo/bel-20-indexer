use std::collections::HashSet;

use super::*;

pub trait Pebble {
    const FIXED_SIZE: Option<usize> = None;
    type Inner;
    fn get_bytes<'a>(v: &'a Self::Inner) -> Cow<'a, [u8]>;
    fn from_bytes(v: Cow<[u8]>) -> anyhow::Result<Self::Inner>;
    fn get_bytes_borrowing<R>(v: &Self::Inner, f: impl FnOnce(&[u8]) -> R) -> R {
        (f)(&Self::get_bytes(v))
    }
}

impl Pebble for () {
    const FIXED_SIZE: Option<usize> = Some(0);
    type Inner = Self;
    fn get_bytes<'a>(_: &'a Self::Inner) -> Cow<'a, [u8]> {
        Cow::Borrowed(&[])
    }

    fn from_bytes(_: Cow<[u8]>) -> anyhow::Result<Self::Inner> {
        Ok(())
    }
}

impl Pebble for Cow<'_, [u8]> {
    type Inner = Self;
    fn get_bytes<'a>(v: &'a Self::Inner) -> Cow<'a, [u8]> {
        v.clone()
    }

    fn from_bytes(v: Cow<[u8]>) -> anyhow::Result<Self::Inner> {
        Ok(Cow::Owned(v.into_owned()))
    }

    fn get_bytes_borrowing<R>(v: &Self::Inner, f: impl FnOnce(&[u8]) -> R) -> R {
        (f)(v)
    }
}

impl Pebble for String {
    type Inner = Self;
    fn get_bytes<'a>(v: &'a Self::Inner) -> Cow<'a, [u8]> {
        Cow::Borrowed(v.as_bytes())
    }

    fn from_bytes(v: Cow<[u8]>) -> anyhow::Result<Self::Inner> {
        String::from_utf8(v.into_owned()).anyhow()
    }
}

thread_local! {
    static POSTCARD_BUFFER: RefCell<Vec<Vec<u8>>> = Default::default();
}

/// Provides rocksdb support for types that implements serde::Serialize and serde::Deserialize (via derive). <br/>
/// UsingSerde internally uses postcard. Certain serde features, for example untagged enums are not supported and will panic at runtime. <br/>
/// If you can derive bitcode::Encode and bitcode::Decode, UsingBitcode<T> is preferred.
pub struct UsingSerde<T>(PhantomData<T>)
where
    T: serde::Serialize + for<'de> serde::Deserialize<'de>;

impl<T> Pebble for UsingSerde<T>
where
    T: serde::Serialize + for<'de> serde::Deserialize<'de>,
{
    type Inner = T;
    fn get_bytes<'a>(v: &'a T) -> Cow<'a, [u8]> {
        Cow::Owned(postcard::to_allocvec(v).unwrap())
    }

    fn from_bytes(v: Cow<[u8]>) -> anyhow::Result<T> {
        postcard::from_bytes(&v).anyhow()
    }

    fn get_bytes_borrowing<R>(v: &Self::Inner, f: impl FnOnce(&[u8]) -> R) -> R {
        let mut buf = POSTCARD_BUFFER.with_borrow_mut(|buf| buf.pop()).unwrap_or_default();
        buf.clear();
        let buf = postcard::to_extend(v, buf).unwrap();
        let x = (f)(&buf);
        POSTCARD_BUFFER.with_borrow_mut(|x| x.push(buf));
        x
    }
}

impl<T: Pebble<Inner = T>> Pebble for Vec<T> {
    type Inner = Self;

    fn from_bytes(v: Cow<[u8]>) -> anyhow::Result<Self::Inner> {
        let size = T::FIXED_SIZE.expect("FIXED_SIZE is required in order to use Vec<Pebble>");
        v.chunks(size).map(|x| T::from_bytes(Cow::Borrowed(x))).collect()
    }

    fn get_bytes<'a>(v: &'a Self::Inner) -> Cow<'a, [u8]> {
        Cow::Owned(v.iter().flat_map(|x| T::get_bytes(x).into_owned()).collect())
    }
}

impl<T: Pebble<Inner = T> + std::hash::Hash + Eq> Pebble for HashSet<T> {
    type Inner = Self;

    fn from_bytes(v: Cow<[u8]>) -> anyhow::Result<Self::Inner> {
        let size = T::FIXED_SIZE.expect("FIXED_SIZE is required in order to use Vec<Pebble>");
        v.chunks(size).map(|x| T::from_bytes(Cow::Borrowed(x))).collect()
    }

    fn get_bytes<'a>(v: &'a Self::Inner) -> Cow<'a, [u8]> {
        Cow::Owned(v.iter().flat_map(|x| T::get_bytes(x).into_owned()).collect())
    }
}

impl<const N: usize> Pebble for [u8; N] {
    type Inner = Self;

    fn get_bytes<'a>(v: &'a Self::Inner) -> Cow<'a, [u8]> {
        Cow::Borrowed(v)
    }

    fn from_bytes(v: Cow<[u8]>) -> anyhow::Result<Self::Inner> {
        Ok(v.into_owned().try_into().expect("Failed to deserlize slice"))
    }
}

pub struct UsingConsensus<T>(PhantomData<T>)
where
    T: bellscoin::consensus::Decodable + bellscoin::consensus::Encodable;

impl<T> Pebble for UsingConsensus<T>
where
    T: bellscoin::consensus::Decodable + bellscoin::consensus::Encodable,
{
    type Inner = T;

    fn get_bytes<'a>(v: &'a Self::Inner) -> Cow<'a, [u8]> {
        let mut result = Vec::new();
        v.consensus_encode(&mut result).unwrap();
        Cow::Owned(result)
    }

    fn from_bytes(v: Cow<[u8]>) -> anyhow::Result<Self::Inner> {
        bellscoin::consensus::Decodable::consensus_decode(&mut std::io::Cursor::new(&v)).anyhow()
    }
}

/// Wrapper for a type with align of 1
#[derive(Copy, Clone)]
#[repr(C, packed)]
struct Packed<T>(pub T);

#[macro_export]
macro_rules! impl_pebble {
    (int $T:ty) => {
        impl $crate::Pebble for $T {
            const FIXED_SIZE: Option<usize> = Some(std::mem::size_of::<Packed<$T>>());
            type Inner = Self;

            fn get_bytes<'a>(v: &'a Self::Inner) -> std::borrow::Cow<'a, [u8]> {
                Cow::Owned(v.to_be_bytes().to_vec())
            }

            fn from_bytes(v: Cow<[u8]>) -> anyhow::Result<Self::Inner> {
                Ok(Self::from_be_bytes((&*v).try_into().anyhow()?))
            }
        }
    };

    ($WRAPPER:ty = $INNER:ty) => {
        impl $crate::Pebble for $WRAPPER {
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
        impl $crate::Pebble for $WRAPPER {
            const FIXED_SIZE: Option<usize> = $INNER::FIXED_SIZE;
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

/// K0 must have fixed size
pub trait MultiPebble {
    type K0: Pebble;
    type K1: Pebble;

    fn get_bytes_k0_into(src: &<Self::K0 as Pebble>::Inner, dest: &mut Vec<u8>) {
        let x = <Self::K0 as Pebble>::get_bytes(src);
        if x.len() > dest.len() {
            *dest = x.to_vec();
        } else {
            dest.clear();
            dest.extend_from_slice(&x);
        }
    }
    fn get_bytes_k1_into(src: &<Self::K1 as Pebble>::Inner, dest: &mut Vec<u8>) {
        let x = <Self::K1 as Pebble>::get_bytes(src);
        dest.extend_from_slice(&x);
    }
}

impl<K0: Pebble, K1: Pebble> MultiPebble for (K0, K1) {
    type K0 = K0;
    type K1 = K1;
}

impl<K0: Pebble, K1: Pebble> Pebble for (K0, K1) {
    type Inner = (K0::Inner, K1::Inner);

    fn get_bytes<'a>(v: &'a Self::Inner) -> Cow<'a, [u8]> {
        assert!(K0::FIXED_SIZE.is_some(), "First key in MultiPebble must have fixed size");

        let mut buf = K0::get_bytes(&v.0).to_vec();
        buf.extend_from_slice(&K1::get_bytes(&v.1));
        Cow::Owned(buf)
    }

    fn from_bytes(v: Cow<[u8]>) -> anyhow::Result<Self::Inner> {
        assert!(K0::FIXED_SIZE.is_some(), "First key in MultiPebble must have fixed size");

        let k0_s = K0::FIXED_SIZE.unwrap();
        let k0 = K0::from_bytes(Cow::Borrowed(&v[..k0_s]))?;
        let k1 = K1::from_bytes(Cow::Borrowed(&v[k0_s..]))?;

        Ok((k0, k1))
    }
}
