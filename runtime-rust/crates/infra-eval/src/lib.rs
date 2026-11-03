//! Maintainer evaluation controllers. Every process is a native executable.
//! Private labels and partial/unknown observations are never converted into success.
pub mod admission;
pub mod artifact;
pub mod bench;
pub mod cost;
pub mod delivery;
pub mod executable;
pub mod field;
pub mod intake;
pub mod isolation;
pub mod native;
pub mod oracle;
pub mod outcome;
pub mod process;
pub mod reconstruction;
pub mod response;
pub mod rng;
pub mod scoring;
pub mod transcript;
pub mod treatments;
pub use infra_core::{Result, RuntimeError};
pub trait ExcerptScorer {
    fn score(
        &mut self,
        query: &str,
        passages: &[String],
        remaining: std::time::Duration,
    ) -> std::result::Result<Vec<f64>, String>;
}
pub mod cli_contract;
pub mod control;
pub mod sampling;
pub use control::execute;
pub mod bench_driver;
pub mod cancel;
pub mod diagnostic;
pub mod facts;
pub mod freshness;
pub mod graph;
pub mod lifecycle;
pub mod onnx_adapter;
pub mod rehearsal;
pub mod reports;
pub mod throttle_oracle;

pub mod clock;
