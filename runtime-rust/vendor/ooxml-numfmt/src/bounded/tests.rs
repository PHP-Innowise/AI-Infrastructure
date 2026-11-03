use super::*;
use crate::ast::{DatePart, FormatPart, Section};
use crate::{DateSystem, NumberFormat};

fn render(code: &str, value: f64, date_system: DateSystem) -> String {
    let mut budget = Budget::default();
    let format = Format::parse(code, &mut budget).unwrap();
    format
        .number(
            value,
            &FormatOptions {
                date_system,
                ..FormatOptions::default()
            },
            &mut budget,
        )
        .unwrap()
}

#[test]
fn retained_ssf_numeric_and_currency_vectors_have_exact_text() {
    // These are small exact-output cases, not upstream's >50% acceptance gates.
    for (code, value, expected) in [
        ("0.00%", 0.25, "25.00%"),
        ("0.000000000000", 0.125, "0.125000000000"),
        ("0.00", 2.675, "2.68"),
        ("0.00", 1.005, "1.00"),
        ("0.00%%", 0.25, "2500.00%%"),
        ("#,##0.00", 1234.5, "1,234.50"),
        ("$#,##0.00;($#,##0.00);\"-\"", 1234.5, "$1,234.50"),
        ("$#,##0.00;($#,##0.00);\"-\"", -1234.5, "($1,234.50)"),
        ("$#,##0.00;($#,##0.00);\"-\"", 0.0, "-"),
        ("[$€-407]#,##0.00", 1234.5, "€1,234.50"),
        ("0.00E+00", 1234.0, "1.23E+03"),
        ("# ?/?", 1.5, "1 1/2"),
        ("General", 1.25, "1.25"),
    ] {
        assert_eq!(
            render(code, value, DateSystem::Date1900),
            expected,
            "{code}"
        );
    }
}

#[test]
fn both_date_epochs_include_zero_fraction_and_phantom_day() {
    for (system, serial, expected) in [
        (DateSystem::Date1900, 0.0, "1900-01-00 00:00"),
        (DateSystem::Date1900, 1.0, "1900-01-01 00:00"),
        (DateSystem::Date1900, 59.0, "1900-02-28 00:00"),
        (DateSystem::Date1900, 60.0, "1900-02-29 00:00"),
        (DateSystem::Date1900, 61.0, "1900-03-01 00:00"),
        (DateSystem::Date1904, 0.0, "1904-01-01 00:00"),
        (DateSystem::Date1904, 0.5, "1904-01-01 12:00"),
        (DateSystem::Date1904, 1.0, "1904-01-02 00:00"),
        (DateSystem::Date1904, 60.0, "1904-03-01 00:00"),
    ] {
        assert_eq!(render("yyyy-mm-dd hh:mm", serial, system), expected);
    }
    assert_eq!(
        crate::date_serial::serial_to_date(0.0, DateSystem::Date1904),
        Some((1904, 1, 1))
    );
    assert_eq!(
        crate::date_serial::serial_to_date(f64::MAX, DateSystem::Date1904),
        None
    );
}

#[test]
fn time_elapsed_and_subseconds_use_supplied_serial_values() {
    assert_eq!(render("h:mm AM/PM", 0.5, DateSystem::Date1900), "12:00 PM");
    assert_eq!(render("[h]:mm:ss", 1.5, DateSystem::Date1900), "36:00:00");
    assert_eq!(render("ss.000", 0.5, DateSystem::Date1900), "00.000");
    assert_eq!(render("yyyy-mm-dd", -1.0, DateSystem::Date1900), "");
    assert_eq!(render("yyyy-mm-dd", 2_958_466.0, DateSystem::Date1900), "");
}

#[test]
fn omitted_sections_hide_values_and_explicit_general_stays_visible() {
    for (value, expected) in [(1.0, "1"), (-1.0, ""), (0.0, "")] {
        assert_eq!(render("0;;", value, DateSystem::Date1900), expected);
    }
    let mut budget = Budget::default();
    let blank = Format::parse(";;;", &mut budget).unwrap();
    assert_eq!(
        blank
            .number(-1.0, &FormatOptions::default(), &mut budget)
            .unwrap(),
        ""
    );
    assert_eq!(
        blank
            .text("original", &FormatOptions::default(), &mut budget)
            .unwrap(),
        ""
    );
    assert_eq!(render("[Blue]General", 1.25, DateSystem::Date1900), "1.25");
}

#[test]
fn general_like_utf8_inputs_do_not_panic_or_poison_convenience_cache() {
    for code in ["]General", "é]General", "🙂]General"] {
        let _ = crate::format_default(1.0, code);
    }
    assert_eq!(crate::format_default(1.25, "0.00").unwrap(), "1.25");
}

#[test]
fn first_lexical_errors_are_not_silently_replaced_by_general() {
    for code in ["\"", "\\", "[Red"] {
        assert!(NumberFormat::parse(code).is_err(), "{code:?}");
    }
}

#[test]
fn skip_width_consumes_one_literal_including_date_letters_and_unicode() {
    for code in ["_m0", "_y0", "_界0"] {
        assert_eq!(render(code, 1.0, DateSystem::Date1900), " 1", "{code}");
    }
}

#[test]
fn subsecond_precision_is_checked_before_u8_conversion_or_pow() {
    for places in [10, 32, 256] {
        for prefix in ["ss.", "mm.", "[s]."] {
            let code = format!("{prefix}{}", "0".repeat(places));
            assert!(matches!(NumberFormat::parse(&code),
                Err(ParseError::UnsupportedPrecision { places: found }) if found == places));
        }
    }
    let forged = NumberFormat::from_sections(vec![Section::new(
        None,
        None,
        vec![FormatPart::DatePart(DatePart::SubSecond(32))],
    )])
    .unwrap();
    assert!(matches!(
        forged.try_format(0.5, &FormatOptions::default()),
        Err(FormatError::UnsupportedPrecision { places: 32 })
    ));
}

#[test]
fn malformed_many_sections_are_rejected_without_truncation() {
    assert!(matches!(
        NumberFormat::parse("0;0;0;@;0"),
        Err(ParseError::TooManySections)
    ));
    let mut budget = Budget::default();
    assert_eq!(
        Format::parse("0;0;0;@;0", &mut budget).unwrap_err().code(),
        "format-section-limit"
    );
    // Quoted, escaped and skip-width semicolons do not add sections.
    for code in ["\"a;b\"0", "\\;0", "_;0"] {
        assert!(Format::parse(code, &mut budget).is_ok(), "{code}");
    }
}

#[test]
fn repeated_text_is_refused_before_semantic_materialization() {
    let mut budget = Budget::default();
    let format = Format::parse(&"@".repeat(128), &mut budget).unwrap();
    let before = budget.remaining();
    let error = format
        .text(&"x".repeat(4096), &FormatOptions::default(), &mut budget)
        .unwrap_err();
    assert_eq!(error.code(), "format-output-limit");
    assert!(budget.remaining().0 < before.0);
    assert_eq!(budget.remaining().1, before.1);
    let small = Format::parse("@@", &mut budget).unwrap();
    assert_eq!(
        small
            .text("ab", &FormatOptions::default(), &mut budget)
            .unwrap(),
        "abab"
    );
}

#[test]
fn fill_limit_is_admitted_before_repetition_and_unicode_width_is_counted() {
    let mut budget = Budget::default();
    let format = Format::parse("0*界", &mut budget).unwrap();
    let options = FormatOptions {
        fill_count: 1 << 30,
        ..FormatOptions::default()
    };
    let before = budget.remaining();
    assert_eq!(
        format
            .number(1.0, &options, &mut budget)
            .unwrap_err()
            .code(),
        "format-fill-limit"
    );
    assert!(budget.remaining().0 < before.0);
    assert_eq!(budget.remaining().1, before.1);
    let options = FormatOptions {
        fill_count: 3,
        ..FormatOptions::default()
    };
    assert_eq!(
        format.number(1.0, &options, &mut budget).unwrap(),
        "1界界界"
    );
}

#[test]
fn unsupported_scientific_and_clipped_number_ranges_return_truthful_errors() {
    let mut budget = Budget::default();
    for (code, value) in [
        ("0", f64::MAX),
        ("0.0000000000000000", 1.25),
        ("0.0E+00%", f64::MAX),
        ("0.0000000000000000E+00", 1.25),
    ] {
        let format = Format::parse(code, &mut budget).unwrap();
        assert_eq!(
            format
                .number(value, &FormatOptions::default(), &mut budget)
                .unwrap_err()
                .code(),
            "unsupported-format",
            "{code}"
        );
    }
    let general = Format::parse("General", &mut budget).unwrap();
    assert!(general
        .number(f64::MAX, &FormatOptions::default(), &mut budget)
        .unwrap()
        .contains('E'));
    assert_eq!(
        general
            .number(f64::NAN, &FormatOptions::default(), &mut budget)
            .unwrap_err()
            .code(),
        "unsupported-format"
    );
}

#[test]
fn format_length_work_output_and_locale_bounds_are_explicit() {
    let mut budget = Budget::default();
    assert_eq!(
        Format::parse(&"0".repeat(MAX_FORMAT_BYTES + 1), &mut budget)
            .unwrap_err()
            .code(),
        "format-length-limit"
    );
    let mut no_work = Budget::with_limits(0, MAX_TOTAL_OUTPUT_BYTES);
    assert_eq!(
        Format::parse("0", &mut no_work).unwrap_err().code(),
        "format-work-limit"
    );
    let mut output = Budget::with_limits(MAX_WORK_UNITS, 2);
    let format = Format::parse("@", &mut output).unwrap();
    assert_eq!(
        format
            .text("abcd", &FormatOptions::default(), &mut output)
            .unwrap_err()
            .code(),
        "format-output-limit"
    );
    let mut options = FormatOptions::default();
    options.locale.am_string = "unsupported custom locale";
    assert_eq!(
        format.text("ok", &options, &mut budget).unwrap_err().code(),
        "unsupported-format"
    );
}

#[test]
fn original_text_is_preserved_when_no_text_section_is_present() {
    let mut budget = Budget::default();
    let format = Format::parse("0.00", &mut budget).unwrap();
    assert_eq!(
        format
            .text("00123", &FormatOptions::default(), &mut budget)
            .unwrap(),
        "00123"
    );
    assert_eq!(
        format
            .text("α\r\nβ", &FormatOptions::default(), &mut budget)
            .unwrap(),
        "α\r\nβ"
    );
}

#[test]
fn repeated_refusals_spend_preflight_work_until_only_constant_time_rejection_remains() {
    let mut budget = Budget::with_limits(80_000, MAX_TOTAL_OUTPUT_BYTES);
    let format = Format::parse(&"@".repeat(128), &mut budget).unwrap();
    let supplied = "x".repeat(4096);
    let output_before = budget.remaining().1;
    let mut exhausted = false;
    for _ in 0..64 {
        let error = format
            .text(&supplied, &FormatOptions::default(), &mut budget)
            .unwrap_err();
        assert_eq!(budget.remaining().1, output_before);
        if error.code() == "format-work-limit" {
            exhausted = true;
            break;
        }
        assert_eq!(error.code(), "format-output-limit");
    }
    assert!(exhausted);
    let mut empty_attempts = Budget::with_limits(32, MAX_TOTAL_OUTPUT_BYTES);
    assert!(matches!(
        Format::parse("", &mut empty_attempts),
        Err(Error::Parse(ParseError::EmptyFormat))
    ));
    assert_eq!(empty_attempts.remaining().0, 0);
    assert_eq!(
        Format::parse("", &mut empty_attempts).unwrap_err().code(),
        "format-work-limit"
    );
}
