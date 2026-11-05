//! Where Python's `json` module stops on a text, and what it says there.
//!
//! The native parser decides what a document is; this only words a refusal
//! the way the retired runtime worded it, so an agent correcting a save in
//! its one retry reads the same position and message. It follows CPython's
//! C scanner (3.13 and later, which names a trailing comma): positions are
//! character indices, lines and columns are counted as `JSONDecodeError`
//! counts them.

/// The first error `json.loads` reports for `text`: its message (with the
/// module's own trailing " at" where it has one) and its line and column.
/// `None` when the module would accept the text.
pub fn first_problem(text: &str) -> Option<(String, usize, usize)> {
    let characters = text.chars().collect::<Vec<_>>();
    let scanner = Scanner { text: &characters };
    let problem = if characters.first() == Some(&'\u{feff}') {
        Some((
            "Unexpected UTF-8 BOM (decode using utf-8-sig)".to_string(),
            0,
        ))
    } else {
        let start = scanner.whitespace(0);
        match scanner.value(start, 0) {
            Err(problem) => Some(problem),
            Ok(end) => {
                let end = scanner.whitespace(end);
                (end != characters.len()).then(|| ("Extra data".to_string(), end))
            }
        }
    }?;
    let (message, position) = problem;
    let before = &characters[..position.min(characters.len())];
    let line = before.iter().filter(|c| **c == '\n').count() + 1;
    let column = match before.iter().rposition(|c| *c == '\n') {
        Some(newline) => position - newline,
        None => position + 1,
    };
    Some((message, line, column))
}

/// Deeper than this, the module itself gives up (RecursionError); the caller
/// then has no position to name.
const DEPTH_LIMIT: usize = 512;

struct Scanner<'a> {
    text: &'a [char],
}
type Scan = std::result::Result<usize, (String, usize)>;

impl Scanner<'_> {
    fn at(&self, index: usize) -> Option<char> {
        self.text.get(index).copied()
    }
    fn whitespace(&self, mut index: usize) -> usize {
        while matches!(self.at(index), Some(' ' | '\t' | '\n' | '\r')) {
            index += 1;
        }
        index
    }
    fn word(&self, index: usize, word: &str) -> bool {
        word.chars()
            .enumerate()
            .all(|(offset, expected)| self.at(index + offset) == Some(expected))
    }
    fn digits(&self, mut index: usize) -> usize {
        while self.at(index).is_some_and(|c| c.is_ascii_digit()) {
            index += 1;
        }
        index
    }
    /// `-?(0|[1-9][0-9]*)(\.[0-9]+)?([eE][-+]?[0-9]+)?` from `index`.
    fn number(&self, index: usize) -> Option<usize> {
        let mut end = index;
        if self.at(end) == Some('-') {
            end += 1;
        }
        match self.at(end) {
            Some('0') => end += 1,
            Some('1'..='9') => end = self.digits(end + 1),
            _ => return None,
        }
        if self.at(end) == Some('.') && self.at(end + 1).is_some_and(|c| c.is_ascii_digit()) {
            end = self.digits(end + 1);
        }
        if matches!(self.at(end), Some('e' | 'E')) {
            let mut exponent = end + 1;
            if matches!(self.at(exponent), Some('+' | '-')) {
                exponent += 1;
            }
            if self.at(exponent).is_some_and(|c| c.is_ascii_digit()) {
                end = self.digits(exponent);
            }
        }
        Some(end)
    }
    fn value(&self, index: usize, depth: usize) -> Scan {
        let expecting = || Err(("Expecting value".to_string(), index));
        if depth > DEPTH_LIMIT {
            return Err(("RecursionError".to_string(), index));
        }
        match self.at(index) {
            None => expecting(),
            Some('"') => self.string(index),
            Some('{') => self.object(index + 1, depth + 1),
            Some('[') => self.array(index + 1, depth + 1),
            Some('n') if self.word(index, "null") => Ok(index + 4),
            Some('t') if self.word(index, "true") => Ok(index + 4),
            Some('f') if self.word(index, "false") => Ok(index + 5),
            Some('N') if self.word(index, "NaN") => Ok(index + 3),
            Some('I') if self.word(index, "Infinity") => Ok(index + 8),
            Some('-') if self.word(index, "-Infinity") => Ok(index + 9),
            Some(_) => self.number(index).map_or_else(expecting, Ok),
        }
    }
    /// A string whose opening quote is at `quote`.
    fn string(&self, quote: usize) -> Scan {
        let unterminated = || Err(("Unterminated string starting at".to_string(), quote));
        let mut index = quote + 1;
        loop {
            match self.at(index) {
                None => return unterminated(),
                Some('"') => return Ok(index + 1),
                Some('\\') => match self.at(index + 1) {
                    None => return unterminated(),
                    Some('u') => {
                        if !(index + 2..index + 6)
                            .all(|k| self.at(k).is_some_and(|c| c.is_ascii_hexdigit()))
                        {
                            return Err(("Invalid \\uXXXX escape".to_string(), index + 1));
                        }
                        index += 6;
                    }
                    Some('"' | '\\' | '/' | 'b' | 'f' | 'n' | 'r' | 't') => index += 2,
                    Some(_) => return Err(("Invalid \\escape".to_string(), index)),
                },
                Some(c) if (c as u32) < 0x20 => {
                    return Err(("Invalid control character at".to_string(), index));
                }
                Some(_) => index += 1,
            }
        }
    }
    fn object(&self, index: usize, depth: usize) -> Scan {
        let mut index = self.whitespace(index);
        if self.at(index) == Some('}') {
            return Ok(index + 1);
        }
        loop {
            if self.at(index) != Some('"') {
                return Err((
                    "Expecting property name enclosed in double quotes".to_string(),
                    index,
                ));
            }
            index = self.whitespace(self.string(index)?);
            if self.at(index) != Some(':') {
                return Err(("Expecting ':' delimiter".to_string(), index));
            }
            index = self.whitespace(self.value(self.whitespace(index + 1), depth)?);
            match self.at(index) {
                Some('}') => return Ok(index + 1),
                Some(',') => {
                    let comma = index;
                    index = self.whitespace(index + 1);
                    if self.at(index) == Some('}') {
                        return Err((
                            "Illegal trailing comma before end of object".to_string(),
                            comma,
                        ));
                    }
                }
                _ => return Err(("Expecting ',' delimiter".to_string(), index)),
            }
        }
    }
    fn array(&self, index: usize, depth: usize) -> Scan {
        let mut index = self.whitespace(index);
        if self.at(index) == Some(']') {
            return Ok(index + 1);
        }
        loop {
            index = self.whitespace(self.value(index, depth)?);
            match self.at(index) {
                Some(']') => return Ok(index + 1),
                Some(',') => {
                    let comma = index;
                    index = self.whitespace(index + 1);
                    if self.at(index) == Some(']') {
                        return Err((
                            "Illegal trailing comma before end of array".to_string(),
                            comma,
                        ));
                    }
                }
                _ => return Err(("Expecting ',' delimiter".to_string(), index)),
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::first_problem;

    /// Each case as Python 3.14's `json.loads` reported it.
    #[test]
    fn problems_are_placed_and_worded_as_python_reports_them() {
        for (text, message, line, column) in [
            (
                "{\"a\":1,}",
                "Illegal trailing comma before end of object",
                1,
                7,
            ),
            ("[1,2,]", "Illegal trailing comma before end of array", 1, 5),
            ("", "Expecting value", 1, 1),
            ("  ", "Expecting value", 1, 3),
            ("{\"a\" 1}", "Expecting ':' delimiter", 1, 6),
            (
                "{a:1}",
                "Expecting property name enclosed in double quotes",
                1,
                2,
            ),
            ("{\"a\":}", "Expecting value", 1, 6),
            ("\"abc", "Unterminated string starting at", 1, 1),
            ("{\"a\":\"x\\q\"}", "Invalid \\escape", 1, 8),
            ("{\"a\":\"\\u12\"}", "Invalid \\uXXXX escape", 1, 8),
            ("01", "Extra data", 1, 2),
            ("{\"a\":01}", "Expecting ',' delimiter", 1, 7),
            ("nul", "Expecting value", 1, 1),
            ("[1 2]", "Expecting ',' delimiter", 1, 4),
            ("{\"a\":1}}", "Extra data", 1, 8),
            (
                "\u{feff}{}",
                "Unexpected UTF-8 BOM (decode using utf-8-sig)",
                1,
                1,
            ),
            ("{\"a\":tru}", "Expecting value", 1, 6),
            (
                "{\"progress\": \"line one\nline two\"}",
                "Invalid control character at",
                1,
                23,
            ),
            (
                "{\n  \"a\": 1\n  \"b\": 2}",
                "Expecting ',' delimiter",
                3,
                3,
            ),
        ] {
            assert_eq!(
                first_problem(text),
                Some((message.to_string(), line, column)),
                "{text:?}"
            );
        }
        for accepted in [
            "{}",
            " {\"a\": [1, 2.5e3, -0, null, true, false, NaN]} ",
            "\"\\u00e9\"",
        ] {
            assert_eq!(first_problem(accepted), None, "{accepted:?}");
        }
    }
}
