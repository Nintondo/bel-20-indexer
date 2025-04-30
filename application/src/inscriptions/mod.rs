use crate::inscriptions::envelope::{ParsedEnvelope, RawEnvelope};
pub use crate::inscriptions::structs::Location;
use crate::inscriptions::tag::Tag;
pub use crate::inscriptions::utils::ScriptToAddr;
use std::sync::Arc;

pub const PROTOCOL_ID: &[u8; 3] = b"ord";

pub mod envelope;
mod media;
pub mod searcher;
pub mod structs;
pub mod tag;
pub mod utils;
