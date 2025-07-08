#![allow(unused)]

use anyhow::bail;
use dutils::error::ContextWrapper;
use rocksdb::WriteBatchWithTransaction;
use std::{
    borrow::{Borrow, Cow},
    cell::RefCell,
    marker::PhantomData,
    ops::{Bound, RangeBounds},
    sync::Arc,
};

mod definition;
mod internal;
mod item;
mod storage;
mod utils;

use internal::{DbInfo, TableInfo};
pub use item::{Pebble, UsingConsensus, UsingSerde};
pub use storage::{RocksDB, RocksTable};
use utils::RcUtils;
