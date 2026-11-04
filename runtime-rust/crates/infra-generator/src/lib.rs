//! Native Infrastructure-Creator deterministic gates and staged publication.
//! All operations execute Rust code; the generator never delegates to Python.
pub mod assets;
pub mod commands;
pub mod common;
pub mod content;
pub mod coverage;
pub mod flows;
pub mod generated;
pub mod gitignore;
pub mod hook_policy;
pub mod mutations;
pub mod ownership;
pub mod publication;
pub mod quality;
pub mod reviews;
pub mod routes;
pub mod seed;
pub use generated::{GeneratedOptions, NativeGateServices, NativeOutput};
pub use routes::{GeneratorOutput, execute};
