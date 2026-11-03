//! Text-value evaluation into unresolved render parts.

use crate::ast::FormatPart;
use crate::compile::{Operation, SectionPlan};

use super::render::{self, RenderPart};

/// Evaluate one selected text plan without resolving fill or skip directives.
pub(super) fn evaluate_text(plan: &SectionPlan, value: &str) -> Vec<RenderPart> {
    let Some(anchor) = plan.text.and_then(|spec| spec.first_line_fill) else {
        return evaluate_text_in_source_order(plan, value);
    };
    let Some(line_ending_start) = first_line_ending_start(value) else {
        return evaluate_text_in_source_order(plan, value);
    };
    let Some(Operation::Fill(fill_character)) = plan.operations.get(anchor.fill_index) else {
        return evaluate_text_in_source_order(plan, value);
    };

    let mut output = Vec::new();
    let mut evaluate_semantic = |_: usize, part: &FormatPart| match part {
        FormatPart::TextPlaceholder => Some(value.to_string()),
        _ => None,
    };

    for (operation_index, operation) in plan.operations.iter().enumerate() {
        if operation_index == anchor.placeholder_index {
            render::push_text(&mut output, &value[..line_ending_start]);
            output.push(RenderPart::Fill(*fill_character));
            render::push_text(&mut output, &value[line_ending_start..]);
        } else if operation_index != anchor.fill_index {
            super::evaluate_operation(
                &mut output,
                operation_index,
                operation,
                &mut evaluate_semantic,
            );
        }
    }

    output
}

/// Evaluate text placeholders without applying a multiline fill anchor.
fn evaluate_text_in_source_order(plan: &SectionPlan, value: &str) -> Vec<RenderPart> {
    super::evaluate_operations(plan, |_, part| match part {
        FormatPart::TextPlaceholder => Some(value.to_string()),
        _ => None,
    })
}

/// Return the byte index before the first LF or complete CRLF line ending.
fn first_line_ending_start(value: &str) -> Option<usize> {
    let line_feed_index = value.find('\n')?;
    Some(
        if line_feed_index > 0 && value.as_bytes()[line_feed_index - 1] == b'\r' {
            line_feed_index - 1
        } else {
            line_feed_index
        },
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::NumberFormat;

    /// Return the sole compiled plan for a parsed format.
    fn plan(code: &str) -> SectionPlan {
        NumberFormat::parse(code).unwrap().compiled.sections[0].clone()
    }

    #[test]
    fn emits_first_line_fill_as_ordered_unresolved_parts() {
        assert_eq!(
            evaluate_text(&plan("@\"!\"*-"), "first\nsecond"),
            vec![
                RenderPart::Text("first".to_string()),
                RenderPart::Fill('-'),
                RenderPart::Text("\nsecond!".to_string()),
            ]
        );
        assert_eq!(
            evaluate_text(&plan("@*-"), "first\r\nsecond"),
            vec![
                RenderPart::Text("first".to_string()),
                RenderPart::Fill('-'),
                RenderPart::Text("\r\nsecond".to_string()),
            ]
        );
        assert_eq!(
            evaluate_text(&plan("0@*-"), "first\nsecond"),
            vec![
                RenderPart::Text("first".to_string()),
                RenderPart::Fill('-'),
                RenderPart::Text("\nsecond".to_string()),
            ]
        );
    }

    #[test]
    fn preserves_source_order_without_an_applicable_multiline_anchor() {
        assert_eq!(
            evaluate_text(&plan("*-@"), "first\nsecond"),
            vec![
                RenderPart::Fill('-'),
                RenderPart::Text("first\nsecond".to_string()),
            ]
        );
        assert_eq!(
            evaluate_text(&plan("@\"!\"*-"), "first"),
            vec![
                RenderPart::Text("first!".to_string()),
                RenderPart::Fill('-'),
            ]
        );
        assert_eq!(
            evaluate_text(&plan("@@*-"), "a\nb"),
            vec![
                RenderPart::Text("a\nba\nb".to_string()),
                RenderPart::Fill('-'),
            ]
        );
    }
}
