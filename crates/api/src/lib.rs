//! HTTP surface. Bound to loopback by default; nothing here is designed to face a public network.

pub mod access;
pub mod applications;
pub mod auth;
pub mod error;
pub mod personalization;
pub mod routes;

pub use access::{AccessMode, Identity};
pub use routes::{router, AppState};
