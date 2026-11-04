//! Bounded, fallible formatting for untrusted worksheet previews.
//!
//! These limits are application admission policy, not OOXML grammar maxima.
//! Admission happens before the upstream evaluator clones semantic text or
//! creates precision/fill strings. This module deliberately accepts only the
//! audited built-in English locale. Unsupported numerical ranges return an
//! error so callers can retain the original value with an explicit warning.
use crate::ast::{DatePart, FormatPart, NumberFormat};
use crate::compile::{FractionDenominatorSpec, Operation, SectionKind, SectionPlan};
use crate::error::{FormatError, ParseError};
use crate::options::FormatOptions;
use crate::parser::{lexer::Lexer, tokens::Token};
use crate::Locale;

pub const MAX_FORMAT_BYTES: usize = 4096;
pub const MAX_SECTIONS: usize = 4;
pub const MAX_PARTS: usize = 8192;
pub const MAX_CELL_OUTPUT_BYTES: usize = 131_072;
pub const MAX_TOTAL_OUTPUT_BYTES: usize = 33_554_432;
pub const MAX_WORK_UNITS: u64 = 8_000_000;
pub const MAX_FILL_COUNT: usize = 256;

#[derive(Debug, thiserror::Error)]
pub enum Error {
    #[error("{0}")]
    Parse(#[from] ParseError),
    #[error("{0}")]
    Format(#[from] FormatError),
    #[error("format admission limit: {0}")]
    Limit(&'static str),
    #[error("unsupported preview format: {0}")]
    Unsupported(&'static str),
}
impl Error {
    pub fn code(&self) -> &'static str {
        match self {
            Self::Parse(ParseError::TooManySections) => "format-section-limit",
            Self::Parse(_) | Self::Format(_) => "invalid-format-code",
            Self::Limit(code) => code,
            Self::Unsupported(_) => "unsupported-format",
        }
    }
}

/// One budget belongs to one workbook; failed work is not refunded.
#[derive(Debug)]
pub struct Budget {
    work_left: u64,
    output_left: usize,
}
impl Default for Budget {
    fn default() -> Self {
        Self::with_limits(MAX_WORK_UNITS, MAX_TOTAL_OUTPUT_BYTES)
    }
}
impl Budget {
    pub fn with_limits(work_units: u64, output_bytes: usize) -> Self {
        Self {
            work_left: work_units.min(MAX_WORK_UNITS),
            output_left: output_bytes.min(MAX_TOTAL_OUTPUT_BYTES),
        }
    }
    /// Charge caller-side normalization before it scans an admitted format code.
    pub fn charge_work(&mut self, units: u64) -> Result<(), Error> {
        self.work(units.max(1))
    }
    pub fn remaining(&self) -> (u64, usize) {
        (self.work_left, self.output_left)
    }
    fn work(&mut self, units: u64) -> Result<(), Error> {
        if units > self.work_left {
            return Err(Error::Limit("format-work-limit"));
        }
        self.work_left -= units;
        Ok(())
    }
    fn admit(&mut self, work: u64, output: usize) -> Result<(), Error> {
        if output > MAX_CELL_OUTPUT_BYTES || output > self.output_left {
            return Err(Error::Limit("format-output-limit"));
        }
        self.work(work)?;
        // Reserve the proven upper bound before evaluating; do not refund
        // speculative work/output on an error and permit retry amplification.
        self.output_left -= output;
        Ok(())
    }
}

#[derive(Clone, Debug)]
pub struct Format {
    inner: NumberFormat,
    preflight_work: u64,
}
impl Format {
    pub fn parse(code: &str, budget: &mut Budget) -> Result<Self, Error> {
        // Charge every attempt, including empty and overlong rejected inputs.
        budget.work(32)?;
        if code.len() > MAX_FORMAT_BYTES {
            return Err(Error::Limit("format-length-limit"));
        }
        let n = code.len() as u64;
        // Parser fraction/subsecond normalization contains nested scans.
        // Charge a conservative quadratic envelope before it is invoked.
        let work = n
            .checked_mul(n)
            .and_then(|v| v.checked_mul(4))
            .and_then(|v| v.checked_add(n * 16))
            .ok_or(Error::Limit("format-work-limit"))?;
        budget.work(work)?;
        let mut lexer = Lexer::new(code);
        let mut sections = 1;
        loop {
            match lexer.next_token()?.token {
                Token::Eof => break,
                Token::SectionSep => {
                    sections += 1;
                    if sections > MAX_SECTIONS {
                        return Err(Error::Limit("format-section-limit"));
                    }
                }
                // These consume exactly one raw character in the parser.
                Token::Asterisk | Token::Underscore => {
                    lexer.next_literal_char();
                }
                _ => {}
            }
        }
        let inner = NumberFormat::parse(code)?;
        let parts = inner
            .sections()
            .iter()
            .try_fold(0usize, |total, section| {
                total.checked_add(section.parts().len())
            })
            .ok_or(Error::Limit("format-work-limit"))?;
        if parts > MAX_PARTS {
            return Err(Error::Limit("format-work-limit"));
        }
        let preflight_work = u64::try_from(parts)
            .ok()
            .and_then(|n| n.checked_mul(8))
            .and_then(|n| n.checked_add(32))
            .ok_or(Error::Limit("format-work-limit"))?;
        Ok(Self {
            inner,
            preflight_work,
        })
    }

    pub fn number(
        &self,
        value: f64,
        options: &FormatOptions,
        budget: &mut Budget,
    ) -> Result<String, Error> {
        // Debit before options, section selection and all fallible scans.
        budget.work(self.preflight_work)?;
        Self::options(options)?;
        if !value.is_finite() {
            return Err(Error::Unsupported("non-finite-number"));
        }
        let index = self.inner.select_section_index(value);
        let plan = &self.inner.compiled.sections[index];
        validate_number(plan, value)?;
        let output = output_bound(plan, None, options.fill_count)?;
        budget.admit(work_bound(plan, output)?, output)?;
        let rendered = self.inner.try_format(value, options)?;
        ensure_bound(rendered, output)
    }

    pub fn text(
        &self,
        value: &str,
        options: &FormatOptions,
        budget: &mut Budget,
    ) -> Result<String, Error> {
        // Even an empty section or repeated refused output spends this debit.
        budget.work(self.preflight_work)?;
        Self::options(options)?;
        let plan = self
            .inner
            .select_text_section_index()
            .map(|index| &self.inner.compiled.sections[index]);
        let output = match plan {
            Some(plan) => output_bound(plan, Some(value.len()), options.fill_count)?,
            None => value.len(),
        };
        let work = match plan {
            Some(plan) => work_bound(plan, output)?,
            None => u64::try_from(output).map_err(|_| Error::Limit("format-work-limit"))?,
        };
        budget.admit(work, output)?;
        let rendered = self.inner.try_format_text(value, options)?;
        ensure_bound(rendered, output)
    }

    fn options(options: &FormatOptions) -> Result<(), Error> {
        if options.fill_count > MAX_FILL_COUNT {
            return Err(Error::Limit("format-fill-limit"));
        }
        if options.locale != Locale::en_us() {
            return Err(Error::Unsupported("locale"));
        }
        Ok(())
    }
}

fn ensure_bound(rendered: String, proven: usize) -> Result<String, Error> {
    // Defense against a future evaluator change invalidating the preflight
    // proof. The admission guard above is the allocation boundary.
    if rendered.len() > proven {
        return Err(Error::Limit("format-output-limit"));
    }
    Ok(rendered)
}

fn work_bound(plan: &SectionPlan, output: usize) -> Result<u64, Error> {
    let operations =
        u64::try_from(plan.operations.len()).map_err(|_| Error::Limit("format-work-limit"))?;
    let output = u64::try_from(output).map_err(|_| Error::Limit("format-work-limit"))?;
    operations
        .checked_mul(32)
        .and_then(|v| v.checked_add(output))
        .ok_or(Error::Limit("format-work-limit"))
}

fn output_bound(
    plan: &SectionPlan,
    text_bytes: Option<usize>,
    fill_count: usize,
) -> Result<usize, Error> {
    if plan.operations.is_empty() {
        return Ok(0);
    }
    plan.operations.iter().try_fold(1usize, |total, operation| {
        let bytes = match operation {
            Operation::Text(text) => text.len(),
            Operation::Fill(ch) => fill_count
                .checked_mul(ch.len_utf8())
                .ok_or(Error::Limit("format-output-limit"))?,
            Operation::Skip(_) => 1,
            Operation::Semantic(FormatPart::TextPlaceholder) => text_bytes.unwrap_or(384),
            Operation::Semantic(FormatPart::Locale(locale)) => {
                locale.currency.as_ref().map_or(0, String::len)
            }
            // Under validate_number and the fixed locale, a numeric/date atom
            // is at most 384 UTF-8 bytes (including fixed f64 General output).
            Operation::Semantic(_) => 384,
        };
        total
            .checked_add(bytes)
            .ok_or(Error::Limit("format-output-limit"))
    })
}

fn validate_number(plan: &SectionPlan, value: f64) -> Result<(), Error> {
    let number = value.abs();
    if plan.date.is_hijri {
        return Err(Error::Unsupported("hijri-calendar"));
    }
    if plan.operations.iter().any(|operation| {
        matches!(operation,
        Operation::Semantic(FormatPart::DatePart(DatePart::SubSecond(places)))
        if *places > 3)
    }) {
        // Canonical SSF implements .0, .00 and .000; the upstream public
        // renderer's wider safe range is not a claim of SSF compatibility.
        return Err(Error::Unsupported("subsecond-precision"));
    }
    if plan.kind == SectionKind::General && plan.condition.is_some() {
        return Err(Error::Unsupported("conditional-general"));
    }
    if let Some(spec) = &plan.number {
        if spec.decimal_placeholders.len() > 15 {
            return Err(Error::Unsupported("decimal-precision"));
        }
        let mut scaled = number;
        for _ in 0..spec.percent_count {
            scaled *= 100.0;
            if !scaled.is_finite() || scaled >= u64::MAX as f64 {
                return Err(Error::Unsupported("scaled-number-range"));
            }
        }
        if scaled >= u64::MAX as f64 {
            return Err(Error::Unsupported("integer-range"));
        }
    }
    if let Some(spec) = &plan.scientific {
        if spec.mantissa_decimal.len() > 15 {
            return Err(Error::Unsupported("scientific-precision"));
        }
        let mut scaled = number;
        for _ in 0..spec.percent_count {
            scaled *= 100.0;
        }
        if !scaled.is_finite() {
            return Err(Error::Unsupported("scaled-number-range"));
        }
        if scaled > 0.0 {
            let places = spec.mantissa_integer.len().max(1);
            let base = scaled.log10().floor();
            let exponent = ((base / places as f64).floor() as i32)
                .checked_mul(
                    i32::try_from(places).map_err(|_| Error::Unsupported("scientific-range"))?,
                )
                .ok_or(Error::Unsupported("scientific-range"))?;
            let scale = 10f64.powi(exponent);
            if scale == 0.0 || !scale.is_finite() || !(scaled / scale).is_finite() {
                return Err(Error::Unsupported("scientific-range"));
            }
        }
    }
    if let Some(spec) = &plan.fraction {
        if number >= u64::MAX as f64 {
            return Err(Error::Unsupported("fraction-range"));
        }
        if spec.integer_placeholders.is_empty() {
            let denominator = match &spec.denominator {
                FractionDenominatorSpec::Fixed { value, .. } => f64::from(*value),
                FractionDenominatorSpec::Variable { placeholders } => {
                    f64::from(10u32.pow(placeholders.len().min(7) as u32) - 1)
                }
            };
            if number * denominator > f64::from(u32::MAX) {
                return Err(Error::Unsupported("fraction-range"));
            }
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests;
