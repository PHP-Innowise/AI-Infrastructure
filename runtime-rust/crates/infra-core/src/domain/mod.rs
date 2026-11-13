pub mod clock;
mod config;
pub mod json;
mod privacy;
mod record;
mod schema;
mod sources;

pub use privacy::policy_pattern_matches;

pub use config::RuntimeConfig;
pub use privacy::{
    guard_shared_text, indexed_secret_policy_reason, mask_secrets, sanitize_automatic_query,
    secret_label, secret_policy_fingerprint, sensitive_label, source_path_problem,
};
pub use record::{
    AUTHORITIES, PHASES, PRIVACY, RECORD_TYPES, initial_state, is_terminal, lifecycle_states,
    normalize_phase, parse_markdown_record, record_attestation, render_current_state,
    render_markdown_record, rstrip, transition_allowed, unique_strings, utc_now, uuid4,
    validate_handoff, validate_record,
};
pub use schema::{
    is_uuid4, python_date, valid_datetime, validate_schema_file, validate_schema_value,
};
pub use sources::{
    citation_problem, digestible_sources, source_changes, source_fingerprints, sources_are_fresh,
};

/// Lowercase hexadecimal for raw bytes, independent of digest-array traits.
pub fn hex_lower(bytes: impl AsRef<[u8]>) -> String {
    const DIGITS: &[u8; 16] = b"0123456789abcdef";
    let bytes = bytes.as_ref();
    let mut out = String::with_capacity(bytes.len() * 2);
    for &byte in bytes {
        out.push(DIGITS[(byte >> 4) as usize] as char);
        out.push(DIGITS[(byte & 15) as usize] as char);
    }
    out
}
