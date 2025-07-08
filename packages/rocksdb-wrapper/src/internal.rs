use super::*;

pub(super) const TABLE_INFO_CF: &str = "__TABLE_INFO_CF";
pub(super) const DB_INFO_CF: &str = "__DB_INFO_CF";

#[derive(serde::Serialize, serde::Deserialize, Debug, PartialEq, Eq)]
pub struct TableInfo {
    pub key_ty_name: String,
    pub val_ty_name: String,
    pub key_struct_size: usize,
    pub val_struct_size: usize,
}

impl TableInfo {
    pub fn new<K: Pebble, V: Pebble>() -> Self {
        let key_ty_name = std::any::type_name::<K>().to_string();
        let val_ty_name = std::any::type_name::<V>().to_string();
        let key_struct_size = std::mem::size_of::<K::Inner>();
        let val_struct_size = std::mem::size_of::<V::Inner>();

        TableInfo {
            key_ty_name,
            val_ty_name,
            key_struct_size,
            val_struct_size,
        }
    }
}

#[derive(serde::Serialize, serde::Deserialize, Debug, PartialEq, Eq)]
pub struct DbInfo {
    pub version: usize,
}
