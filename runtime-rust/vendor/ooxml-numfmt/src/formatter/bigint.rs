//! BigInt formatting for arbitrary precision integers.
//!
//! This module handles formatting of large integers that exceed f64's safe integer range (±2^53).
//! For values within the safe range, the regular f64 formatting path is used.
//! For values outside the safe range, string-based arithmetic is used to preserve precision.

use crate::ast::FormatPart;
use crate::compile::{SectionKind, SectionPlan};
use crate::error::FormatError;
use crate::options::FormatOptions;
use num_bigint::BigInt;

use super::render::RenderPart;

/// The maximum safe integer value for f64 (2^53 - 1)
pub const MAX_SAFE_INTEGER: i64 = 9_007_199_254_740_991;
/// The minimum safe integer value for f64 (-(2^53 - 1))
pub const MIN_SAFE_INTEGER: i64 = -9_007_199_254_740_991;

/// Check if a BigInt is within the safe f64 integer range.
pub fn is_safe_integer(n: &BigInt) -> bool {
    let min_safe = BigInt::from(MIN_SAFE_INTEGER);
    let max_safe = BigInt::from(MAX_SAFE_INTEGER);
    n >= &min_safe && n <= &max_safe
}

/// Evaluate a large BigInt through a compiled plan without resolving layout.
pub(super) fn evaluate_bigint(
    value: &BigInt,
    plan: &SectionPlan,
    opts: &FormatOptions,
) -> Result<Vec<RenderPart>, FormatError> {
    match plan.kind {
        SectionKind::Number => evaluate_compiled_number(value, plan, opts),
        SectionKind::General => Ok(super::evaluate_operations(plan, |_, part| match part {
            FormatPart::GeneralNumber => Some(value.to_string()),
            FormatPart::Locale(locale) => locale.currency.clone(),
            FormatPart::Percent => Some("%".to_string()),
            _ => None,
        })),
        SectionKind::Literal | SectionKind::Text => {
            let mut numeric_text = (plan.kind == SectionKind::Text).then(|| value.to_string());
            Ok(super::evaluate_operations(plan, |_, part| match part {
                FormatPart::TextPlaceholder => numeric_text.take(),
                FormatPart::Locale(locale) => locale.currency.clone(),
                FormatPart::Percent => Some("%".to_string()),
                _ => None,
            }))
        }
        SectionKind::DateTime => Err(FormatError::TypeMismatch {
            expected: "numeric format",
            got: "date format with BigInt value",
        }),
        SectionKind::Scientific | SectionKind::Fraction => Err(FormatError::TypeMismatch {
            expected: "standard numeric format",
            got: "precision-dependent BigInt format",
        }),
    }
}

/// Apply compiled percentage and scaling semantics using exact BigInt arithmetic.
fn evaluate_compiled_number(
    value: &BigInt,
    plan: &SectionPlan,
    opts: &FormatOptions,
) -> Result<Vec<RenderPart>, FormatError> {
    let spec = plan.number.as_ref().ok_or(FormatError::TypeMismatch {
        expected: "compiled number format",
        got: "non-number section",
    })?;
    let mut adjusted = if value.sign() == num_bigint::Sign::Minus {
        -value.clone()
    } else {
        value.clone()
    };

    if spec.percent_count > 0 {
        adjusted *= BigInt::from(100_u8).pow(spec.percent_count as u32);
    }
    if spec.thousands_scale > 0 {
        adjusted /= BigInt::from(1000_u16).pow(spec.thousands_scale as u32);
    }

    super::number::evaluate_integer_digits(&adjusted.to_string(), plan, opts)
}

/// Fallback formatting for BigInt values.
/// Converts to string representation.
pub fn fallback_format_bigint(value: &BigInt) -> String {
    value.to_string()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_is_safe_integer() {
        assert!(is_safe_integer(&BigInt::from(0)));
        assert!(is_safe_integer(&BigInt::from(1000)));
        assert!(is_safe_integer(&BigInt::from(-1000)));
        assert!(is_safe_integer(&BigInt::from(MAX_SAFE_INTEGER)));
        assert!(is_safe_integer(&BigInt::from(MIN_SAFE_INTEGER)));

        // Just outside safe range
        let above_max = BigInt::from(MAX_SAFE_INTEGER) + 1;
        let below_min = BigInt::from(MIN_SAFE_INTEGER) - 1;
        assert!(!is_safe_integer(&above_max));
        assert!(!is_safe_integer(&below_min));

        // Large values
        assert!(!is_safe_integer(
            &BigInt::parse_bytes(b"123456822333333000", 10).unwrap()
        ));
    }

    #[test]
    fn test_fallback_format_bigint() {
        let big = BigInt::parse_bytes(b"123456822333333000", 10).unwrap();
        assert_eq!(fallback_format_bigint(&big), "123456822333333000");
    }

    #[test]
    fn evaluates_large_values_with_the_compiled_number_spec() {
        let value = BigInt::parse_bytes(b"123456789012345678", 10).unwrap();
        let format = crate::NumberFormat::parse("$#,##0.00").unwrap();

        assert_eq!(
            format
                .try_format_bigint(&value, &FormatOptions::default())
                .unwrap(),
            "$123,456,789,012,345,678.00"
        );
    }

    #[test]
    fn repeated_text_placeholders_render_bigint_once() {
        let value = BigInt::parse_bytes(b"123456789012345678", 10).unwrap();
        let format = crate::NumberFormat::parse("@@").unwrap();

        assert_eq!(
            format.format_bigint(&value, &FormatOptions::default()),
            "123456789012345678"
        );
    }

    #[test]
    fn renders_bigint_before_decimal_without_integer_placeholder() {
        let value = BigInt::from(42);
        let format = crate::NumberFormat::parse(".00").unwrap();

        assert_eq!(
            format.format_bigint(&value, &FormatOptions::default()),
            "42.00"
        );
    }

    #[test]
    fn preserves_fill_inside_large_integer_placeholders() {
        let value = BigInt::parse_bytes(b"123456789012345678", 10).unwrap();
        let format = crate::NumberFormat::parse("0*x0").unwrap();
        let plan = &format.compiled.sections[0];
        let parts = evaluate_bigint(&value, plan, &FormatOptions::default()).unwrap();

        assert_eq!(
            super::super::render::resolve_layout(&parts, 3).unwrap(),
            "12345678901234567xxx8"
        );
    }

    #[test]
    fn applies_bigint_percentage_and_scaling_exactly() {
        let value = BigInt::parse_bytes(b"123456789012345678", 10).unwrap();
        let percent = crate::NumberFormat::parse("0%").unwrap();
        let scaled = crate::NumberFormat::parse("0,,").unwrap();

        assert_eq!(
            percent.format_bigint(&value, &FormatOptions::default()),
            "12345678901234567800%"
        );
        assert_eq!(
            scaled.format_bigint(&value, &FormatOptions::default()),
            "123456789012"
        );
    }
}
