//! Native Harness services, durable workflows and local HTTP/desktop runtime.
pub mod error;
pub use error::{HarnessError, Result};
pub mod attachments;
pub mod blackboard;
pub mod brain_settings;
pub mod chat_merge;
pub mod clash;
pub mod context_usage;
pub mod cost_observation;
pub mod creator;
pub mod creator_files;
mod creator_payload;
pub mod creator_runner;
pub mod delivery;
pub mod desktop;
pub mod fleet;
pub mod http;
mod inbox;
pub mod knowledge;
pub mod launch_accounting;
pub mod legacy;
pub mod legacy_msgpack;
pub mod native_diagnostics;
pub mod process;
pub mod projects;
pub mod provider;
pub mod results;
pub mod run_activity;
pub mod sandbox;
pub mod sdd;
pub mod services;
mod session_mounts;
mod session_scheduler;
pub mod sessions;
pub mod setup;
pub mod skills;
pub mod store;
pub mod strict_json;

pub mod memory_draft;

pub mod memory_attempts;

pub mod lifecycle;

pub mod accelerators;
pub mod agent_activity;
pub mod bindings;
pub mod commands;
pub mod facade;
pub mod native;
pub mod prompt;
pub mod updates;

pub use facade::run;

pub use lifecycle::{
    LifecycleOptions, RecoveryReply, RecoveryRequest, RecoveryRunner, RunBinding, RunnerLifecycle,
};
