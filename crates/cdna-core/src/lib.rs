//! The sole authority and persistence boundary of C-DNA.
//! No HTTP port is opened by default. External agents can propose, not approve.
pub mod app;
pub mod auth;
pub mod backup;
pub mod diagnostics;
pub mod domain;
pub mod features;
pub mod imports;
pub mod jobs;
pub mod keys;
pub mod policy;
pub mod questions;
pub mod store;
pub mod transport;
pub mod trees;
pub use app::Runtime;
pub use domain::{Error, Result};
#[cfg(test)]
mod tests;
