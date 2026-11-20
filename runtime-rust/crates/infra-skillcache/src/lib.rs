//! SkillCache: a repeated use of a skill runs as a short compiled plan.
//!
//! A cold session with an unchanged skill is observed; a compiler agent turns
//! the request, the skill and the observed trace into a capsule; three gates
//! qualify it without calling a model; a warm session then gets the capsule's
//! short description and a plan executor instead of the skill's text.
//!
//! This crate holds what needs no Harness: the capsule's shape and its strict
//! reader ([`capsule`]), the structural gate ([`gate1`]), packages on disk
//! ([`package`]), content identities ([`fingerprint`]) and the compiler's
//! instructions ([`compile_rules`]).
pub mod capsule;
pub mod compile_rules;
pub mod fingerprint;
pub mod gate1;
pub mod package;

pub use capsule::{Examples, Form, SCHEMA_VERSION, SkillCapsule, Step, Violation};
