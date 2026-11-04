# Unicode 16 default full case folding

The native runtime preserves the canonical Python 3.14 / UCD 16.0.0 default full, non-Turkic case-fold contract. It performs no Unicode normalization and no whitespace transformation.

Primary data: https://www.unicode.org/Public/16.0.0/ucd/CaseFolding.txt
CaseFolding-16.0.0.txt SHA-256: 6f1f9c588eb4a5c718d9e8f93b782685e5c7fec872cf05e8e6878053599e09bb
Unicode license: https://www.unicode.org/license.txt
LICENSE.txt SHA-256: e7a93b009565cfce55919a381437ac4db883e9da2126fa28b91d12732bc53d96

The runtime helper uses the Cargo-pinned unicode-casefold 0.2.0 (Unicode 9.0.0) plus a sorted 156-row Unicode 16 delta. The complete valid-scalar comparison found no removed legacy mapping requiring an identity override; the helper can represent such an override without changing its lookup algorithm. C and F rows are authoritative. S and T rows are excluded. Compiler-dependent lowercase tables are not used.

The repository integration test verifies all 1,112,064 valid Unicode scalars against the independently parsed full reference, including unlisted identity mappings. Preparation independently compiled the pinned legacy crate and proposed delta in a private standalone Rust oracle; all valid scalars also matched host Python 3.14.8 / UCD 16.0.0 with zero differences. This preparation evidence is not a production dependency.

The Unicode license is reproduced in full in LICENSE.txt. The canonical installed memory-bank/MCP.md carries the same complete notice so native artifact distributions retain associated Unicode-data attribution. Do not edit generated edition/seed copies by hand.
