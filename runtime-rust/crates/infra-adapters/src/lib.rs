//! Native host adapters shared by the standalone engine and orchestration.
//! Dependencies point downwards into core/host/tooling/generator.
pub mod bank;
pub mod counters;
pub mod dispatch;
pub mod generator_gate;
pub mod hook_policy;
pub mod hooks;
pub mod output;
pub mod process;
pub mod shell;

/// Trailing-only string operations over the exact original Unicode text.
/// Repeated suffix removal uses the standard literal strip operation.
pub trait TailPattern {
    fn strip<'a>(&self, value: &'a str) -> Option<&'a str>;
}
impl TailPattern for char {
    fn strip<'a>(&self, value: &'a str) -> Option<&'a str> {
        value.strip_suffix(*self)
    }
}
impl TailPattern for &str {
    fn strip<'a>(&self, value: &'a str) -> Option<&'a str> {
        if self.is_empty() {
            None
        } else {
            value.strip_suffix(*self)
        }
    }
}
pub trait TailTrim {
    fn tail(&self) -> &str;
    fn tail_matches<P: TailPattern>(&self, pattern: P) -> &str;
}
impl TailTrim for str {
    fn tail(&self) -> &str {
        let end = self
            .char_indices()
            .rev()
            .find(|(_, character)| !character.is_whitespace())
            .map(|(at, character)| at + character.len_utf8())
            .unwrap_or(0);
        &self[..end]
    }
    fn tail_matches<P: TailPattern>(&self, pattern: P) -> &str {
        let mut value = self;
        while let Some(prefix) = pattern.strip(value) {
            value = prefix;
        }
        value
    }
}
