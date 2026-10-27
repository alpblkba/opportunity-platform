//! Persistence and the ingest pipeline. Everything that touches the database or the network lives here or in `opportunity-adapters`; the rules themselves live in `opportunity-domain`.

pub mod applications;
pub mod auth;
pub mod candidate;
pub mod config;
pub mod db;
pub mod digest;
pub mod documents;
pub mod embeddings;
pub mod ingest;
pub mod mail;
pub mod matching;
pub mod ops;
pub mod product;
pub mod profiles;
pub mod query;
pub mod scheduler;
pub mod snapshot;
pub mod sources;
pub mod worker;
pub mod workflow;

pub use db::{connect, migrate, Db};
