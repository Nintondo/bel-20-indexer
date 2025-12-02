use super::*;

mod client;
mod range;

pub use client::{Auth, Client};
pub use range::BlockHeightRange;

pub fn arr_to_hex(data: &[u8]) -> String {
    data.iter().fold(String::with_capacity(data.len() * 2), |mut output, b| {
        write!(output, "{b:02x?}").unwrap();
        output
    })
}

/// Calculates merkle root for the whole block
/// See: https://en.bitcoin.it/wiki/Protocol_documentation#Merkle_Trees
pub fn merkle_root(hashes: Vec<sha256d::Hash>) -> sha256d::Hash {
    let mut hashes = hashes;

    while hashes.len() > 1 {
        // Calculates double sha hash for each pair. If len is odd, last value is ignored.
        let mut new_hashes = hashes
            .chunks(2)
            .filter(|c| c.len() == 2)
            .map(|c| sha256d::Hash::hash(&[c[0], c[1]].concat()))
            .collect::<Vec<sha256d::Hash>>();

        // If the length is odd, take the last hash twice
        if hashes.len() % 2 == 1 {
            let last_hash = hashes.last().unwrap();
            new_hashes.push(sha256d::Hash::hash(&[&last_hash[..], &last_hash[..]].concat()));
        }
        hashes = new_hashes;
    }
    *hashes.first().expect("unable to calculate merkle root on empty hashes")
}
