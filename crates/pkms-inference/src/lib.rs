//! Protocol and isolated model adapters. No workspace database writes.
pub mod chat_format;
#[cfg(feature = "embedding")]
pub mod embedding;
#[cfg(feature = "ollama")]
pub mod ollama;
pub mod profile;
pub mod protocol;
#[cfg(feature = "remote")]
pub mod remote;
#[cfg(feature = "vector")]
pub mod vector;
#[cfg(feature = "vector-flat")]
pub mod vector_flat;

pub mod remote_chunks;
#[cfg(feature = "remote")]
pub mod remote_embeddings;
