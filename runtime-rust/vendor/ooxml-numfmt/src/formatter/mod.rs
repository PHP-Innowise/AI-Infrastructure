//! Format value formatting engine

mod date;
mod fraction;
mod number;
mod render;
mod text;

#[cfg(feature = "bigint")]
mod bigint;

#[cfg(feature = "bigint")]
#[allow(unused_imports)]
pub use bigint::{fallback_format_bigint, is_safe_integer};

use crate::ast::{FormatPart, NumberFormat};
use crate::compile::{Operation, SectionKind, SectionPlan};
use crate::error::FormatError;
use crate::options::FormatOptions;

fn strip_trailing_char(mut value: &str, suffix: char) -> &str {
    while let Some(prefix) = value.strip_suffix(suffix) {
        value = prefix;
    }
    value
}

impl NumberFormat {
    /// Format a numeric value using this format code.
    ///
    /// This is an infallible method that returns a formatted string.
    /// For date formats or when precise error handling is needed,
    /// use `try_format()` instead.
    pub fn format(&self, value: f64, opts: &FormatOptions) -> String {
        match self.try_format(value, opts) {
            Ok(result) => result,
            Err(_) => fallback_format(value),
        }
    }

    /// Try to format a numeric value using this format code.
    ///
    /// Returns an error if the format cannot be applied to the value.
    pub fn try_format(&self, value: f64, opts: &FormatOptions) -> Result<String, FormatError> {
        // Handle special float values
        if value.is_nan() {
            return Ok("NaN".to_string());
        }
        if value.is_infinite() {
            return Ok(if value.is_sign_positive() {
                "Infinity"
            } else {
                "-Infinity"
            }
            .to_string());
        }

        // Select the appropriate section based on value
        let sections = self.sections();
        let section_index = self.select_section_index(value);
        let section = &sections[section_index];
        let plan = &self.compiled.sections[section_index];

        // Excel behavior: when a conditional section strictly matches, format using absolute value
        // Use absolute value only when the condition is strictly satisfied (not at boundary)
        let has_conditions = self
            .compiled
            .sections
            .iter()
            .any(|plan| plan.condition.is_some());
        let use_abs_value = has_conditions
            && plan.condition.is_some()
            && plan.condition.unwrap().is_strict_match(value);
        let format_value = if use_abs_value { value.abs() } else { value };

        // A true empty section hides the selected value; explicit General has a
        // GeneralNumber marker and retains the upstream conditional behavior.
        if section.parts().is_empty() {
            return Ok(String::new());
        }
        if matches!(section.parts(), [FormatPart::GeneralNumber]) {
            let shown = if use_abs_value {
                format_value.trunc()
            } else {
                format_value
            };
            return render::resolve_layout(
                &[render::RenderPart::Text(fallback_format(shown))],
                opts.fill_count,
            );
        }

        // Check if this is a date format
        if plan.kind == SectionKind::DateTime {
            let parts = date::evaluate_date(format_value, plan, opts)?;
            return render::resolve_layout(&parts, opts.fill_count);
        }

        // Determine if we need to add a minus sign
        // For single-section formats, we add the minus sign ourselves
        // For multi-section formats, the section handles it
        // For literal-only formats (no numeric parts), add minus ONLY if it's a single unescaped single-char literal
        // But NOT if we're using absolute value due to conditional matching
        // Scientific notation retains its specialized sign path until its own refactor.
        let num_sections = sections.len();
        let has_numeric_parts = section.parts().iter().any(|p| p.is_numeric_part());
        let is_single_char_literal = section.parts().len() == 1
            && matches!(&section.parts()[0], FormatPart::Literal(s) if s.len() == 1);
        let need_minus_sign = num_sections == 1
            && value < 0.0
            && (has_numeric_parts || is_single_char_literal)
            && !use_abs_value
            && plan.kind != SectionKind::Scientific;

        if matches!(plan.kind, SectionKind::General | SectionKind::Literal) {
            let mut parts = evaluate_simple_number(plan, format_value);
            if need_minus_sign {
                parts.insert(0, render::RenderPart::Text("-".to_string()));
            }
            return render::resolve_layout(&parts, opts.fill_count);
        }

        if plan.kind == SectionKind::Number {
            let mut parts = number::evaluate_number(format_value, plan, opts)?;
            if need_minus_sign {
                parts.insert(0, render::RenderPart::Text("-".to_string()));
            }
            return render::resolve_layout(&parts, opts.fill_count);
        }

        if plan.kind == SectionKind::Scientific {
            let parts = number::evaluate_scientific(format_value, plan, opts)?;
            return render::resolve_layout(&parts, opts.fill_count);
        }

        if plan.kind == SectionKind::Fraction {
            let mut parts = fraction::evaluate_fraction(format_value, plan, opts)?;
            if need_minus_sign {
                parts.insert(0, render::RenderPart::Text("-".to_string()));
            }
            return render::resolve_layout(&parts, opts.fill_count);
        }

        if plan.kind == SectionKind::Text {
            let mut numeric_text = Some(fallback_format(format_value));
            let parts = evaluate_operations(plan, |_, part| match part {
                FormatPart::TextPlaceholder => numeric_text.take(),
                FormatPart::Locale(locale) => locale.currency.clone(),
                _ => None,
            });
            return render::resolve_layout(&parts, opts.fill_count);
        }

        unreachable!("all compiled section kinds are handled above")
    }

    /// Return the index of the appropriate format section based on the value.
    ///
    /// Section selection rules:
    /// - 1 section: used for all values
    /// - 2 sections: first for positive/zero, second for negative
    /// - 3 sections: positive, negative, zero
    /// - 4 sections: positive, negative, zero, text
    pub fn select_section_index(&self, value: f64) -> usize {
        let plans = &self.compiled.sections;

        // Check if any section has conditions
        let has_conditions = plans.iter().any(|plan| plan.condition.is_some());

        if has_conditions {
            // With conditions: find matching conditional, or first non-conditional
            for (index, plan) in plans.iter().enumerate() {
                if let Some(condition) = plan.condition {
                    if condition.evaluate(value) {
                        return index;
                    }
                } else {
                    // No condition on this section - use it as fallback
                    return index;
                }
            }
            // Fallback to last section if nothing matched
            return plans.len() - 1;
        }

        // Standard section selection based on value sign (no conditions)
        match plans.len() {
            0 => unreachable!("NumberFormat should always have at least one section"),
            1 => 0,
            2 => {
                if value < 0.0 {
                    1
                } else {
                    0
                }
            }
            3 | 4 => {
                if value > 0.0 {
                    0
                } else if value < 0.0 {
                    1
                } else {
                    // Zero value - use section[2]
                    // Unless it's text-only (@), then use positive section
                    if plans[2].kind == SectionKind::Text
                        && !self.sections()[2].parts().iter().any(|p| {
                            p.is_numeric_part()
                                || matches!(
                                    p,
                                    FormatPart::Literal(_) | FormatPart::EscapedLiteral(_)
                                )
                        })
                    {
                        0
                    } else {
                        2
                    }
                }
            }
            _ => 0,
        }
    }

    /// Return the insertion position and character of the effective text fill.
    ///
    /// The position is a UTF-8 byte offset and character boundary in the
    /// selected text section's fill-free output. This is the output produced by
    /// [`Self::try_format_text`] when [`FormatOptions::fill_count`] is zero.
    /// Literal text and the ASCII space emitted by each skip directive are
    /// included in the offset.
    ///
    /// Returns `None` when no text section is selected or the selected section
    /// has no effective fill directive.
    pub fn text_fill_position(&self, text: &str) -> Option<(usize, char)> {
        let section_index = self.select_text_section_index()?;
        let parts = text::evaluate_text(&self.compiled.sections[section_index], text);
        render::fill_position(&parts)
    }

    /// Format a text value using this format code.
    pub fn format_text(&self, text: &str, opts: &FormatOptions) -> String {
        self.try_format_text(text, opts)
            .unwrap_or_else(|_| text.to_string())
    }

    /// Try to format a text value while reporting layout allocation failures.
    pub fn try_format_text(&self, text: &str, opts: &FormatOptions) -> Result<String, FormatError> {
        if let Some(index) = self.select_text_section_index() {
            let parts = text::evaluate_text(&self.compiled.sections[index], text);
            render::resolve_layout(&parts, opts.fill_count)
        } else {
            // Default: return text as-is
            Ok(text.to_string())
        }
    }

    /// Selects the text section with the following policy:
    /// - If the 4th section is present, always return it.
    /// - With fewer sections, use the final section only if it contains `@`.
    /// - Otherwise, return None.
    pub fn select_text_section_index(&self) -> Option<usize> {
        let sections = self.sections();

        // Text section is the 4th section if present
        if sections.len() >= 4 {
            return Some(3);
        }

        // With fewer sections, the final section is the text section only if it contains `@`.
        sections
            .last()
            .filter(|section| section.has_text_placeholder())
            .map(|_| sections.len() - 1)
    }

    /// Format a BigInt value using this format code (requires `bigint` feature).
    ///
    /// For values within f64's safe integer range (±2^53), converts to f64 and uses
    /// standard formatting. For larger values, uses string-based formatting to
    /// preserve precision.
    #[cfg(feature = "bigint")]
    pub fn format_bigint(&self, value: &num_bigint::BigInt, opts: &FormatOptions) -> String {
        match self.try_format_bigint(value, opts) {
            Ok(result) => result,
            Err(_) => bigint::fallback_format_bigint(value),
        }
    }

    /// Try to format a BigInt value using this format code (requires `bigint` feature).
    ///
    /// For values within f64's safe integer range (±2^53), converts to f64 and uses
    /// standard formatting. For larger values, uses string-based formatting to
    /// preserve precision.
    #[cfg(feature = "bigint")]
    pub fn try_format_bigint(
        &self,
        value: &num_bigint::BigInt,
        opts: &FormatOptions,
    ) -> Result<String, FormatError> {
        use num_bigint::Sign;

        // Check if value is within safe f64 range
        if bigint::is_safe_integer(value) {
            // Convert to f64 and use standard formatting
            let float_val: f64 = value.to_string().parse().unwrap_or(0.0);
            return self.try_format(float_val, opts);
        }

        // For large integers, use string-based formatting
        let is_negative = value.sign() == Sign::Minus;
        let section_index = if is_negative && self.sections().len() >= 2 {
            1
        } else {
            0
        };
        let section = &self.sections()[section_index];
        let plan = &self.compiled.sections[section_index];

        // Handle "General" format (empty section with no parts)
        if section.parts().is_empty() {
            return Ok(String::new());
        }

        let mut parts = bigint::evaluate_bigint(value, plan, opts)?;

        // Add minus sign for negative values in single-section formats
        let sections = self.sections();
        if sections.len() == 1 && is_negative && plan.kind == SectionKind::Number {
            parts.insert(0, render::RenderPart::Text("-".to_string()));
        }

        render::resolve_layout(&parts, opts.fill_count)
    }
}

/// Evaluate General and literal-only numeric sections without resolving layout.
fn evaluate_simple_number(plan: &SectionPlan, value: f64) -> Vec<render::RenderPart> {
    evaluate_operations(plan, |_, part| match part {
        FormatPart::GeneralNumber => Some(fallback_format(value)),
        FormatPart::Locale(locale) => locale.currency.clone(),
        FormatPart::Percent => Some("%".to_string()),
        _ => None,
    })
}

/// Execute ordered operations while delegating semantic field evaluation.
fn evaluate_operations(
    plan: &SectionPlan,
    mut evaluate_semantic: impl FnMut(usize, &FormatPart) -> Option<String>,
) -> Vec<render::RenderPart> {
    let mut output = Vec::new();

    for (operation_index, operation) in plan.operations.iter().enumerate() {
        evaluate_operation(
            &mut output,
            operation_index,
            operation,
            &mut evaluate_semantic,
        );
    }

    output
}

/// Evaluate one compiled operation into ordered unresolved render parts.
fn evaluate_operation(
    output: &mut Vec<render::RenderPart>,
    operation_index: usize,
    operation: &Operation,
    evaluate_semantic: &mut impl FnMut(usize, &FormatPart) -> Option<String>,
) {
    match operation {
        Operation::Text(text) => render::push_text(output, text.as_ref()),
        Operation::Fill(character) => output.push(render::RenderPart::Fill(*character)),
        Operation::Skip(character) => output.push(render::RenderPart::Skip(*character)),
        Operation::Semantic(part) => {
            if let Some(text) = evaluate_semantic(operation_index, part) {
                render::push_text(output, text);
            }
        }
    }
}

/// Fallback formatting for when the format code cannot be applied.
///
/// Implements Excel's "General" number format behavior:
/// - Very small numbers (0 < |x| < 1E-4) use scientific notation
/// - Exact integers below 1E11 are displayed without scientific notation
/// - Values at or above 1E11 use scientific notation
/// - No trailing zeros after decimal point
pub fn fallback_format(value: f64) -> String {
    // Handle zero and non-finite values before decimal precision calculations.
    if value == 0.0 {
        return "0".to_string();
    }
    if !value.is_finite() {
        return value.to_string();
    }

    // Integer fast path for exact values below the scientific boundary.
    let int_val = value.trunc() as i64;
    if (value - int_val as f64).abs() < f64::EPSILON && value.abs() >= 1.0 && value.abs() < 1e11 {
        return int_val.to_string();
    }

    let abs_value = value.abs();

    // Choose fixed precision and round before deciding which notation to return.
    // Excel's General format shows up to 11 characters total (including decimal point)
    // but we need to be smart about significant figures.
    let decimal_formatted = if abs_value >= 1.0 {
        // For numbers >= 1, format with appropriate decimal places.
        let integer_digits = abs_value.log10().floor() as usize + 1;
        let decimal_places = if integer_digits >= 10 {
            0
        } else {
            (10 - integer_digits).min(10)
        };
        format!("{:.prec$}", value, prec = decimal_places)
    } else {
        // For numbers < 1, format with up to 9 decimal places (to fit in 11 chars: "0." + 9 digits)
        // Excel's limit is 11 chars for the numeric part, not counting the sign
        // So negative numbers can be up to 12 chars total
        let max_decimals = 9;
        let test_format = format!("{:.prec$}", value, prec = max_decimals);

        // Check length of numeric part only (excluding sign for negative numbers)
        let numeric_part = if value < 0.0 {
            &test_format[1..] // Skip the '-' sign
        } else {
            &test_format[..]
        };

        // If numeric part exceeds 11 chars, reduce decimal places
        if numeric_part.len() > 11 {
            let excess = numeric_part.len() - 11;
            let reduced_decimals = max_decimals.saturating_sub(excess);
            format!("{:.prec$}", value, prec = reduced_decimals)
        } else {
            test_format
        }
    };

    let rounded_reaches_boundary = decimal_formatted
        .parse::<f64>()
        .map(|rounded| rounded.abs() >= 1e11)
        .unwrap_or(false);

    // Very small values use scientific notation when their decimal representation
    // does not fit within General's 11-character limit.
    let small_value_needs_scientific = if abs_value > 0.0 && abs_value < 0.0001 {
        let test_str = format!("{:.15}", abs_value);
        // Trim trailing zeros
        let trimmed = strip_trailing_char(strip_trailing_char(&test_str, '0'), '.');

        // If it doesn't fit in 11 chars, use scientific notation
        trimmed.len() > 11
    } else {
        false
    };

    let use_scientific = rounded_reaches_boundary || small_value_needs_scientific;

    if use_scientific {
        // Format in scientific notation with up to 5 decimal places
        // Excel shows "1.23457E+12" format
        let formatted = format!("{:.5E}", value);

        // Excel uses specific scientific notation format:
        // Remove trailing zeros from mantissa, but keep at least one decimal place
        if let Some(e_pos) = formatted.find('E') {
            let (mantissa, exponent) = formatted.split_at(e_pos);
            let trimmed_mantissa = strip_trailing_char(mantissa, '0');
            let final_mantissa = trimmed_mantissa
                .strip_suffix('.')
                .unwrap_or(trimmed_mantissa);

            // Format exponent to match Excel: E+12, E-05, etc.
            let exp_str = &exponent[1..]; // Skip 'E'
            let exp_value: i32 = exp_str.parse().unwrap_or(0);
            format!("{}E{:+03}", final_mantissa, exp_value)
        } else {
            formatted
        }
    } else {
        // Use decimal notation

        // Trim trailing zeros after decimal point
        if decimal_formatted.contains('.') {
            let trimmed = strip_trailing_char(&decimal_formatted, '0');
            if trimmed.ends_with('.') {
                strip_trailing_char(trimmed, '.').to_string()
            } else {
                trimmed.to_string()
            }
        } else {
            decimal_formatted
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ast::{Condition, DigitPlaceholder, Section};

    fn make_format(sections: Vec<Section>) -> NumberFormat {
        NumberFormat::from_sections(sections).unwrap()
    }

    fn make_section(parts: Vec<FormatPart>) -> Section {
        Section::new(None, None, parts)
    }

    #[test]
    fn test_select_section_single() {
        let fmt = make_format(vec![make_section(vec![FormatPart::Digit(
            DigitPlaceholder::Zero,
        )])]);

        let opts = FormatOptions::default();
        // Single-section formats: negative values get a minus sign prefix
        assert_eq!(fmt.format(42.0, &opts), "42");
        assert_eq!(fmt.format(-42.0, &opts), "-42");
        assert_eq!(fmt.format(0.0, &opts), "0");
    }

    #[test]
    fn test_select_section_two_sections() {
        let fmt = make_format(vec![
            make_section(vec![FormatPart::Digit(DigitPlaceholder::Zero)]),
            make_section(vec![
                FormatPart::Literal("-".to_string()),
                FormatPart::Digit(DigitPlaceholder::Zero),
            ]),
        ]);

        let opts = FormatOptions::default();
        assert_eq!(fmt.format(42.0, &opts), "42");
        assert_eq!(fmt.format(-42.0, &opts), "-42");
        assert_eq!(fmt.format(0.0, &opts), "0");
    }

    #[test]
    fn test_select_section_three_sections() {
        let fmt = make_format(vec![
            make_section(vec![
                FormatPart::Literal("+".to_string()),
                FormatPart::Digit(DigitPlaceholder::Zero),
            ]),
            make_section(vec![
                FormatPart::Literal("-".to_string()),
                FormatPart::Digit(DigitPlaceholder::Zero),
            ]),
            make_section(vec![FormatPart::Literal("ZERO".to_string())]),
        ]);

        let opts = FormatOptions::default();
        assert_eq!(fmt.format(42.0, &opts), "+42");
        assert_eq!(fmt.format(-42.0, &opts), "-42");
        assert_eq!(fmt.format(0.0, &opts), "ZERO");
    }

    #[test]
    fn test_select_section_with_condition() {
        let fmt = make_format(vec![
            Section::new(
                Some(Condition::GreaterThan(100.0)),
                None,
                vec![FormatPart::Literal("BIG".to_string())],
            ),
            make_section(vec![FormatPart::Digit(DigitPlaceholder::Zero)]),
        ]);

        let opts = FormatOptions::default();
        assert_eq!(fmt.format(150.0, &opts), "BIG");
        assert_eq!(fmt.format(50.0, &opts), "50");
    }

    #[test]
    fn test_fallback_format() {
        assert_eq!(fallback_format(42.0), "42");
        assert_eq!(fallback_format(42.5), "42.5");
        assert_eq!(fallback_format(42.123456), "42.123456");
    }

    #[test]
    fn test_format_text() {
        let fmt = make_format(vec![
            make_section(vec![FormatPart::Digit(DigitPlaceholder::Zero)]),
            make_section(vec![FormatPart::Digit(DigitPlaceholder::Zero)]),
            make_section(vec![FormatPart::Digit(DigitPlaceholder::Zero)]),
            make_section(vec![
                FormatPart::Literal("<<".to_string()),
                FormatPart::TextPlaceholder,
                FormatPart::Literal(">>".to_string()),
            ]),
        ]);

        let opts = FormatOptions::default();
        assert_eq!(fmt.format_text("hello", &opts), "<<hello>>");
    }

    #[test]
    fn test_format_text_with_placeholder_in_single_section() {
        let fmt = NumberFormat::parse("\"pre\"@\"post\"").unwrap();
        let opts = FormatOptions::default();

        assert_eq!(fmt.format_text("hello", &opts), "prehellopost");
    }

    #[test]
    fn repeated_text_placeholders_render_numeric_values_once() {
        let format = NumberFormat::parse("@@").unwrap();
        let options = FormatOptions::default();

        assert_eq!(format.format(1.0, &options), "1");
        assert_eq!(format.format(-1.0, &options), "-1");
        assert_eq!(format.format(0.0, &options), "0");
        assert_eq!(format.format_text("sheetjs", &options), "sheetjssheetjs");
    }

    #[test]
    fn test_format_text_with_placeholder_in_second_section() {
        let fmt = NumberFormat::parse("0;\"pre\"@").unwrap();
        let opts = FormatOptions::default();

        assert_eq!(fmt.format_text("hello", &opts), "prehello");
    }

    #[test]
    fn test_format_text_with_placeholder_in_third_section() {
        let fmt = NumberFormat::parse("0;0;\"pre\"@").unwrap();
        let opts = FormatOptions::default();

        assert_eq!(fmt.format_text("hello", &opts), "prehello");
    }

    #[test]
    fn test_format_text_with_literal_only_fourth_section() {
        let fmt = NumberFormat::parse("0;0;0;\"literal\"").unwrap();
        let opts = FormatOptions::default();

        assert_eq!(fmt.format_text("hello", &opts), "literal");
    }

    #[test]
    fn test_format_text_with_explicitly_empty_fourth_section() {
        let fmt = NumberFormat::parse("0;0;0;").unwrap();
        let opts = FormatOptions::default();

        assert_eq!(fmt.format_text("hello", &opts), "");
    }
}
