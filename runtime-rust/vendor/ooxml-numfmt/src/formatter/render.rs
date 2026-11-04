//! Final materialization of unresolved semantic and layout output.

use crate::error::FormatError;

/// One evaluated output fragment before layout directives are resolved.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum RenderPart {
    /// Materialized semantic or literal text.
    Text(String),
    /// A fill directive awaiting its runtime repetition count.
    Fill(char),
    /// A width hint represented as one plain-text space.
    Skip(char),
}

/// Append text while coalescing adjacent materialized fragments.
pub(crate) fn push_text(parts: &mut Vec<RenderPart>, text: impl Into<String>) {
    let text = text.into();
    if text.is_empty() {
        return;
    }

    if let Some(RenderPart::Text(previous)) = parts.last_mut() {
        previous.push_str(&text);
    } else {
        parts.push(RenderPart::Text(text));
    }
}

/// Return the first fill directive's position in fill-free output.
pub(crate) fn fill_position(parts: &[RenderPart]) -> Option<(usize, char)> {
    let mut offset = 0_usize;

    for part in parts {
        match part {
            RenderPart::Text(text) => offset = offset.checked_add(text.len())?,
            RenderPart::Fill(character) => return Some((offset, *character)),
            RenderPart::Skip(_) => offset = offset.checked_add(1)?,
        }
    }

    None
}

/// Resolve every layout directive into one plain-text string.
pub(crate) fn resolve_layout(
    parts: &[RenderPart],
    fill_count: usize,
) -> Result<String, FormatError> {
    let capacity = parts.iter().try_fold(0_usize, |capacity, part| {
        let additional = match part {
            RenderPart::Text(text) => text.len(),
            RenderPart::Fill(character) => fill_count
                .checked_mul(character.len_utf8())
                .ok_or(FormatError::OutputTooLarge { fill_count })?,
            RenderPart::Skip(_) => 1,
        };
        capacity
            .checked_add(additional)
            .ok_or(FormatError::OutputTooLarge { fill_count })
    })?;
    let mut output = String::new();
    output
        .try_reserve(capacity)
        .map_err(|_| FormatError::OutputTooLarge { fill_count })?;

    for part in parts {
        match part {
            RenderPart::Text(text) => output.push_str(text),
            RenderPart::Fill(character) => {
                output.extend(std::iter::repeat_n(*character, fill_count));
            }
            RenderPart::Skip(_) => output.push(' '),
        }
    }

    Ok(output)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn resolves_empty_and_text_only_parts() {
        assert_eq!(resolve_layout(&[], 4).unwrap(), "");
        assert_eq!(
            resolve_layout(&[RenderPart::Text("text".to_string())], 4).unwrap(),
            "text"
        );
    }

    #[test]
    fn resolves_fill_at_every_layout_position() {
        let parts = [
            RenderPart::Fill('-'),
            RenderPart::Text("a".to_string()),
            RenderPart::Fill('é'),
            RenderPart::Text("b".to_string()),
            RenderPart::Fill('!'),
        ];

        assert_eq!(resolve_layout(&parts, 0).unwrap(), "ab");
        assert_eq!(resolve_layout(&parts, 1).unwrap(), "-aéb!");
        assert_eq!(resolve_layout(&parts, 3).unwrap(), "---aéééb!!!");
    }

    #[test]
    fn resolves_skip_as_one_ascii_space() {
        assert_eq!(resolve_layout(&[RenderPart::Skip(')')], 0).unwrap(), " ");
    }

    #[test]
    fn locates_fill_in_fill_free_utf8_output() {
        let parts = [
            RenderPart::Text("é".to_string()),
            RenderPart::Skip('界'),
            RenderPart::Text("값".to_string()),
            RenderPart::Fill('界'),
        ];

        assert_eq!(fill_position(&parts), Some((6, '界')));
    }

    #[test]
    fn coalesces_adjacent_text_without_crossing_layout_boundaries() {
        let mut parts = Vec::new();
        push_text(&mut parts, "a");
        push_text(&mut parts, "b");
        parts.push(RenderPart::Fill('.'));
        push_text(&mut parts, "c");

        assert_eq!(
            parts,
            vec![
                RenderPart::Text("ab".to_string()),
                RenderPart::Fill('.'),
                RenderPart::Text("c".to_string()),
            ]
        );
    }

    #[test]
    fn rejects_output_length_overflow() {
        let parts = [RenderPart::Fill('é')];

        assert_eq!(
            resolve_layout(&parts, usize::MAX),
            Err(FormatError::OutputTooLarge {
                fill_count: usize::MAX,
            })
        );
    }

    #[test]
    fn rejects_unreservable_output() {
        let parts = [RenderPart::Fill('x')];

        assert_eq!(
            resolve_layout(&parts, usize::MAX),
            Err(FormatError::OutputTooLarge {
                fill_count: usize::MAX,
            })
        );
    }
}
