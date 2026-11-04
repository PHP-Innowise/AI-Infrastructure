//! Compiled standard-number and scientific field evaluation.

use crate::ast::{DigitPlaceholder, FormatPart};
use crate::compile::{NumberPlaceholder, NumberSpec, ScientificSpec, SectionPlan};
use crate::error::FormatError;
use crate::options::FormatOptions;

use super::render::RenderPart;

/// Evaluate a compiled standard-number plan without resolving layout directives.
pub(super) fn evaluate_number(
    value: f64,
    plan: &SectionPlan,
    opts: &FormatOptions,
) -> Result<Vec<RenderPart>, FormatError> {
    let spec = plan.number.as_ref().ok_or(FormatError::TypeMismatch {
        expected: "compiled number format",
        got: "non-number section",
    })?;
    let fields = prepare_number_fields(value, plan.operations.len(), spec, opts);

    Ok(super::evaluate_operations(
        plan,
        |operation_index, part| match part {
            FormatPart::Digit(_) | FormatPart::DecimalPoint | FormatPart::ThousandsSeparator => {
                fields[operation_index].clone()
            }
            FormatPart::Percent => Some("%".to_string()),
            FormatPart::Locale(locale) => locale.currency.clone(),
            _ => None,
        },
    ))
}

/// Evaluate a compiled scientific plan while retaining ordered layout anchors.
pub(super) fn evaluate_scientific(
    value: f64,
    plan: &SectionPlan,
    _opts: &FormatOptions,
) -> Result<Vec<RenderPart>, FormatError> {
    let spec = plan.scientific.as_ref().ok_or(FormatError::TypeMismatch {
        expected: "compiled scientific format",
        got: "non-scientific section",
    })?;
    let fields = prepare_scientific_fields(value, plan.operations.len(), spec);
    let mut output = super::evaluate_operations(plan, |operation_index, part| match part {
        FormatPart::Digit(_) | FormatPart::DecimalPoint | FormatPart::Scientific { .. } => {
            fields[operation_index].clone()
        }
        FormatPart::Percent => Some("%".to_string()),
        FormatPart::Locale(locale) => locale.currency.clone(),
        _ => None,
    });

    if value < 0.0 {
        output.insert(0, RenderPart::Text("-".to_string()));
    }

    Ok(output)
}

/// Prepare mantissa and exponent text for each scientific field operation.
fn prepare_scientific_fields(
    value: f64,
    operation_count: usize,
    spec: &ScientificSpec,
) -> Vec<Option<String>> {
    let mut adjusted = value.abs();
    for _ in 0..spec.percent_count {
        adjusted *= 100.0;
    }

    let integer_places = spec.mantissa_integer.len().max(1);
    let decimal_places = spec.mantissa_decimal.len();
    let (mantissa, mut exponent) = if adjusted == 0.0 {
        (0.0, 0)
    } else {
        let base_exponent = adjusted.log10().floor() as i32;
        // Multiple integer placeholders use engineering-style exponent groups.
        let exponent = if integer_places > 1 {
            ((base_exponent as f64) / (integer_places as f64)).floor() as i32
                * integer_places as i32
        } else {
            base_exponent
        };
        (adjusted / 10_f64.powi(exponent), exponent)
    };
    let mut mantissa_text = if decimal_places > 0 {
        format!("{mantissa:.decimal_places$}")
    } else {
        format!("{mantissa:.0}")
    };
    let rounded_integer = mantissa_text
        .split_once('.')
        .map_or(mantissa_text.as_str(), |(integer, _)| integer);
    if rounded_integer.bytes().all(|byte| byte.is_ascii_digit())
        && rounded_integer.len() > integer_places
    {
        // Rounding can carry the mantissa into the next exponent group. Move
        // it back into range before assigning digits to source placeholders.
        exponent += integer_places as i32;
        mantissa_text = if decimal_places > 0 {
            format!("{:.decimal_places$}", 1.0)
        } else {
            "1".to_string()
        };
    }
    let (mantissa_integer, mantissa_decimal) = mantissa_text
        .split_once('.')
        .unwrap_or((mantissa_text.as_str(), ""));
    let exponent_width = if adjusted == 0.0 || spec.exponent_digits.len() >= 2 {
        2
    } else {
        1
    };
    let exponent_text = format!("{:0>width$}", exponent.abs(), width = exponent_width);
    let exponent_marker = if spec.upper { 'E' } else { 'e' };
    let exponent_sign = if exponent < 0 {
        "-"
    } else if spec.show_plus {
        "+"
    } else {
        ""
    };
    let mut fields = vec![None; operation_count];

    assign_right_aligned(mantissa_integer, &spec.mantissa_integer, &mut fields);
    assign_left_aligned(mantissa_decimal, &spec.mantissa_decimal, &mut fields);
    assign_right_aligned(&exponent_text, &spec.exponent_digits, &mut fields);
    if let Some(operation_index) = spec.decimal_point_index {
        if decimal_places > 0 {
            fields[operation_index]
                .get_or_insert_with(String::new)
                .push('.');
        }
    }
    fields[spec.exponent_marker_index]
        .get_or_insert_with(String::new)
        .push_str(&format!("{exponent_marker}{exponent_sign}"));

    fields
}

/// Assign text right-to-left with placeholder padding and overflow kept leftmost.
fn assign_right_aligned(
    text: &str,
    placeholders: &[NumberPlaceholder],
    fields: &mut [Option<String>],
) {
    if placeholders.is_empty() {
        return;
    }

    let characters: Vec<char> = text.chars().collect();
    let missing = placeholders.len().saturating_sub(characters.len());
    for placeholder in &placeholders[..missing] {
        if let Some(character) = placeholder.placeholder.empty_char() {
            fields[placeholder.operation_index] = Some(character.to_string());
        }
    }

    let extra = characters.len().saturating_sub(placeholders.len());
    if extra > 0 {
        fields[placeholders[0].operation_index] = Some(characters[..extra].iter().collect());
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

/// Assign text left-to-right across placeholder operations.
fn assign_left_aligned(
    text: &str,
    placeholders: &[NumberPlaceholder],
    fields: &mut [Option<String>],
) {
    for (placeholder, character) in placeholders.iter().zip(text.chars()) {
        fields[placeholder.operation_index] = Some(character.to_string());
    }
}

/// Prepare text for each numeric semantic operation in one value-specific pass.
fn prepare_number_fields(
    value: f64,
    operation_count: usize,
    spec: &NumberSpec,
    opts: &FormatOptions,
) -> Vec<Option<String>> {
    let mut fields = vec![None; operation_count];

    // SSF uses a separate integer path. Preserve it for exact safe integers so
    // conversion and decimal rounding cannot lose precision.
    const MAX_SAFE_INTEGER: f64 = 9_007_199_254_740_992.0;
    if value.fract() == 0.0
        && value.abs() < MAX_SAFE_INTEGER
        && spec.decimal_placeholders.is_empty()
    {
        let mut adjusted = (value as i64).unsigned_abs();
        for _ in 0..spec.percent_count {
            adjusted = adjusted.saturating_mul(100);
        }
        for _ in 0..spec.thousands_scale {
            adjusted /= 1000;
        }
        prepare_integer_fields(adjusted, spec, opts, &mut fields);
        return fields;
    }

    let mut adjusted = value.abs();
    for _ in 0..spec.percent_count {
        adjusted *= 100.0;
    }
    for _ in 0..spec.thousands_scale {
        adjusted /= 1000.0;
    }

    // f64 has roughly 15 significant decimal digits, so larger requested
    // precision cannot improve the rounded semantic value.
    let effective_decimal_places = spec.decimal_places().min(15);
    let multiplier = 10_f64.powi(effective_decimal_places as i32);
    let rounded = (adjusted * multiplier).round() / multiplier;
    prepare_integer_fields(rounded.trunc() as u64, spec, opts, &mut fields);
    prepare_decimal_fields(rounded.fract(), spec, &mut fields);

    if let Some(operation_index) = spec.decimal_point_index {
        fields[operation_index]
            .get_or_insert_with(String::new)
            .push(opts.locale.decimal_separator);
    }

    fields
}

/// Map integer digits, padding, and grouping to their source placeholders.
fn prepare_integer_fields(
    value: u64,
    spec: &NumberSpec,
    opts: &FormatOptions,
    fields: &mut [Option<String>],
) {
    let value_digits: Vec<char> = value.to_string().chars().collect();
    prepare_integer_digit_fields(&value_digits, spec, opts, fields);
}

/// Map an arbitrary-length integer digit slice to compiled placeholders.
fn prepare_integer_digit_fields(
    value_digits: &[char],
    spec: &NumberSpec,
    opts: &FormatOptions,
    fields: &mut [Option<String>],
) {
    let placeholders = &spec.integer_placeholders;
    if placeholders.is_empty() {
        return;
    }

    let minimum_digits = placeholders
        .iter()
        .filter(|field| field.placeholder.is_required())
        .count();
    if value_digits.iter().all(|character| *character == '0') && minimum_digits == 0 {
        return;
    }

    // Without grouping SSF evaluates the full placeholder width. With grouping
    // it narrows optional leading placeholders to the displayed digit width.
    let uses_grouping = spec.uses_thousands();
    let position_count = if uses_grouping {
        value_digits.len().max(minimum_digits)
    } else {
        value_digits.len().max(placeholders.len())
    };
    let overflow_count = position_count.saturating_sub(placeholders.len());
    let active_count = position_count.min(placeholders.len());
    let active_start = placeholders.len() - active_count;
    let leading_padding = position_count - value_digits.len();

    // Overflow positions share the first placeholder field so generated digits
    // and grouping remain before every following source operation.
    let assigned_placeholders = std::iter::repeat_n(placeholders[0], overflow_count)
        .chain(placeholders[active_start..].iter().copied());
    let mut positions = assigned_placeholders.enumerate().peekable();

    while let Some((logical_index, placeholder)) = positions.next() {
        let character = if logical_index < leading_padding {
            placeholder.placeholder.empty_char()
        } else {
            Some(value_digits[logical_index - leading_padding])
        };
        if let Some(character) = character {
            fields[placeholder.operation_index]
                .get_or_insert_with(String::new)
                .push(character);
        }

        let positions_remaining = position_count - logical_index - 1;
        if uses_grouping && positions_remaining % 3 == 0 {
            let Some((_, next_placeholder)) = positions.peek().copied() else {
                continue;
            };

            // Prefer the source comma between these placeholders so adjacent
            // fills and literals retain their operation order.
            let source_anchor =
                spec.grouping_comma_indices
                    .iter()
                    .copied()
                    .find(|operation_index| {
                        placeholder.operation_index < *operation_index
                            && *operation_index < next_placeholder.operation_index
                    });

            if let Some(operation_index) = source_anchor {
                fields[operation_index] = Some(opts.locale.thousands_separator.to_string());
            } else {
                // Overflow boundaries have no distinct source operation and
                // therefore remain in the repeated leading placeholder field.
                fields[placeholder.operation_index]
                    .get_or_insert_with(String::new)
                    .push(opts.locale.thousands_separator);
            }
        }
    }
}

/// Evaluate exact arbitrary-length integer digits through a compiled number plan.
#[cfg(feature = "bigint")]
pub(super) fn evaluate_integer_digits(
    digits: &str,
    plan: &SectionPlan,
    opts: &FormatOptions,
) -> Result<Vec<RenderPart>, FormatError> {
    let spec = plan.number.as_ref().ok_or(FormatError::TypeMismatch {
        expected: "compiled number format",
        got: "non-number section",
    })?;
    let mut fields = vec![None; plan.operations.len()];
    let value_digits: Vec<char> = digits.chars().collect();
    prepare_integer_digit_fields(&value_digits, spec, opts, &mut fields);
    prepare_decimal_fields(0.0, spec, &mut fields);
    if let Some(operation_index) = spec.decimal_point_index {
        fields[operation_index]
            .get_or_insert_with(String::new)
            .push(opts.locale.decimal_separator);
    }

    Ok(super::evaluate_operations(
        plan,
        |operation_index, part| match part {
            FormatPart::Digit(_) | FormatPart::DecimalPoint | FormatPart::ThousandsSeparator => {
                fields[operation_index].clone()
            }
            FormatPart::Percent => Some("%".to_string()),
            FormatPart::Locale(locale) => locale.currency.clone(),
            _ => None,
        },
    ))
}

/// Map rounded fractional digits and optional padding to decimal placeholders.
fn prepare_decimal_fields(value: f64, spec: &NumberSpec, fields: &mut [Option<String>]) {
    let placeholders = &spec.decimal_placeholders;
    if placeholders.is_empty() {
        return;
    }

    // Align fractional extraction with the numeric rounding cap above.
    // A fractional field scaled by at most 10^15 fits safely in u64.
    let effective_places = placeholders.len().min(15);
    let multiplier = 10_f64.powi(effective_places as i32);
    let decimal_integer = (value * multiplier).round() as u64;
    let decimal_text = format!("{decimal_integer:0>effective_places$}");
    let decimal_digits: Vec<char> = decimal_text.chars().collect();
    let all_zeros = decimal_digits.iter().all(|character| *character == '0');
    let mut trailing_zeros_start = if all_zeros { 0 } else { placeholders.len() };

    if !all_zeros {
        for index in (0..placeholders.len().min(effective_places)).rev() {
            if decimal_digits.get(index) == Some(&'0') {
                if !placeholders[index].placeholder.is_required() {
                    trailing_zeros_start = index;
                } else {
                    break;
                }
            } else {
                break;
            }
        }
    }

    for (index, field) in placeholders.iter().enumerate() {
        let character = if index < effective_places {
            decimal_digits.get(index).copied().unwrap_or('0')
        } else {
            match field.placeholder.empty_char() {
                Some(character) => character,
                None => continue,
            }
        };

        if index >= trailing_zeros_start && character == '0' && !field.placeholder.is_required() {
            if matches!(field.placeholder, DigitPlaceholder::Question) {
                fields[field.operation_index] = Some(" ".to_string());
            }
        } else {
            fields[field.operation_index] = Some(character.to_string());
        }
    }
}

/// Format a simple integer value with digit placeholders.
///
/// This follows SSF's `write_num` helper and is shared by fraction fields.
pub(crate) fn format_simple_with_placeholders(
    value: u64,
    placeholders: &[DigitPlaceholder],
) -> String {
    if placeholders.is_empty() {
        return value.to_string();
    }

    let value_text = value.to_string();
    let value_digits: Vec<char> = value_text.chars().collect();
    if value_digits.len() > placeholders.len() {
        return value_text;
    }

    let mut characters = Vec::with_capacity(placeholders.len());
    for position_from_right in 0..placeholders.len() {
        let digit_index = value_digits.len() as isize - 1 - position_from_right as isize;
        let placeholder_index = placeholders.len() - 1 - position_from_right;
        if digit_index >= 0 {
            characters.push(value_digits[digit_index as usize]);
        } else if let Some(character) = placeholders[placeholder_index].empty_char() {
            characters.push(character);
        }
    }

    characters.reverse();
    characters.into_iter().collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn preserves_fill_inside_numeric_placeholders() {
        let format = crate::NumberFormat::parse("0*-0").unwrap();
        let plan = &format.compiled.sections[0];
        let parts = evaluate_number(42.0, plan, &FormatOptions::default()).unwrap();

        assert_eq!(
            super::super::render::resolve_layout(&parts, 3).unwrap(),
            "4---2"
        );
    }

    #[test]
    fn preserves_fill_inside_scientific_fields() {
        let format = crate::NumberFormat::parse("0.0*xE+00").unwrap();
        let plan = &format.compiled.sections[0];
        let parts = evaluate_scientific(120.0, plan, &FormatOptions::default()).unwrap();

        assert_eq!(
            super::super::render::resolve_layout(&parts, 3).unwrap(),
            "1.2xxxE+02"
        );
    }

    #[test]
    fn renormalizes_scientific_mantissa_after_rounding() {
        let format = crate::NumberFormat::parse("0*x.0E+0").unwrap();
        let plan = &format.compiled.sections[0];
        let parts = evaluate_scientific(9.96, plan, &FormatOptions::default()).unwrap();

        assert_eq!(
            super::super::render::resolve_layout(&parts, 3).unwrap(),
            "1xxx.0E+1"
        );

        let format = crate::NumberFormat::parse("0*x0.0E+0").unwrap();
        let plan = &format.compiled.sections[0];
        let parts = evaluate_scientific(99.96, plan, &FormatOptions::default()).unwrap();

        assert_eq!(
            super::super::render::resolve_layout(&parts, 3).unwrap(),
            "0xxx1.0E+2"
        );
    }

    #[test]
    fn preserves_implicit_scientific_mantissa_integer() {
        let format = crate::NumberFormat::parse(".0E+00").unwrap();
        let plan = &format.compiled.sections[0];
        let parts = evaluate_scientific(1.2, plan, &FormatOptions::default()).unwrap();

        assert_eq!(
            super::super::render::resolve_layout(&parts, 0).unwrap(),
            "1.2E+00"
        );

        let format = crate::NumberFormat::parse("E+00").unwrap();
        let plan = &format.compiled.sections[0];
        let parts = evaluate_scientific(1.2, plan, &FormatOptions::default()).unwrap();

        assert_eq!(
            super::super::render::resolve_layout(&parts, 0).unwrap(),
            "1E+00"
        );
    }
}
