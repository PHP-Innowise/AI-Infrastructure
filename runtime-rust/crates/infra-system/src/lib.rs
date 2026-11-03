//! Native system passports, bounded discovery, reviewed plans and sequential execution.
pub mod backend;
pub mod catalog;
pub mod common;
pub mod discovery;
pub mod editor;
pub mod execution;
pub mod manager;
pub mod native;
pub mod planner;
pub mod routes;
pub mod runner;
pub mod workers;
pub use backend::{Project, SessionBridge};
pub use manager::SystemManager;
pub use routes::{execute, execute_async, initialize};
pub use workers::{
    AgentSignal, run_discovery_request_with_adapter, run_system_request_with_adapter,
};
pub use workers::{run_discovery_request, run_system_request};
