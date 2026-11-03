//! Native repository tooling. No command invokes a Python implementation.
pub mod artifacts;
pub mod budget;
pub mod common;
pub mod file_modes;
pub mod links;
pub mod manifests;
pub mod mirrors;
pub mod policy;
pub mod sync;
pub use infra_core::{Result, RuntimeError};
pub mod dispatch;
pub mod installer;
pub use dispatch::{execute, report_exit_code};
pub mod attach;
pub mod changelog;
pub mod checker;
pub mod collect;
pub mod kit;
pub mod mcp_config;
pub mod php;
pub mod process;
pub mod registry;
pub mod routes;
pub mod stabilization;
pub mod temp;

pub mod release;
pub mod ui_build;

pub mod wasm_assets;
