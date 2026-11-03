//! Compiled fraction preparation and field evaluation.

use crate::ast::FormatPart;
use crate::compile::{FractionDenominatorSpec, FractionSpec, NumberPlaceholder, SectionPlan};
use crate::error::FormatError;
use crate::formatter::number::format_simple_with_placeholders;
use crate::options::FormatOptions;

use super::render::RenderPart;

/// Shared value-dependent state prepared once for all fraction operations.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct PreparedFraction {
    /// Whole-number portion of a mixed fraction.
    integer: u64,
    /// Prepared numerator after approximation and carry.
    numerator: u32,
    /// Prepared variable or fixed denominator.
    denominator: u32,
    /// SSF semantic width used to pad variable fraction components.
    padding_width: usize,
}

/// Evaluate compiled fraction fields without resolving surrounding layout.
pub(super) fn evaluate_fraction(
    value: f64,
    plan: &SectionPlan,
    _opts: &FormatOptions,
) -> Result<Vec<RenderPart>, FormatError> {
    let spec = plan.fraction.as_ref().ok_or(FormatError::TypeMismatch {
        expected: "compiled fraction format",
        got: "non-fraction section",
    })?;
    let prepared = prepare_fraction(value, spec)?;
    let fields = prepare_fraction_fields(plan.operations.len(), spec, prepared);

    Ok(super::evaluate_operations(
        plan,
        |operation_index, part| match part {
            FormatPart::Fraction(_) => fields[operation_index].clone(),
            FormatPart::Locale(locale) => locale.currency.clone(),
            FormatPart::Percent => Some("%".to_string()),
            _ => None,
        },
    ))
}

/// Prepare fraction arithmetic once for all semantic source fields.
fn prepare_fraction(value: f64, spec: &FractionSpec) -> Result<PreparedFraction, FormatError> {
    let abs_value = value.abs();
    if !abs_value.is_finite() || abs_value >= u64::MAX as f64 {
        return Err(FormatError::InvalidSerialNumber { value });
    }
    let mut integer = abs_value.trunc() as u64;
    let is_mixed = !spec.integer_placeholders.is_empty();
    let padding_width = match &spec.denominator {
        FractionDenominatorSpec::Variable { placeholders } => {
            if is_mixed {
                spec.numerator_placeholders
                    .len()
                    .max(placeholders.len())
                    .min(7)
            } else {
                placeholders.len().min(7)
            }
        }
        FractionDenominatorSpec::Fixed { .. } => 0,
    };
    let approximation_value = if is_mixed {
        abs_value.fract()
    } else {
        abs_value
    };
    let (mut numerator, denominator) = match &spec.denominator {
        FractionDenominatorSpec::Variable { .. } => {
            let max_denominator = 10_u32.pow(padding_width as u32) - 1;
            find_best_fraction(approximation_value, max_denominator)?
        }
        FractionDenominatorSpec::Fixed { value, .. } => {
            let rounded = (approximation_value * f64::from(*value)).round();
            if !rounded.is_finite() || rounded > f64::from(u32::MAX) {
                return Err(FormatError::InvalidSerialNumber {
                    value: approximation_value,
                });
            }
            (rounded as u32, *value)
        }
    };

    if is_mixed && denominator > 0 && numerator >= denominator {
        integer = integer.saturating_add(u64::from(numerator / denominator));
        numerator %= denominator;
    }

    Ok(PreparedFraction {
        integer,
        numerator,
        denominator,
        padding_width,
    })
}

/// Prepare operation-indexed fraction text while leaving layout operations unresolved.
fn prepare_fraction_fields(
    operation_count: usize,
    spec: &FractionSpec,
    prepared: PreparedFraction,
) -> Vec<Option<String>> {
    let mut fields = vec![None; operation_count];
    let is_mixed = !spec.integer_placeholders.is_empty();

    if is_mixed {
        if prepared.integer > 0 || prepared.numerator == 0 {
            assign_formatted_component(prepared.integer, &spec.integer_placeholders, &mut fields);
        } else {
            assign_empty_placeholders(&spec.integer_placeholders, &mut fields);
        }
    }

    if is_mixed && prepared.numerator == 0 {
        let numerator_width = match &spec.denominator {
            FractionDenominatorSpec::Variable { .. } => prepared.padding_width,
            FractionDenominatorSpec::Fixed { .. } => spec.numerator_placeholders.len(),
        };
        assign_right_aligned(
            &" ".repeat(numerator_width),
            &spec.numerator_placeholders,
            &mut fields,
        );
        fields[spec.slash_index] = Some(" ".to_string());
        match &spec.denominator {
            FractionDenominatorSpec::Variable { placeholders } => assign_left_aligned(
                &" ".repeat(prepared.padding_width),
                placeholders,
                &mut fields,
            ),
            FractionDenominatorSpec::Fixed { digits, .. } => {
                for digit in digits {
                    fields[digit.operation_index] = Some(" ".to_string());
                }
            }
        }
        return fields;
    }

    if is_mixed {
        let width = match &spec.denominator {
            FractionDenominatorSpec::Variable { .. } => prepared.padding_width,
            FractionDenominatorSpec::Fixed { .. } => spec.numerator_placeholders.len(),
        };
        assign_right_aligned(
            &format!("{:>width$}", prepared.numerator),
            &spec.numerator_placeholders,
            &mut fields,
        );
    } else {
        assign_formatted_component(
            u64::from(prepared.numerator),
            &spec.numerator_placeholders,
            &mut fields,
        );
    }
    fields[spec.slash_index] = Some("/".to_string());

    match &spec.denominator {
        FractionDenominatorSpec::Variable { placeholders } => {
            let denominator = format!(
                "{:<width$}",
                prepared.denominator,
                width = prepared.padding_width
            );
            assign_left_aligned(&denominator, placeholders, &mut fields);
        }
        FractionDenominatorSpec::Fixed { digits, .. } => {
            for digit in digits {
                fields[digit.operation_index] = Some(digit.digit.to_string());
            }
        }
    }

    fields
}

/// Format one placeholder-based integer component and map it back to operations.
fn assign_formatted_component(
    value: u64,
    placeholders: &[NumberPlaceholder],
    fields: &mut [Option<String>],
) {
    let syntax: Vec<_> = placeholders.iter().map(|field| field.placeholder).collect();
    let text = format_simple_with_placeholders(value, &syntax);
    assign_right_aligned(&text, placeholders, fields);
}

/// Assign each placeholder's empty representation at its own source operation.
fn assign_empty_placeholders(placeholders: &[NumberPlaceholder], fields: &mut [Option<String>]) {
    for field in placeholders {
        if let Some(character) = field.placeholder.empty_char() {
            fields[field.operation_index] = Some(character.to_string());
        }
    }
}

/// Assign text right-to-left, attaching leading overflow to the first field.
fn assign_right_aligned(
    text: &str,
    placeholders: &[NumberPlaceholder],
    fields: &mut [Option<String>],
) {
    if placeholders.is_empty() {
        return;
    }

    let characters: Vec<char> = text.chars().collect();
    let overflow = characters.len().saturating_sub(placeholders.len());
    if overflow > 0 {
        fields[placeholders[0].operation_index] = Some(characters[..overflow].iter().collect());
    }
    let placeholder_start = placeholders.len().saturating_sub(characters.len());
    let character_start = characters.len().saturating_sub(placeholders.len());
    for (placeholder, character) in placeholders[placeholder_start..]
        .iter()
        .zip(&characters[character_start..])
    {
        fields[placeholder.operation_index]
            .get_or_insert_with(String::new)
            .push(*character);
    }
}

/// Assign text left-to-right, attaching trailing overflow to the final field.
fn assign_left_aligned(
    text: &str,
    placeholders: &[NumberPlaceholder],
    fields: &mut [Option<String>],
) {
    let Some(last) = placeholders.last() else {
        return;
    };

    let mut characters = text.chars();
    for placeholder in placeholders {
        let Some(character) = characters.next() else {
            return;
        };
        fields[placeholder.operation_index] = Some(character.to_string());
    }
    fields[last.operation_index]
        .get_or_insert_with(String::new)
        .extend(characters);
}

/// Find the best fraction approximation for a decimal value.
/// Uses continued fractions algorithm for best rational approximation.
fn find_best_fraction(value: f64, max_denom: u32) -> Result<(u32, u32), FormatError> {
    if !value.is_finite() || !(0.0..=f64::from(u32::MAX)).contains(&value) {
        return Err(FormatError::InvalidSerialNumber { value });
    }
    if value == 0.0 || max_denom == 0 {
        return Ok((0, 1));
    }

    // Handle values very close to 0
    if value.abs() < 1e-10 {
        return Ok((0, 1));
    }

    // Use continued fractions algorithm
    let mut x = value;
    let mut a = x.floor();
    let mut h = [a as i64, 1];
    let mut k = [1_i64, 0];

    let mut n = 0;
    while n < 20 {
        // Limit iterations
        if (x - a).abs() < 1e-10 {
            break;
        }

        x = 1.0 / (x - a);
        a = x.floor();

        let factor = a as i64;
        let k_next = factor
            .checked_mul(k[0])
            .and_then(|n| n.checked_add(k[1]))
            .ok_or(FormatError::InvalidSerialNumber { value })?;
        // Reject an inadmissible denominator before numerator multiplication.
        if k_next > i64::from(max_denom) {
            break;
        }
        let h_next = factor
            .checked_mul(h[0])
            .and_then(|n| n.checked_add(h[1]))
            .ok_or(FormatError::InvalidSerialNumber { value })?;
        if h_next > i64::from(u32::MAX) {
            return Err(FormatError::InvalidSerialNumber { value });
        }

        h[1] = h[0];
        h[0] = h_next;
        k[1] = k[0];
        k[0] = k_next;

        n += 1;
    }

    // Ensure we don't exceed max denominator
    if k[0] > max_denom as i64 {
        // Fall back to simple rounding
        let denom = max_denom.min(10);
        let num = (value * denom as f64).round() as u32;
        return Ok((num, denom));
    }

    Ok((h[0].max(0) as u32, k[0].max(1) as u32))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_find_best_fraction() {
        assert_eq!(find_best_fraction(0.2, 9).unwrap(), (1, 5));
        assert_eq!(find_best_fraction(0.333333, 9).unwrap(), (1, 3));
        assert_eq!(find_best_fraction(0.666666, 9).unwrap(), (2, 3));
    }

    #[test]
    fn preserves_layout_around_fraction_fields() {
        let format = crate::NumberFormat::parse("# ?/?*x").unwrap();
        let plan = &format.compiled.sections[0];
        let parts = evaluate_fraction(1.5, plan, &FormatOptions::default()).unwrap();

        assert_eq!(
            super::super::render::resolve_layout(&parts, 2).unwrap(),
            "1 1/2xx"
        );
    }

    #[test]
    fn test_prepares_mixed_fraction_carry_once() {
        let format = crate::NumberFormat::parse("# ?/2").unwrap();
        let spec = format.compiled.sections[0].fraction.as_ref().unwrap();

        assert_eq!(
            prepare_fraction(1.75, spec).unwrap(),
            PreparedFraction {
                integer: 2,
                numerator: 0,
                denominator: 2,
                padding_width: 0,
            }
        );
    }

    #[test]
    fn test_assigns_fixed_denominator_digits_around_fill() {
        let format = crate::NumberFormat::parse("# ?/1*x6").unwrap();
        let plan = &format.compiled.sections[0];

        assert_eq!(
            evaluate_fraction(1.2, plan, &FormatOptions::default()).unwrap(),
            vec![
                RenderPart::Text("1 3/1".to_string()),
                RenderPart::Fill('x'),
                RenderPart::Text("6".to_string()),
            ]
        );
    }

    #[test]
    fn test_assigns_variable_denominator_padding_without_crossing_fill() {
        let format = crate::NumberFormat::parse("# ??/?*x?").unwrap();
        let plan = &format.compiled.sections[0];

        assert_eq!(
            evaluate_fraction(1.5, plan, &FormatOptions::default()).unwrap(),
            vec![
                RenderPart::Text("1  1/2".to_string()),
                RenderPart::Fill('x'),
                RenderPart::Text(" ".to_string()),
            ]
        );
    }

    #[test]
    fn test_applies_zero_hash_and_question_integer_placeholders() {
        let format = crate::NumberFormat::parse("0#? ?/?").unwrap();
        let plan = &format.compiled.sections[0];

        assert_eq!(
            evaluate_fraction(0.5, plan, &FormatOptions::default()).unwrap(),
            vec![RenderPart::Text("0  1/2".to_string())]
        );
    }
}
