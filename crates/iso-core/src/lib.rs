//! Isoline shared types. This crate does no IO; every other crate builds on it.
//!
//! The contracts in here are frozen after phase 0 (see CONTRACTS.md). Change them
//! only through the lead.

pub mod cache;
pub mod facts;
pub mod flow;
pub mod ids;
pub mod ipc;
pub mod jobs;
pub mod lang;

pub use facts::*;
pub use flow::*;
pub use ids::*;
pub use lang::*;

/// Hash arbitrary bytes to a stable 64-bit value (BLAKE3, first 8 bytes, little endian).
pub fn hash64(bytes: &[u8]) -> u64 {
    let h = blake3::hash(bytes);
    u64::from_le_bytes(h.as_bytes()[..8].try_into().expect("8 bytes"))
}

/// Full content hash as lowercase hex (BLAKE3, 32 bytes).
pub fn content_hash(bytes: &[u8]) -> String {
    blake3::hash(bytes).to_hex().to_string()
}
