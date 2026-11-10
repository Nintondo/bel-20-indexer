use super::*;

pub trait RcUtils: Sized {
    fn arc(self) -> Arc<Self> {
        Arc::new(self)
    }
}

impl<T: Sized> RcUtils for T {}

#[macro_export]
macro_rules! generate_db_code {
    (@cf $name:ident, $cf:literal) => { $cf };
    (@cf $name:ident) => { stringify!($name).to_uppercase().as_str() };
    ($($name:ident: $key_type:ty => $value_type:ty $(=> $cf:literal)?),* $(,)?) => {
        pub struct DB {
            pub storage: $crate::RocksDB,
            $(
                pub $name: super::RocksTable<$key_type, $value_type>,
            )*
        }

        impl DB {
            pub fn open(path: &str) -> Self {
                let db = RocksDB::open_db(
                    path,
                    [
                        $(
                            $crate::generate_db_code!(@cf $name $(, $cf)?),
                        )*
                    ],
                );

                Self {
                    storage: db.clone(),
                    $(
                        $name: db.table($crate::generate_db_code!(@cf $name $(, $cf)?)),
                    )*
                }
            }

            pub fn flush_all(&self) {
                $(
                    self.$name.flush();
                )*
            }
        }

        $(
            const _: fn() = || {
                fn assert_pebble<T: $crate::Pebble>() {}
                assert_pebble::<$key_type>();
                assert_pebble::<$value_type>();
            };
        )*
    };
}
