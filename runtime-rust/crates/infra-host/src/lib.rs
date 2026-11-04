//! Shared concrete native host, CLI and isolation implementations.
pub mod capture;
pub mod error;
pub mod process;
pub mod provider;
pub mod sandbox;
pub mod strict_json;
pub use error::HarnessError;
pub use error::{HarnessError as HostError, Result};
pub mod store {
    pub fn hexid() -> String {
        uuid::Uuid::new_v4().simple().to_string()
    }
    pub fn sha(bytes: &[u8]) -> String {
        infra_core::query::sha256(bytes)
    }
}
pub mod activity;
pub mod command;
pub mod delegation;
pub mod model_catalog;
pub mod rollout;
pub mod util;
