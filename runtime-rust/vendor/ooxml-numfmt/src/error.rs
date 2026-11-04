//! Error types for parsing and formatting.

use thiserror::Error;

/// Errors that can occur when parsing or programmatically constructing a format code.
#[derive(Debug, Clone, PartialEq, Error)]
pub enum ParseError {
    #[error("unexpected token at position {position}: found '{found}'")]
    UnexpectedToken { position: usize, found: char },

    #[error("unterminated bracket at position {position}")]
    UnterminatedBracket { position: usize },

    #[error("invalid condition at position {position}: {reason}")]
    InvalidCondition { position: usize, reason: String },

    #[error("invalid locale code at position {position}")]
    InvalidLocaleCode { position: usize },

    #[error("too many sections (maximum 4 allowed)")]
    TooManySections,

    #[error("empty format code")]
    EmptyFormat,

    #[error("unsupported subsecond precision: {places} (maximum 9)")]
    UnsupportedPrecision { places: usize },

    #[error("invalid format ID: {0} is not a recognized built-in format")]
    InvalidFormatId(u32),

    /// A programmatically supplied fraction section violates fraction invariants.
    #[error("invalid fraction in section {section_index}: {reason}")]
    InvalidFraction {
        /// Zero-based index of the invalid section.
        section_index: usize,
        /// Description of the invalid fraction syntax.
        reason: &'static str,
    },
}

/// Errors that can occur when formatting a value.
#[derive(Debug, Clone, PartialEq, Error)]
pub enum FormatError {
    #[error("type mismatch: expected {expected}, got {got}")]
    TypeMismatch {
        expected: &'static str,
        got: &'static str,
    },

    #[error("date out of range: serial number {serial}")]
    DateOutOfRange { serial: f64 },

    #[error("invalid serial number: {value}")]
    InvalidSerialNumber { value: f64 },

    #[error("unsupported subsecond precision: {places} (maximum 9)")]
    UnsupportedPrecision { places: u8 },

    /// The requested layout cannot fit in a Rust string.
    #[error("formatted output is too large for fill count {fill_count}")]
    OutputTooLarge {
        /// Fill repetition count that caused the output-size failure.
        fill_count: usize,
    },
}
