use super::*;

pub trait RcUtils: Sized {
    fn arc(self) -> Arc<Self> {
        Arc::new(self)
    }
}

impl<T: Sized> RcUtils for T {}

#[macro_export]
macro_rules! generate_db_code {
    ($($name:ident: $key_type:ty => $value_type:ty),* $(,)?) => {
        pub struct DB {
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
                            stringify!($name).to_uppercase().as_str(),
                        )*
                    ],
                );

                Self {
                    $(
                        $name: db.table(stringify!($name).to_uppercase().as_str()),
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
