# ooxml-numfmt

Excel-compatible ECMA-376 number format codes for Rust, inspired by [ssfmt](https://github.com/ketbra/ssfmt).

## Features

- Parse and format Excel/OOXML number format codes
- Match Excel's actual behavior, including quirks
- Support for dates, times, percentages, fractions
- Multiple format sections (positive/negative/zero/text)
- Color and conditional format detection
- Both 1900 and 1904 date systems
- Efficient compile-once, format-many pattern
- Exact caller-controlled expansion of OOXML `*x` fill directives

## Usage

```rust
use ooxml_numfmt::{format_default, NumberFormat, FormatOptions};

// Simple one-off formatting
let result = format_default(1234.56, "#,##0.00").unwrap();
assert_eq!(result, "1,234.56");

// Compile once, format many values
let fmt = NumberFormat::parse("yyyy-mm-dd").unwrap();
let opts = FormatOptions::default();
assert_eq!(fmt.format(46031.0, &opts), "2026-01-09");
```

### Fill directives

OOXML uses `*x` to repeat `x` across the available cell width. This crate does
not calculate font or cell widths; callers provide the exact repetition count.
The same parsed format can be reused with different runtime counts.

```rust
use ooxml_numfmt::{FormatOptions, NumberFormat};

let fmt = NumberFormat::parse("_($* #,##0.00_)").unwrap();
let opts = FormatOptions {
    fill_count: 3,
    ..FormatOptions::default()
};

assert_eq!(fmt.format(1234.56, &opts), " $   1,234.56 ");
```

`fill_count` counts Unicode scalar values and defaults to zero. `_x` remains a
plain-text width approximation and emits one ASCII space.

## Testing

ssfmt was inspired by SheetJS SSF, so this fork keeps its SSF-based compatibility tests. The suite covers 19.5+ million cases, with 99.99% of evaluated cases passing.

See [docs/TESTING.md](https://github.com/a4s-lab/ooxml-numfmt/blob/9d87b9daeaaa6ebc1937957de71934a8601b5b3a/docs/TESTING.md) for details.

## License

MIT OR Apache-2.0
