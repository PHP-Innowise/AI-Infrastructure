use serde_json::{Value, json};

pub const A1: &str = "PILOT_A1";
pub const A2: &str = "PILOT_A2";
pub const B: &str = "PILOT_B";
pub const A_BASE: &str = include_str!("../../fixtures/session-ab/a-baseline.rs");
pub const A_GOLD: &str = include_str!("../../fixtures/session-ab/a-gold.rs");
pub const B_BASE: &str = include_str!("../../fixtures/session-ab/b-baseline.rs");
pub const B_GOLD: &str = include_str!("../../fixtures/session-ab/b-gold.rs");
pub const A_ORACLE: &str = include_str!("../../fixtures/session-ab/a-oracle.rs");
pub const B_ORACLE: &str = include_str!("../../fixtures/session-ab/b-oracle.rs");
pub const README: &str = "Public pilot fixture. Change source.rs only; preserve this file. Rust std-only; no dependencies or network.\n";
pub const POLICIES: &[&str] = &[
    "gold",
    "no-op",
    "partial",
    "wrong-b",
    "forbidden",
    "exit-zero",
    "deadline",
];

pub fn prompt(role: &str) -> &'static str {
    match role {
        A1 => {
            "PILOT_A1\nFix compress_runs in source.rs: return consecutive (value,count) runs in input order, including the final run. Empty input returns empty; preserve i32 values. Keep expand_runs's signature for the next task. Change source.rs only; preserve README.md. Use Rust std only."
        }
        A2 => {
            "PILOT_A2\nImplement expand_runs in source.rs without regressing compress_runs. Return Err for zero counts, usize sum overflow, or expanded length exceeding limit, before allocating. Otherwise return expanded values, including empty for empty input and limit 0. Err text is unspecified. Preserve both public signatures and README.md; change source.rs only. Use Rust std only."
        }
        B => {
            "PILOT_B\nFix parse_duration in source.rs. Accept one or more ASCII decimal digits followed immediately by lowercase s, m or h. Return checked u64 seconds (multipliers 1,60,3600). Leading zeros and zero are valid. Reject signs, whitespace, decimals, other units and overflow in parsing/multiplication. Err text is unspecified. Preserve the public signature and README.md; change source.rs only. Use Rust std only."
        }
        _ => unreachable!("sealed pilot role"),
    }
}
pub fn partial_a() -> String {
    format!(
        "{}\npub fn expand_runs(_: &[(i32, usize)], _: usize) -> Result<Vec<i32>, String> {{ Err(\"not implemented\".into()) }}\n",
        A_GOLD.split("pub fn expand_runs").next().unwrap()
    )
}
pub fn expected(kind: &str) -> &'static str {
    match kind {
        "a" => {
            "C:[]\nC:[(1, 1)]\nC:[(1, 3)]\nC:[(1, 1), (2, 1), (1, 1)]\nC:[(-1, 2), (0, 3), (2, 1)]\nC:[(-2147483648, 2), (2147483647, 1)]\nE:[]\nE:[3, 3, -1]\nE:ERR\nE:ERR\nE:ERR\nR:true\n"
        }
        "b" => {
            "D:0\nD:42\nD:120\nD:3600\nD:60\nD:18446744073709551615\nD:ERR\nD:ERR\nD:ERR\nD:ERR\nD:ERR\nD:ERR\nD:ERR\nD:ERR\nD:ERR\nD:ERR\nD:ERR\nD:ERR\n"
        }
        _ => unreachable!("sealed pilot kind"),
    }
}
pub fn dataset() -> Value {
    json!({"id":"runs-duration-public-pilot-v1","split":"public-pilot","independent_streams":1,"paired_unit":"whole_stream","initial":{"a":A_BASE,"b":B_BASE,"README.md":README},"prompts":{"a1":prompt(A1),"a2":prompt(A2),"b":prompt(B)},"gold":{"a":A_GOLD,"b":B_GOLD},"oracle":{"a":{"source":A_ORACLE,"expected_stdout":expected("a")},"b":{"source":B_ORACLE,"expected_stdout":expected("b")}},"grading":"final A1+A2 artifact and B; no intermediate A1 correctness claim"})
}
