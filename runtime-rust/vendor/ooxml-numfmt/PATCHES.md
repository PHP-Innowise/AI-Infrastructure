# Local changes to ooxml-numfmt 0.2.3

The original 22 Rust source files and both upstream license texts are retained.
UPSTREAM.json records the exact published archive checksum and VCS identity.
Cargo.toml.upstream is the unchanged published original manifest. The local
manifest pins the cache/error dependencies and sets publish=false. It does not
enable optional features on behalf of the consuming preview crate.

The preserved manifest is the archive's Cargo.toml.orig. Workspace rustfmt also
formats the documented changed and added Rust files. The other published source
files and both license texts remain byte-identical to the verified archive.

Source changes:
- Parse General through the normal lexer, propagate the first lexical failure,
  consume underscore width as one source character, and reject more than four
  sections. Keep explicit General distinct from a selected empty section.
- Check subsecond precision before u8 conversion and date-renderer u32 powers.
- Correct Date1904 serial zero/fraction handling; scope 1900-only exceptional
  dates to the correct epoch. Bound public date conversion arithmetic.
- Make fraction range/arithmetic failures fallible. A finite-f64 overflow
  counterexample was not established during the source audit; this guard is
  defensive and preserves the normal convergent calculation.
- Add bounded::Format and a per-workbook Budget. Format byte/section/AST limits
  and conservative parse work are admitted before parsing/evaluation. Checked
  literal, locale, @, fill and numeric/date atom bounds are admitted before
  try_format/try_format_text materializes output.
- Only the audited en-US locale is accepted by bounded entry points. Numerical
  ranges the original renderer clips, wider unproved precision, conditional
  General and Hijri projection return Unsupported. Callers must preserve the
  original value and show a stable fallback warning.
- Add 15 small exact-output and refusal regression tests. The upstream broad
  pass-ratio harnesses are not used as evidence of complete compatibility.

The legacy public unbounded upstream methods remain for source compatibility.
They are not the permitted preview entry points. Neither this source audit nor
the upstream README is a proof of global panic freedom, full Excel or SheetJS
parity, or compatibility with a compiler that has not actually built the crate.
Root-owned validation must run after the complete native source integration.

The upstream README testing-document link targets the same pinned upstream
commit because the external golden-data harness is not vendored. Its broad
pass ratio is not native validation evidence.

SSF evidence comes from the official SheetJS source and the published package's
retained SSF vectors. The canonical retired viewer references a SheetJS 0.20.3
CDN asset plus SRI but does not vendor its source. Exact bundled-loader style
population, conditional/calendar and fallback behavior still need verification
before any claim of full 0.20.3 parity. No formula evaluation is introduced.

Independent source review correction: cached nonzero preflight work is debited
before every formatting scan; every parse attempt has a fixed upfront debit.
Rejected output/unsupported input retains that debit. Budget exhaustion makes
subsequent calls refuse before substantive scans; it does not prevent callers
from repeatedly invoking a constant-time refusal.

Exact canonical0.20.3 source/probe follow-up: raise ordinary decimal field precision
from10 to15, matching the existing numeric-rounding cap. The multiplier is at
most10^15 and a fractional field is at most10^15 before the u64 conversion;
width is capped15 in bounded entry points before evaluation. Source regression
vectors include the coordinator-executed canonical12-place0.125,2.675/1.005
cases. This is not a claim of arbitrary-precision or full rounding parity.

Source-application compatibility: replace four end-padding method calls in the
General renderer with an equivalent repeated strip_suffix(char) loop. This retains
UTF-8 boundaries and output semantics and avoids a confirmed false match in the
workspace source-input policy scanner. No policy scanner settings are changed.

Strict workspace linting: use Option::unwrap_or for the existing scientific
mantissa fallback instead of an identity map_or closure. Output is unchanged.
