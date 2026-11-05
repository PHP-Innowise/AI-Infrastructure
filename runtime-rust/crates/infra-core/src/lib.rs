//! Native foundation and governed Project Brain records.
//!
//! Authoritative records retain the edition's Markdown/JSON shapes. This
//! crate performs filesystem and Git operations itself and never runs Python.

pub mod advisory;
pub mod application;
pub mod bank;
pub mod brain;
pub mod capsule;
pub mod checkpoint;
pub mod continuity;
pub mod cursor_rule;
pub mod domain;
pub mod error;
pub mod excerpts;
pub mod fs;
pub mod git;
pub mod handoff;
pub mod history;
pub mod index;
pub mod mcp;
pub mod native;
pub mod observability;
pub mod python_json;
pub mod query;
pub mod queue;
pub mod receipts;
pub mod result_store;
pub mod results;
pub mod retrieval;
pub mod review;
pub mod serving;
pub mod source_links;
pub mod storage;
pub mod strict_json;
mod unicode_fold;
pub mod workspace;

pub use application::{CommandOutput, MemoryService, RefreshOutcome, RefreshRequest};
pub use bank::BankService;
pub use brain::{BrainRepository, CreateRecord, MessageInput, RecordImage, UpdateRecord};
pub use capsule::{Capsule, DeliveryProfile};
pub use domain::{
    RuntimeConfig, parse_markdown_record, render_markdown_record, source_fingerprints,
    validate_record, validate_schema_value,
};
pub use error::{ErrorKind, Result, RuntimeError};
pub use handoff::{HandoffService, SaveHandoff};
pub use index::{IndexReport, refresh_index};
pub use mcp::McpServer;
pub use native::{Host, NativeDrain, NativeSave, NativeService, NativeStop};
pub use query::{SearchItem, SearchRequest, search};
pub use results::{
    Learning, NormalizedResult, PrepareResult, RecordResult, ResultService, Revision,
};
pub use retrieval::{RetrievalRequest, retrieve, retrieve_with_preparer};
pub use review::ReviewService;
pub use serving::{TaskResolver, TaskService};
pub use storage::{
    Binding, CompleteTask, Episode, EpisodeInput, OpenOptions, StartTask, Store, TurnDelta,
    UpdateTask, WorkingTask,
};
pub use workspace::{Roots, RuntimeContext};

pub mod chat_continuity;
pub mod dispatch;
pub use chat_continuity::ChatContinuity;
pub use dispatch::{DispatchService, capsule_report, validate_capsule_text};

pub(crate) mod queue_process;
pub mod security;

pub mod health;

pub mod source_policy;

pub mod source_hints;

pub mod parity;
pub use parity::{CrossParity, ParityRequest, ParityService};
