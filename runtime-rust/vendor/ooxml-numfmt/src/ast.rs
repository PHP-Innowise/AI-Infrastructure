//! AST types for parsed format codes.

use crate::compile::CompiledFormat;
use crate::error::ParseError;
use std::fmt;
use std::str::FromStr;

/// Named colors supported in format codes.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum NamedColor {
    Black,
    Blue,
    Cyan,
    Green,
    Magenta,
    Red,
    White,
    Yellow,
}

impl FromStr for NamedColor {
    type Err = ();

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        match s.to_lowercase().as_str() {
            "black" => Ok(NamedColor::Black),
            "blue" => Ok(NamedColor::Blue),
            "cyan" => Ok(NamedColor::Cyan),
            "green" => Ok(NamedColor::Green),
            "magenta" => Ok(NamedColor::Magenta),
            "red" => Ok(NamedColor::Red),
            "white" => Ok(NamedColor::White),
            "yellow" => Ok(NamedColor::Yellow),
            _ => Err(()),
        }
    }
}

/// Color specification in a format section.
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum Color {
    Named(NamedColor),
    Indexed(u8),
}

/// Conditional expression for section selection.
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum Condition {
    GreaterThan(f64),
    LessThan(f64),
    Equal(f64),
    GreaterOrEqual(f64),
    LessOrEqual(f64),
    NotEqual(f64),
}

impl Condition {
    /// Evaluate this condition against a value.
    pub fn evaluate(&self, value: f64) -> bool {
        match self {
            Condition::GreaterThan(n) => value > *n,
            Condition::LessThan(n) => value < *n,
            Condition::Equal(n) => (value - n).abs() < f64::EPSILON,
            Condition::GreaterOrEqual(n) => value >= *n,
            Condition::LessOrEqual(n) => value <= *n,
            Condition::NotEqual(n) => (value - n).abs() >= f64::EPSILON,
        }
    }

    /// Check if value strictly satisfies the condition (not at boundary).
    /// For <=, >=, and = conditions, returns true only if not exactly equal.
    /// For <, >, and != conditions, returns same as evaluate().
    pub fn is_strict_match(&self, value: f64) -> bool {
        match self {
            Condition::GreaterThan(n) => value > *n,
            Condition::LessThan(n) => value < *n,
            Condition::Equal(n) => (value - n).abs() < f64::EPSILON,
            Condition::GreaterOrEqual(n) => value > *n, // Strictly greater, not equal
            Condition::LessOrEqual(n) => value < *n,    // Strictly less, not equal
            Condition::NotEqual(n) => (value - n).abs() >= f64::EPSILON,
        }
    }
}

/// Digit placeholder type.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DigitPlaceholder {
    /// `0` - Display digit or zero
    Zero,
    /// `#` - Display digit or nothing
    Hash,
    /// `?` - Display digit or space
    Question,
}

impl DigitPlaceholder {
    /// Returns true if this placeholder requires a digit (shows 0 for missing).
    pub fn is_required(&self) -> bool {
        matches!(self, DigitPlaceholder::Zero)
    }

    /// Returns the character to display when no digit is present.
    pub fn empty_char(&self) -> Option<char> {
        match self {
            DigitPlaceholder::Zero => Some('0'),
            DigitPlaceholder::Hash => None,
            DigitPlaceholder::Question => Some(' '),
        }
    }
}

/// Date/time format parts.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DatePart {
    /// `yy` - Two-digit year
    Year2,
    /// `yyy` - Three-digit year (minimum 3 digits, shows full year)
    Year3,
    /// `yyyy` - Four-digit year
    Year4,
    /// `m` - Month as number without leading zero (1-12)
    Month,
    /// `mm` - Month as number with leading zero (01-12)
    Month2,
    /// `mmm` - Month as abbreviated name (Jan, Feb, etc.)
    MonthAbbr,
    /// `mmmm` - Month as full name (January, February, etc.)
    MonthFull,
    /// `mmmmm` - Month as single letter (J, F, M, etc.)
    MonthLetter,
    /// `d` - Day of month without leading zero (1-31)
    Day,
    /// `dd` - Day of month with leading zero (01-31)
    Day2,
    /// `ddd` - Day of week as abbreviated name (Sun, Mon, etc.)
    DayAbbr,
    /// `dddd` - Day of week as full name (Sunday, Monday, etc.)
    DayFull,
    /// `h` - Hour without leading zero (0-23 or 1-12 with AM/PM)
    Hour,
    /// `hh` - Hour with leading zero (00-23 or 01-12 with AM/PM)
    Hour2,
    /// `m` - Minute without leading zero (0-59), when following h/hh
    Minute,
    /// `mm` - Minute with leading zero (00-59), when following h/hh
    Minute2,
    /// `s` - Second without leading zero (0-59)
    Second,
    /// `ss` - Second with leading zero (00-59)
    Second2,
    /// `.0`, `.00`, etc. - Fractional seconds with specified decimal places
    SubSecond(u8),
    /// `b` or `bb` - Buddhist year (Thai calendar), last 2 digits (Gregorian + 543)
    BuddhistYear2,
    /// `bbbb` - Buddhist year (Thai calendar), 4 digits (Gregorian + 543)
    BuddhistYear4,
    /// `B2yyyy` - Alternative Buddhist calendar era, 4 digits (Gregorian - 582)
    BuddhistYear4Alt,
    /// `B2yy` - Alternative Buddhist calendar era, last 2 digits (Gregorian - 582)
    BuddhistYear2Alt,
}

/// AM/PM format style.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AmPmStyle {
    /// `AM/PM` - Uppercase AM or PM
    Upper,
    /// `am/pm` - Lowercase am or pm
    Lower,
    /// `A/P` - Uppercase single letter A or P
    ShortUpper,
    /// `a/p` - Lowercase single letter a or p
    ShortLower,
    /// `AM/P` - Malformed uppercase pattern (outputs A0/P or A1/P)
    MalformedUpper,
    /// `am/p` - Malformed lowercase pattern (outputs a0/p or a1/p)
    MalformedLower,
}

/// Elapsed time format part (for durations).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ElapsedPart {
    /// `[h]` - Total elapsed hours without padding
    Hours,
    /// `[hh]` - Total elapsed hours with zero-padding to 2 digits
    Hours2,
    /// `[m]` - Total elapsed minutes without padding
    Minutes,
    /// `[mm]` - Total elapsed minutes with zero-padding to 2 digits
    Minutes2,
    /// `[s]` - Total elapsed seconds without padding
    Seconds,
    /// `[ss]` - Total elapsed seconds with zero-padding to 2 digits
    Seconds2,
}

/// A source-ordered semantic component of a fraction format.
///
/// Literal spacing and layout directives between these components remain
/// separate [`FormatPart`] values in the syntax tree. Programmatic sections
/// without a [`FractionPart::Slash`] are normalized back to ordinary numeric
/// and literal parts by [`Section::new`], matching parser behavior for format
/// codes that are not fractions.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FractionPart {
    /// A digit placeholder belonging to the whole-number portion of a mixed fraction.
    IntegerDigit(DigitPlaceholder),
    /// A digit placeholder belonging to the numerator.
    NumeratorDigit(DigitPlaceholder),
    /// The fraction slash.
    Slash,
    /// A digit placeholder belonging to a variable denominator.
    DenominatorDigit(DigitPlaceholder),
    /// One source digit belonging to a fixed denominator.
    ///
    /// Programmatic sections must use values from `0` through `9`, and the
    /// complete fixed denominator must fit in a [`u32`].
    FixedDenominatorDigit(u8),
}

/// Locale code from format string.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LocaleCode {
    /// Currency symbol to display (e.g., "$", "€", "£")
    pub currency: Option<String>,
    /// Windows Locale Identifier for language/region-specific formatting
    pub lcid: Option<u32>,
}

/// A single part of a format section.
#[derive(Debug, Clone, PartialEq)]
pub enum FormatPart {
    /// Literal text to display as-is (from unescaped characters or quoted strings)
    Literal(String),
    /// Escaped literal character (e.g., `\r`, `\#`) - does not get minus sign for negative values
    EscapedLiteral(String),
    /// Digit placeholder (0, #, or ?)
    Digit(DigitPlaceholder),
    /// Decimal point separator
    DecimalPoint,
    /// Thousands grouping separator (comma in US locale)
    ThousandsSeparator,
    /// Percent sign - multiplies value by 100
    Percent,
    /// Scientific notation (E+, E-, e+, e-)
    Scientific {
        /// True for uppercase E, false for lowercase e
        upper: bool,
        /// True to always show sign, false for minus only
        show_plus: bool,
    },
    /// One source-ordered fraction component (for example, a numerator digit or slash).
    ///
    /// A complete fraction is represented by multiple `Fraction` parts. Source
    /// spacing remains represented by [`FormatPart::Literal`], and fixed
    /// denominator digits remain distinct components.
    Fraction(FractionPart),
    /// Date/time component
    DatePart(DatePart),
    /// AM/PM indicator
    AmPm(AmPmStyle),
    /// Elapsed time component for durations
    Elapsed(ElapsedPart),
    /// `@` - Text placeholder for text values
    TextPlaceholder,
    /// `*x` - Repeat character to fill available width
    Fill(char),
    /// `_x` - Skip width of character (for alignment)
    Skip(char),
    /// `[$...]` - Locale/currency specification
    Locale(LocaleCode),
    /// General number formatting (used when "General" keyword appears with additional format parts)
    GeneralNumber,
}

impl FormatPart {
    /// Returns true if this is a date/time part.
    pub fn is_date_part(&self) -> bool {
        matches!(
            self,
            FormatPart::DatePart(_) | FormatPart::AmPm(_) | FormatPart::Elapsed(_)
        )
    }

    /// Returns true if this is a numeric formatting part.
    pub fn is_numeric_part(&self) -> bool {
        matches!(
            self,
            FormatPart::Digit(_)
                | FormatPart::DecimalPoint
                | FormatPart::ThousandsSeparator
                | FormatPart::Percent
                | FormatPart::Scientific { .. }
                | FormatPart::Fraction(_)
        )
    }
}

/// A single section of a format code.
///
/// Format codes can have up to 4 sections:
/// 1. Positive numbers (or all numbers if only one section)
/// 2. Negative numbers
/// 3. Zero
/// 4. Text
#[derive(Debug, Clone, PartialEq)]
pub struct Section {
    /// Optional condition for this section (e.g., `[>100]`)
    condition: Option<Condition>,
    /// Optional color for this section (e.g., `[Red]`)
    color: Option<Color>,
    /// The format parts that make up this section
    parts: Vec<FormatPart>,
}

impl Section {
    /// Create a section from optional selectors and semantic syntax parts.
    ///
    /// When multiple fill directives are supplied, only the final directive is
    /// retained at its original position, matching parser normalization.
    /// Fraction components without a slash are restored to ordinary numeric
    /// placeholders and literals rather than treated as fraction syntax.
    pub fn new(
        condition: Option<Condition>,
        color: Option<Color>,
        mut parts: Vec<FormatPart>,
    ) -> Self {
        if !parts
            .iter()
            .any(|part| matches!(part, FormatPart::Fraction(FractionPart::Slash)))
        {
            for part in &mut parts {
                let replacement = match part {
                    FormatPart::Fraction(
                        FractionPart::IntegerDigit(placeholder)
                        | FractionPart::NumeratorDigit(placeholder)
                        | FractionPart::DenominatorDigit(placeholder),
                    ) => Some(FormatPart::Digit(*placeholder)),
                    FormatPart::Fraction(FractionPart::FixedDenominatorDigit(digit)) => {
                        Some(FormatPart::Literal(digit.to_string()))
                    }
                    _ => None,
                };
                if let Some(replacement) = replacement {
                    *part = replacement;
                }
            }
        }

        if let Some(final_fill_index) = parts
            .iter()
            .rposition(|part| matches!(part, FormatPart::Fill(_)))
        {
            let mut index = 0;
            parts.retain(|part| {
                let retain = !matches!(part, FormatPart::Fill(_)) || index == final_fill_index;
                index += 1;
                retain
            });
        }
        Self {
            condition,
            color,
            parts,
        }
    }

    /// Return this section's optional selection condition.
    pub fn condition(&self) -> Option<Condition> {
        self.condition
    }

    /// Return this section's optional display color.
    pub fn color(&self) -> Option<Color> {
        self.color
    }

    /// Return the normalized syntax parts in source-relative order.
    pub fn parts(&self) -> &[FormatPart] {
        &self.parts
    }

    /// Returns true if this section contains any date/time parts.
    pub fn has_date_parts(&self) -> bool {
        self.parts().iter().any(|p| p.is_date_part())
    }

    /// Returns true if this section contains a text placeholder.
    pub fn has_text_placeholder(&self) -> bool {
        self.parts()
            .iter()
            .any(|p| matches!(p, FormatPart::TextPlaceholder))
    }

    /// Returns true if this section contains a percent sign.
    pub fn has_percent(&self) -> bool {
        self.parts()
            .iter()
            .any(|p| matches!(p, FormatPart::Percent))
    }
}

/// A parsed number format code.
///
/// This is the main type returned by parsing. It can be reused to format
/// multiple values efficiently.
#[derive(Clone)]
pub struct NumberFormat {
    sections: Box<[Section]>,
    pub(crate) compiled: CompiledFormat,
}

impl PartialEq for NumberFormat {
    fn eq(&self, other: &Self) -> bool {
        self.sections == other.sections
    }
}

impl fmt::Debug for NumberFormat {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("NumberFormat")
            .field("sections", &self.sections)
            .finish()
    }
}

impl NumberFormat {
    /// Create a `NumberFormat` from programmatically supplied sections.
    ///
    /// At most four sections are retained, matching Excel's section limit.
    /// Explicit [`FractionPart`] components are validated before the compiled
    /// execution plan is created.
    ///
    /// # Errors
    ///
    /// Returns [`ParseError::InvalidFraction`] when a complete fraction contains
    /// a non-decimal fixed-denominator component or has a fixed denominator
    /// larger than [`u32::MAX`].
    pub fn from_sections(sections: Vec<Section>) -> Result<Self, ParseError> {
        let sections = if sections.len() > 4 {
            sections.into_iter().take(4).collect()
        } else {
            sections
        };
        let sections = sections.into_boxed_slice();
        let compiled = CompiledFormat::new(&sections)?;
        Ok(NumberFormat { sections, compiled })
    }

    /// Get the sections of this format.
    pub fn sections(&self) -> &[Section] {
        &self.sections
    }

    /// Returns true if this format contains date/time parts.
    pub fn is_date_format(&self) -> bool {
        self.compiled
            .sections
            .iter()
            .any(|plan| plan.kind == crate::compile::SectionKind::DateTime)
    }

    /// Returns true if this is a text-only format.
    pub fn is_text_format(&self) -> bool {
        self.compiled.sections.len() == 1
            && self.compiled.sections[0].kind == crate::compile::SectionKind::Text
    }

    /// Returns true if this format contains a percent sign.
    pub fn is_percentage(&self) -> bool {
        self.sections.iter().any(|s| s.has_percent())
    }

    /// Returns true if any section has a color.
    pub fn has_color(&self) -> bool {
        self.compiled
            .sections
            .iter()
            .any(|plan| plan.color.is_some())
    }

    /// Returns true if any section has a condition.
    pub fn has_condition(&self) -> bool {
        self.compiled
            .sections
            .iter()
            .any(|plan| plan.condition.is_some())
    }

    /// Returns true if any section contains a fill directive.
    pub fn has_fill(&self) -> bool {
        self.sections.iter().any(|section| {
            section
                .parts()
                .iter()
                .any(|part| matches!(part, FormatPart::Fill(_)))
        })
    }

    /// Parse a format code string into a NumberFormat.
    pub fn parse(format_code: &str) -> Result<NumberFormat, ParseError> {
        crate::parser::parse(format_code)
    }
}
