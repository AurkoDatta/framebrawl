//! Wire protocol and simulated network links.

mod bits;
mod link;
mod wire;

pub use bits::{BitReader, BitWriter, OutOfData};
pub use link::{ImpairedLink, Impairment};
pub use wire::{
    BLOB_CHUNK, BlobAssembler, ClientMessage, DecodeError, PROTOCOL_VERSION, ServerMessage,
    blob_chunks,
};
