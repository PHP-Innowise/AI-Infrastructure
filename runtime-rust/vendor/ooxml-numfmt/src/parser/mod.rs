//! Parser for ECMA-376 number format codes.

pub mod lexer;
pub mod tokens;

use crate::ast::{
    AmPmStyle, Color, Condition, DatePart, DigitPlaceholder, ElapsedPart, FormatPart, FractionPart,
    LocaleCode, NamedColor, NumberFormat, Section,
};
use crate::error::ParseError;
use lexer::Lexer;
use tokens::{SpannedToken, Token};

/// Parse a format code string into a NumberFormat.
pub fn parse(format_code: &str) -> Result<NumberFormat, ParseError> {
    if format_code.is_empty() {
        return Err(ParseError::EmptyFormat);
    }

    // Preserve bracket directives and distinguish explicit General from empty sections.
    let mut parser = Parser::new(format_code)?;
    parser.parse()
}

/// Parser for format code strings.
struct Parser<'a> {
    lexer: Lexer<'a>,
    /// Current token
    current: SpannedToken,
    /// Whether we've seen an hour token in the current section (for minute vs month disambiguation)
    seen_hour: bool,
}

impl<'a> Parser<'a> {
    /// Create a new parser for the given format code.
    fn new(format_code: &'a str) -> Result<Self, ParseError> {
        let mut lexer = Lexer::new(format_code);
        // Initial lexical failure is an input error, not an empty section.
        let current = lexer.next_token()?;
        Ok(Self {
            lexer,
            current,
            seen_hour: false,
        })
    }

    /// Advance to the next token.
    fn advance(&mut self) -> Result<(), ParseError> {
        self.current = self.lexer.next_token()?;
        Ok(())
    }

    /// Parse the format code into a NumberFormat.
    fn parse(&mut self) -> Result<NumberFormat, ParseError> {
        let mut sections = Vec::new();

        loop {
            let section = self.parse_section()?;
            sections.push(section);
            if sections.len() > 4 {
                return Err(ParseError::TooManySections);
            }

            // Check for section separator or end
            if matches!(self.current.token, Token::Eof) {
                break;
            }

            if matches!(self.current.token, Token::SectionSep) {
                self.advance()?;
                // Continue to next section
            } else {
                break;
            }
        }

        NumberFormat::from_sections(sections)
    }

    /// Parse a single section of the format.
    fn parse_section(&mut self) -> Result<Section, ParseError> {
        let mut builder = SectionBuilder::new();
        self.seen_hour = false;

        loop {
            match &self.current.token {
                Token::Eof | Token::SectionSep => break,

                // An empty section intentionally hides its selected value.
                Token::General => {
                    builder.add_part(FormatPart::GeneralNumber);
                    self.advance()?;
                }

                // Bracket content - could be color, condition, elapsed time, or locale
                Token::OpenBracket => {
                    let bracket_start = self.current.start;
                    self.advance()?;
                    self.parse_bracket_content(&mut builder, bracket_start)?;
                }

                // Digit placeholders
                Token::Zero => {
                    builder.add_part(FormatPart::Digit(DigitPlaceholder::Zero));
                    self.advance()?;
                }
                Token::Hash => {
                    builder.add_part(FormatPart::Digit(DigitPlaceholder::Hash));
                    self.advance()?;
                }
                Token::Question => {
                    builder.add_part(FormatPart::Digit(DigitPlaceholder::Question));
                    self.advance()?;
                }

                // Separators
                Token::DecimalPoint => {
                    builder.add_part(FormatPart::DecimalPoint);
                    self.advance()?;
                }
                Token::ThousandsSep => {
                    builder.add_part(FormatPart::ThousandsSeparator);
                    self.advance()?;
                }

                // Special characters
                Token::Percent => {
                    builder.add_part(FormatPart::Percent);
                    self.advance()?;
                }
                Token::At => {
                    builder.add_part(FormatPart::TextPlaceholder);
                    self.advance()?;
                }
                Token::Asterisk => {
                    // Fill character - next source character is the fill
                    if let Some(ch) = self.lexer.next_literal_char() {
                        builder.add_part(FormatPart::Fill(ch));
                    }
                    self.advance()?;
                }
                Token::Underscore => {
                    // The width character is one source character, not a parsed date token.
                    if let Some(ch) = self.lexer.next_literal_char() {
                        builder.add_part(FormatPart::Skip(ch));
                    }
                    self.advance()?;
                }

                // Scientific notation - but only if followed by + or -
                // Otherwise, check if it's an era year (e/E patterns like 'e', 'ee', 'eee')
                Token::ExponentUpper | Token::ExponentLower => {
                    let is_lower = matches!(self.current.token, Token::ExponentLower);
                    self.advance()?;

                    // Check if followed by + or - (scientific notation) or just a literal/date part
                    if matches!(self.current.token, Token::Plus | Token::Minus) {
                        let show_plus = matches!(self.current.token, Token::Plus);
                        self.advance()?;
                        let upper = !is_lower;
                        builder.add_part(FormatPart::Scientific { upper, show_plus });
                    } else {
                        // Standalone 'e' or 'E' - could be era year (date format)
                        // Skip consecutive e/E tokens
                        while matches!(
                            self.current.token,
                            Token::ExponentLower | Token::ExponentUpper
                        ) {
                            self.advance()?;
                        }

                        // Era year format: e, ee, eee, eeee all output 4-digit year
                        // For Gregorian calendar, era year is same as regular year
                        // Excel always shows the full year for 'e' format
                        builder.add_part(FormatPart::DatePart(DatePart::Year4));
                    }
                }

                // Signs become literals in format context (when not part of scientific notation)
                Token::Plus => {
                    builder.add_part(FormatPart::Literal("+".to_string()));
                    self.advance()?;
                }
                Token::Minus => {
                    builder.add_part(FormatPart::Literal("-".to_string()));
                    self.advance()?;
                }

                // Fraction
                Token::Slash => {
                    builder.add_part(FormatPart::Literal("/".to_string()));
                    self.advance()?;
                }

                // Date/time tokens
                Token::Year => {
                    let count = self.count_consecutive(&Token::Year)?;
                    let part = if count >= 4 {
                        DatePart::Year4
                    } else if count == 3 {
                        DatePart::Year3
                    } else {
                        DatePart::Year2
                    };
                    builder.add_part(FormatPart::DatePart(part));
                }
                Token::Month => {
                    // Check if this should be minute (after hour) or month
                    // BEFORE consuming tokens, check if seconds follow
                    let has_seconds_following = self.has_seconds_ahead();
                    let count = self.count_consecutive(&Token::Month)?;
                    // It's a minute if:
                    // 1. We've seen an hour token, OR
                    // 2. There are seconds tokens following (mm:ss pattern)
                    let part = if self.seen_hour || has_seconds_following {
                        // This is minute
                        if count >= 2 {
                            DatePart::Minute2
                        } else {
                            DatePart::Minute
                        }
                    } else {
                        // This is month
                        match count {
                            1 => DatePart::Month,
                            2 => DatePart::Month2,
                            3 => DatePart::MonthAbbr,
                            4 => DatePart::MonthFull,
                            _ => DatePart::MonthLetter,
                        }
                    };
                    builder.add_part(FormatPart::DatePart(part));
                }
                Token::Day => {
                    let count = self.count_consecutive(&Token::Day)?;
                    let part = match count {
                        1 => DatePart::Day,
                        2 => DatePart::Day2,
                        3 => DatePart::DayAbbr,
                        _ => DatePart::DayFull,
                    };
                    builder.add_part(FormatPart::DatePart(part));
                }
                Token::Hour => {
                    self.seen_hour = true;
                    let count = self.count_consecutive(&Token::Hour)?;
                    let part = if count >= 2 {
                        DatePart::Hour2
                    } else {
                        DatePart::Hour
                    };
                    builder.add_part(FormatPart::DatePart(part));

                    // Check for fractional hours (.0, .00, .000, etc.)
                    if matches!(self.current.token, Token::DecimalPoint) {
                        self.advance()?;
                        // Count consecutive zeros after decimal point
                        let mut frac_places = 0;
                        while matches!(self.current.token, Token::Zero) {
                            frac_places += 1;
                            self.advance()?;
                        }
                        if frac_places > 0 {
                            // Add decimal point as literal
                            builder.add_part(FormatPart::Literal(".".to_string()));
                            // Treat as subsecond for now (fractional time)
                            builder.add_part(FormatPart::DatePart(DatePart::SubSecond(
                                subsecond_precision(frac_places)?,
                            )));
                        }
                    }
                }
                Token::Second => {
                    let count = self.count_consecutive(&Token::Second)?;
                    let part = if count >= 2 {
                        DatePart::Second2
                    } else {
                        DatePart::Second
                    };
                    builder.add_part(FormatPart::DatePart(part));

                    // Check for subsecond formatting (.0, .00, .000, etc.)
                    if matches!(self.current.token, Token::DecimalPoint) {
                        self.advance()?;
                        // Count consecutive zeros after decimal point
                        let mut subsec_places = 0;
                        while matches!(self.current.token, Token::Zero) {
                            subsec_places += 1;
                            self.advance()?;
                        }
                        if subsec_places > 0 {
                            // Add decimal point as literal
                            builder.add_part(FormatPart::Literal(".".to_string()));
                            builder.add_part(FormatPart::DatePart(DatePart::SubSecond(
                                subsecond_precision(subsec_places)?,
                            )));
                        }
                    }
                }

                // Buddhist calendar
                Token::BuddhistYear => {
                    let count = self.count_consecutive(&Token::BuddhistYear)?;
                    let part = if count >= 4 {
                        DatePart::BuddhistYear4
                    } else {
                        DatePart::BuddhistYear2
                    };
                    builder.add_part(FormatPart::DatePart(part));
                }
                Token::BuddhistYearUpper => {
                    self.advance()?;
                    // Check if this is 'B2' format (alternative Buddhist calendar)
                    if matches!(self.current.token, Token::Literal('2')) {
                        self.advance()?;
                        // B2 is a prefix that modifies subsequent year formatting
                        // Check if followed by year tokens and convert them to BuddhistYear*Alt
                        if matches!(self.current.token, Token::Year) {
                            let count = self.count_consecutive(&Token::Year)?;
                            if count >= 4 {
                                // B2yyyy -> use alternative Buddhist calendar for 4-digit year
                                builder.add_part(FormatPart::DatePart(DatePart::BuddhistYear4Alt));
                            } else {
                                // B2yy -> use 2-digit alternative Buddhist year
                                builder.add_part(FormatPart::DatePart(DatePart::BuddhistYear2Alt));
                            }
                        } else {
                            // B2 not followed by year - treat as literal
                            builder.add_part(FormatPart::Literal("B2".to_string()));
                        }
                    } else {
                        // Just 'B' by itself - treat as regular Buddhist year
                        let count = 1 + self.count_consecutive(&Token::BuddhistYearUpper)?;
                        let part = if count >= 4 {
                            DatePart::BuddhistYear4
                        } else {
                            DatePart::BuddhistYear2
                        };
                        builder.add_part(FormatPart::DatePart(part));
                    }
                }

                // AM/PM
                Token::AmPm(s) => {
                    let style = parse_am_pm_style(s);
                    builder.add_part(FormatPart::AmPm(style));
                    self.advance()?;
                }

                // Literals
                Token::Literal(ch) => {
                    builder.add_part(FormatPart::Literal(ch.to_string()));
                    self.advance()?;
                }
                Token::EscapedChar(ch) => {
                    builder.add_part(FormatPart::EscapedLiteral(ch.to_string()));
                    self.advance()?;
                }
                Token::QuotedString(s) => {
                    builder.add_part(FormatPart::Literal(s.clone()));
                    self.advance()?;
                }

                Token::CloseBracket => {
                    // Unexpected close bracket - treat as literal
                    builder.add_part(FormatPart::Literal("]".to_string()));
                    self.advance()?;
                }
            }
        }

        builder.build()
    }

    /// Parse bracket content such as `[Red]`, `[>100]`, `[h]`, or `[$-409]`.
    fn parse_bracket_content(
        &mut self,
        builder: &mut SectionBuilder,
        bracket_start: usize,
    ) -> Result<(), ParseError> {
        // Collect all content until we hit the close bracket
        let mut content = String::new();

        loop {
            match &self.current.token {
                Token::CloseBracket => {
                    self.advance()?;
                    break;
                }
                Token::Eof => {
                    return Err(ParseError::UnterminatedBracket {
                        position: bracket_start,
                    });
                }
                Token::Literal(ch) => {
                    content.push(*ch);
                    self.advance()?;
                }
                // Other tokens that might appear inside brackets
                Token::Zero => {
                    content.push('0');
                    self.advance()?;
                }
                Token::Hash => {
                    content.push('#');
                    self.advance()?;
                }
                Token::Question => {
                    content.push('?');
                    self.advance()?;
                }
                Token::DecimalPoint => {
                    content.push('.');
                    self.advance()?;
                }
                Token::ThousandsSep => {
                    content.push(',');
                    self.advance()?;
                }
                Token::Percent => {
                    content.push('%');
                    self.advance()?;
                }
                Token::At => {
                    content.push('@');
                    self.advance()?;
                }
                Token::Asterisk => {
                    content.push('*');
                    self.advance()?;
                }
                Token::Underscore => {
                    content.push('_');
                    self.advance()?;
                }
                Token::Plus => {
                    content.push('+');
                    self.advance()?;
                }
                Token::Minus => {
                    content.push('-');
                    self.advance()?;
                }
                Token::Slash => {
                    content.push('/');
                    self.advance()?;
                }
                Token::ExponentUpper => {
                    content.push('E');
                    self.advance()?;
                }
                Token::ExponentLower => {
                    content.push('e');
                    self.advance()?;
                }
                _ => {
                    // Skip other tokens inside brackets
                    self.advance()?;
                }
            }
        }

        // Now parse the bracket content
        let content = content.trim();

        // Try to parse as color
        if let Some(color) = try_parse_color(content) {
            builder.color = Some(color);
            return Ok(());
        }

        // Try to parse as condition
        if let Some(condition) = try_parse_condition(content) {
            builder.condition = Some(condition);
            return Ok(());
        }

        // Try to parse as elapsed time
        if let Some(elapsed) = try_parse_elapsed(content) {
            builder.add_part(FormatPart::Elapsed(elapsed));
            // If this is elapsed hours, set seen_hour so that subsequent 'mm' is parsed as minutes
            if matches!(elapsed, ElapsedPart::Hours | ElapsedPart::Hours2) {
                self.seen_hour = true;
            }
            return Ok(());
        }

        // Try to parse as locale code
        if let Some(locale) = try_parse_locale(content) {
            builder.add_part(FormatPart::Locale(locale));
            return Ok(());
        }

        // Unknown bracket content - treat as literal (or ignore)
        Ok(())
    }

    /// Count consecutive tokens of the same type and advance past them.
    fn count_consecutive(&mut self, token_type: &Token) -> Result<usize, ParseError> {
        let mut count = 0;
        while self.token_matches(token_type) {
            count += 1;
            self.advance()?;
        }
        Ok(count)
    }

    /// Check if current token matches the given token type (ignoring content).
    fn token_matches(&self, token_type: &Token) -> bool {
        std::mem::discriminant(&self.current.token) == std::mem::discriminant(token_type)
    }

    /// Check if there are seconds tokens appearing immediately after in a time context.
    /// Used to disambiguate mm:ss (minutes:seconds) from mm-dd (month-day).
    /// Returns true if 's' or 'S' appears next (with or without colon), indicating time format.
    /// Note: Called while current token is a Month token, so need to skip past 'm'/'M' chars.
    fn has_seconds_ahead(&self) -> bool {
        // Look ahead in the lexer's remaining input, starting from current position
        let mut remaining = &self.lexer.input[self.current.start..];

        // Skip past 'm' or 'M' characters (the month/minute tokens we're currently at)
        remaining = remaining.trim_start_matches(['m', 'M']);
        remaining = remaining.trim_start();

        // Check for time context patterns:
        // 1. ":s" or ":ss" - minutes:seconds with colon (e.g., "mm:ss")
        // 2. "s" or "ss" - minutes+seconds without colon (e.g., "mmss.0")
        if let Some(after_colon) = remaining.strip_prefix(':') {
            // Check if 's' or 'S' follows the colon
            if let Some(first_ch) = after_colon.chars().next() {
                return first_ch == 's' || first_ch == 'S';
            }
        } else if let Some(first_ch) = remaining.chars().next() {
            // Check if 's' or 'S' follows immediately (without colon)
            return first_ch == 's' || first_ch == 'S';
        }

        false
    }
}

/// Helper struct for building sections.
struct SectionBuilder {
    condition: Option<Condition>,
    color: Option<Color>,
    parts: Vec<FormatPart>,
}

/// A complete parser-side fraction match expressed in source-part indices.
struct FractionMatch {
    integer_indices: Vec<usize>,
    numerator_indices: Vec<usize>,
    slash_index: usize,
    denominator: MatchedDenominator,
}

/// Denominator syntax retained while a complete fraction candidate is validated.
enum MatchedDenominator {
    Variable(Vec<usize>),
    Fixed(Vec<(usize, u8)>),
}

impl SectionBuilder {
    fn new() -> Self {
        Self {
            condition: None,
            color: None,
            parts: Vec::new(),
        }
    }

    /// Add a normalized part, replacing any earlier fill directive.
    fn add_part(&mut self, part: FormatPart) {
        // The semantic AST retains at most one fill. A later directive replaces
        // the earlier node while preserving the later directive's position.
        if matches!(&part, FormatPart::Fill(_)) {
            self.parts
                .retain(|part| !matches!(part, FormatPart::Fill(_)));
        }
        self.parts.push(part);
    }

    fn build(mut self) -> Result<Section, ParseError> {
        // Post-process to detect fraction patterns
        self.detect_fractions();

        // Post-process to detect subsecond patterns in date formats
        self.detect_subseconds()?;

        Ok(Section::new(self.condition, self.color, self.parts))
    }

    /// Tag a complete fraction pattern without rebuilding the source-ordered parts.
    fn detect_fractions(&mut self) {
        let Some(matched) =
            (0..self.parts.len()).find_map(|slash_index| self.analyze_fraction_at(slash_index))
        else {
            return;
        };

        for index in matched.integer_indices {
            let FormatPart::Digit(placeholder) = self.parts[index] else {
                unreachable!("fraction analysis records only digit placeholders");
            };
            self.parts[index] = FormatPart::Fraction(FractionPart::IntegerDigit(placeholder));
        }
        for index in matched.numerator_indices {
            let FormatPart::Digit(placeholder) = self.parts[index] else {
                unreachable!("fraction analysis records only digit placeholders");
            };
            self.parts[index] = FormatPart::Fraction(FractionPart::NumeratorDigit(placeholder));
        }
        self.parts[matched.slash_index] = FormatPart::Fraction(FractionPart::Slash);
        match matched.denominator {
            MatchedDenominator::Variable(indices) => {
                for index in indices {
                    let FormatPart::Digit(placeholder) = self.parts[index] else {
                        unreachable!("variable denominators contain only placeholders");
                    };
                    self.parts[index] =
                        FormatPart::Fraction(FractionPart::DenominatorDigit(placeholder));
                }
            }
            MatchedDenominator::Fixed(digits) => {
                for (index, digit) in digits {
                    self.parts[index] =
                        FormatPart::Fraction(FractionPart::FixedDenominatorDigit(digit));
                }
            }
        }
    }

    /// Analyze one slash candidate and return only after the full fraction is valid.
    fn analyze_fraction_at(&self, slash_index: usize) -> Option<FractionMatch> {
        if !matches!(self.parts.get(slash_index), Some(FormatPart::Literal(text) | FormatPart::EscapedLiteral(text)) if text == "/")
        {
            return None;
        }

        let denominator_start = (slash_index + 1..self.parts.len())
            .find(|index| !is_fraction_layout_anchor(&self.parts[*index]))?;
        let denominator = match &self.parts[denominator_start] {
            FormatPart::Digit(_) => {
                let indices = self.collect_variable_denominator(denominator_start);
                (!indices.is_empty()).then_some(MatchedDenominator::Variable(indices))?
            }
            part if fixed_denominator_digit(part).is_some() => {
                MatchedDenominator::Fixed(self.collect_fixed_denominator(denominator_start)?)
            }
            _ => return None,
        };

        let mut pre_slash_digits = Vec::new();
        let mut index = slash_index;
        while index > 0 {
            index -= 1;
            match &self.parts[index] {
                FormatPart::Digit(_) => pre_slash_digits.push(index),
                part if is_fraction_layout_anchor(part) => {}
                _ => break,
            }
        }
        if pre_slash_digits.is_empty() {
            return None;
        }
        pre_slash_digits.reverse();

        let separator = (pre_slash_digits[0]..*pre_slash_digits.last()?).find(|candidate| {
            is_literal_space(&self.parts[*candidate])
                && pre_slash_digits.iter().any(|index| index < candidate)
                && pre_slash_digits.iter().any(|index| index > candidate)
        });
        let (integer_indices, numerator_indices) = if let Some(separator) = separator {
            pre_slash_digits
                .into_iter()
                .partition(|index| *index < separator)
        } else {
            (Vec::new(), pre_slash_digits)
        };

        Some(FractionMatch {
            integer_indices,
            numerator_indices,
            slash_index,
            denominator,
        })
    }

    /// Collect a variable denominator across retained layout anchors.
    fn collect_variable_denominator(&self, start: usize) -> Vec<usize> {
        let mut indices = Vec::new();
        for index in start..self.parts.len() {
            match &self.parts[index] {
                FormatPart::Digit(_) => indices.push(index),
                part if is_fraction_layout_anchor(part) => {}
                _ => break,
            }
        }
        indices
    }

    /// Collect and validate every fixed denominator digit atomically.
    fn collect_fixed_denominator(&self, start: usize) -> Option<Vec<(usize, u8)>> {
        let mut digits = Vec::new();
        let mut value = 0_u32;
        for index in start..self.parts.len() {
            if let Some(digit) = fixed_denominator_digit(&self.parts[index]) {
                value = value.checked_mul(10)?.checked_add(u32::from(digit))?;
                digits.push((index, digit));
            } else if !is_fraction_layout_anchor(&self.parts[index]) {
                break;
            }
        }
        (!digits.is_empty()).then_some(digits)
    }

    /// Detect and convert subsecond patterns in date formats.
    /// Looks for DecimalPoint followed by Digit(Zero) placeholders after date/time parts
    /// and converts them to Literal(".") + DatePart::SubSecond(n).
    fn detect_subseconds(&mut self) -> Result<(), ParseError> {
        let mut new_parts = Vec::new();
        let mut i = 0;

        while i < self.parts.len() {
            // Check if current part is a DecimalPoint
            if matches!(&self.parts[i], FormatPart::DecimalPoint) {
                // Check if there are consecutive Zero digit placeholders after it
                let mut zero_count = 0;
                let mut j = i + 1;
                while j < self.parts.len()
                    && matches!(&self.parts[j], FormatPart::Digit(DigitPlaceholder::Zero))
                {
                    zero_count += 1;
                    j += 1;
                }

                // If we found zeros after the decimal point, check if there are date/time parts before
                if zero_count > 0 {
                    let has_date_parts = new_parts.iter().any(|p| {
                        matches!(
                            p,
                            FormatPart::DatePart(_) | FormatPart::AmPm(_) | FormatPart::Elapsed(_)
                        )
                    });

                    if has_date_parts {
                        // Convert to subsecond formatting
                        new_parts.push(FormatPart::Literal(".".to_string()));
                        new_parts.push(FormatPart::DatePart(DatePart::SubSecond(
                            subsecond_precision(zero_count)?,
                        )));
                        i = j; // Skip past the decimal point and zeros
                        continue;
                    }
                }
            }

            // Not a subsecond pattern, keep the part as-is
            new_parts.push(self.parts[i].clone());
            i += 1;
        }

        self.parts = new_parts;
        Ok(())
    }
}

// u32 date-renderer powers are defined only through 10^9.
fn subsecond_precision(places: usize) -> Result<u8, ParseError> {
    if places > 9 {
        return Err(ParseError::UnsupportedPrecision { places });
    }
    Ok(places as u8)
}

/// Return whether a source part may sit between fraction semantic atoms.
fn is_fraction_layout_anchor(part: &FormatPart) -> bool {
    matches!(part, FormatPart::Fill(_) | FormatPart::Skip(_)) || is_literal_space(part)
}

/// Return whether a part is literal whitespace retained in source order.
fn is_literal_space(part: &FormatPart) -> bool {
    matches!(part, FormatPart::Literal(text) | FormatPart::EscapedLiteral(text) if !text.is_empty() && text.chars().all(char::is_whitespace))
}

/// Extract one fixed-denominator source digit, including a parsed `0` token.
fn fixed_denominator_digit(part: &FormatPart) -> Option<u8> {
    match part {
        FormatPart::Literal(text) | FormatPart::EscapedLiteral(text) => {
            let mut characters = text.chars();
            let character = characters.next()?;
            if characters.next().is_none() && character.is_ascii_digit() {
                Some(character as u8 - b'0')
            } else {
                None
            }
        }
        FormatPart::Digit(DigitPlaceholder::Zero) => Some(0),
        _ => None,
    }
}

/// Parse AM/PM style from the matched string.
fn parse_am_pm_style(s: &str) -> AmPmStyle {
    match s {
        "AM/PM" => AmPmStyle::Upper,
        "am/pm" => AmPmStyle::Lower,
        "AM/P" => AmPmStyle::MalformedUpper,
        "am/p" => AmPmStyle::MalformedLower,
        "A/P" => AmPmStyle::ShortUpper,
        "a/p" => AmPmStyle::ShortLower,
        // Default to upper for mixed case
        _ => {
            if s.len() == 4 {
                // 4-char patterns like "Am/P" - treat as malformed
                if s.chars().next().map(|c| c.is_uppercase()).unwrap_or(false) {
                    AmPmStyle::MalformedUpper
                } else {
                    AmPmStyle::MalformedLower
                }
            } else if s.len() <= 3 {
                if s.chars().next().map(|c| c.is_uppercase()).unwrap_or(false) {
                    AmPmStyle::ShortUpper
                } else {
                    AmPmStyle::ShortLower
                }
            } else if s.chars().next().map(|c| c.is_uppercase()).unwrap_or(false) {
                AmPmStyle::Upper
            } else {
                AmPmStyle::Lower
            }
        }
    }
}

/// Try to parse bracket content as a color.
fn try_parse_color(content: &str) -> Option<Color> {
    // Check for named colors
    if let Ok(named) = content.parse::<NamedColor>() {
        return Some(Color::Named(named));
    }

    // Check for indexed colors: Color1 through Color56
    let lower = content.to_lowercase();
    if lower.starts_with("color") {
        if let Ok(index) = content[5..].parse::<u8>() {
            if (1..=56).contains(&index) {
                return Some(Color::Indexed(index));
            }
        }
    }

    None
}

/// Try to parse bracket content as a condition.
fn try_parse_condition(content: &str) -> Option<Condition> {
    let content = content.trim();

    // Parse conditions like >=, <=, <>, >, <, =
    if let Some(value_str) = content.strip_prefix(">=") {
        if let Ok(value) = value_str.trim().parse::<f64>() {
            return Some(Condition::GreaterOrEqual(value));
        }
    } else if let Some(value_str) = content.strip_prefix("<=") {
        if let Ok(value) = value_str.trim().parse::<f64>() {
            return Some(Condition::LessOrEqual(value));
        }
    } else if let Some(value_str) = content.strip_prefix("<>") {
        if let Ok(value) = value_str.trim().parse::<f64>() {
            return Some(Condition::NotEqual(value));
        }
    } else if let Some(value_str) = content.strip_prefix('>') {
        if let Ok(value) = value_str.trim().parse::<f64>() {
            return Some(Condition::GreaterThan(value));
        }
    } else if let Some(value_str) = content.strip_prefix('<') {
        if let Ok(value) = value_str.trim().parse::<f64>() {
            return Some(Condition::LessThan(value));
        }
    } else if let Some(value_str) = content.strip_prefix('=') {
        if let Ok(value) = value_str.trim().parse::<f64>() {
            return Some(Condition::Equal(value));
        }
    }

    None
}

/// Try to parse bracket content as elapsed time.
fn try_parse_elapsed(content: &str) -> Option<ElapsedPart> {
    let lower = content.to_lowercase();
    match lower.as_str() {
        "h" => Some(ElapsedPart::Hours),
        "hh" => Some(ElapsedPart::Hours2),
        "m" => Some(ElapsedPart::Minutes),
        "mm" => Some(ElapsedPart::Minutes2),
        "s" => Some(ElapsedPart::Seconds),
        "ss" => Some(ElapsedPart::Seconds2),
        _ => None,
    }
}

/// Try to parse bracket content as a locale code.
fn try_parse_locale(content: &str) -> Option<LocaleCode> {
    // Locale codes start with $ e.g., [$-409], [$€-407]
    if !content.starts_with('$') {
        return None;
    }

    let rest = &content[1..];

    // Parse [$currency-lcid] format
    if let Some(dash_pos) = rest.find('-') {
        let currency_part = &rest[..dash_pos];
        let lcid_part = &rest[dash_pos + 1..];

        let currency = if currency_part.is_empty() {
            None
        } else {
            Some(currency_part.to_string())
        };

        let lcid = u32::from_str_radix(lcid_part, 16).ok();

        Some(LocaleCode { currency, lcid })
    } else {
        // Just a currency symbol
        Some(LocaleCode {
            currency: if rest.is_empty() {
                None
            } else {
                Some(rest.to_string())
            },
            lcid: None,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_parse_empty() {
        let result = parse("");
        assert!(matches!(result, Err(ParseError::EmptyFormat)));
    }

    #[test]
    fn test_parse_single_zero() {
        let fmt = parse("0").unwrap();
        assert_eq!(fmt.sections().len(), 1);
        assert_eq!(fmt.sections()[0].parts().len(), 1);
    }

    #[test]
    fn test_try_parse_color_named() {
        assert!(matches!(
            try_parse_color("Red"),
            Some(Color::Named(NamedColor::Red))
        ));
        assert!(matches!(
            try_parse_color("blue"),
            Some(Color::Named(NamedColor::Blue))
        ));
    }

    #[test]
    fn test_try_parse_color_indexed() {
        assert!(matches!(try_parse_color("Color1"), Some(Color::Indexed(1))));
        assert!(matches!(
            try_parse_color("Color56"),
            Some(Color::Indexed(56))
        ));
        assert!(try_parse_color("Color0").is_none());
        assert!(try_parse_color("Color57").is_none());
    }

    #[test]
    fn test_try_parse_condition() {
        assert!(matches!(
            try_parse_condition(">100"),
            Some(Condition::GreaterThan(n)) if (n - 100.0).abs() < f64::EPSILON
        ));
        assert!(matches!(
            try_parse_condition("<0"),
            Some(Condition::LessThan(n)) if n.abs() < f64::EPSILON
        ));
        assert!(matches!(
            try_parse_condition(">=50"),
            Some(Condition::GreaterOrEqual(n)) if (n - 50.0).abs() < f64::EPSILON
        ));
        assert!(matches!(
            try_parse_condition("<=10"),
            Some(Condition::LessOrEqual(n)) if (n - 10.0).abs() < f64::EPSILON
        ));
        assert!(matches!(
            try_parse_condition("=5"),
            Some(Condition::Equal(n)) if (n - 5.0).abs() < f64::EPSILON
        ));
        assert!(matches!(
            try_parse_condition("<>0"),
            Some(Condition::NotEqual(n)) if n.abs() < f64::EPSILON
        ));
    }

    #[test]
    fn test_try_parse_elapsed() {
        assert!(matches!(try_parse_elapsed("h"), Some(ElapsedPart::Hours)));
        assert!(matches!(try_parse_elapsed("hh"), Some(ElapsedPart::Hours2)));
        assert!(matches!(try_parse_elapsed("m"), Some(ElapsedPart::Minutes)));
        assert!(matches!(
            try_parse_elapsed("mm"),
            Some(ElapsedPart::Minutes2)
        ));
        assert!(matches!(try_parse_elapsed("s"), Some(ElapsedPart::Seconds)));
        assert!(matches!(
            try_parse_elapsed("ss"),
            Some(ElapsedPart::Seconds2)
        ));
    }

    #[test]
    fn test_try_parse_locale() {
        let locale = try_parse_locale("$-409").unwrap();
        assert!(locale.currency.is_none());
        assert_eq!(locale.lcid, Some(0x409));

        let locale = try_parse_locale("$€-407").unwrap();
        assert_eq!(locale.currency, Some("€".to_string()));
        assert_eq!(locale.lcid, Some(0x407));

        let locale = try_parse_locale("$$").unwrap();
        assert_eq!(locale.currency, Some("$".to_string()));
        assert!(locale.lcid.is_none());
    }
}
