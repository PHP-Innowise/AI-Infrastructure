//! The reference capsule the gate tests start from. Each test changes one
//! thing about it and names the violation that change must raise.
#![allow(dead_code)]
use infra_skillcache::package::Files;
use serde_json::{Value, json};

pub const SKILL: &str = ".claude/skills/pdf/SKILL.md";

pub fn capsule() -> Value {
    json!({
        "schema_version": 1,
        "form": "hybrid",
        "mu": {"name": "pdf-invoice-totals", "description": "Extract totals from invoice PDFs into a CSV summary"},
        "source": {"skills": [{"name": "pdf", "path": SKILL, "tree_sha256": "0".repeat(64)}]},
        "capabilities": {"network": "none", "runtimes": ["python3"]},
        "bindings": [
            {"name": "input_dir", "type": "path", "origin": "request", "description": "Folder with the invoices"},
            {"name": "currency", "type": "string", "origin": "request"}
        ],
        "programs": [
            {
                "id": "extract",
                "entry": ["python3", "programs/extract/main.py"],
                "inputs": ["input_dir", "quarter_files"],
                "outputs": ["rows_file"],
                "check": ["python3", "programs/extract/main.py", "--check"],
                "timeout_seconds": 120,
                "read_only": false
            },
            {
                "id": "summarize",
                "entry": ["python3", "programs/summarize/main.py"],
                "inputs": ["rows_file", "currency"],
                "outputs": ["summary_file"],
                "check": ["python3", "programs/summarize/main.py", "--check"],
                "timeout_seconds": 60,
                "read_only": false
            },
            {
                "id": "verify_csv",
                "entry": ["python3", "programs/verify_csv/main.py"],
                "inputs": ["summary_file", "quarter_files"],
                "outputs": [],
                "check": ["python3", "programs/verify_csv/main.py", "--check"],
                "timeout_seconds": 30,
                "read_only": true
            }
        ],
        "steps": [
            {"kind": "instruction", "text": "Confirm which invoices belong to the requested quarter.", "source": {"path": SKILL, "section": "Selecting files"}, "produces": ["quarter_files"]},
            {"kind": "program", "program": "extract"},
            {"kind": "program", "program": "summarize"}
        ],
        "verification": [
            {"kind": "program", "program": "verify_csv"},
            {"kind": "inspection", "text": "Open the CSV and confirm one row per invoice."}
        ]
    })
}

pub fn examples() -> Value {
    json!({
        "bindings": {
            "input_dir": "invoices/2026-q3/",
            "currency": "EUR",
            "quarter_files": ["q3-01.pdf", "q3-02.pdf"]
        },
        "steps": {
            "1": {"result": {"rows_file": "out/rows.json", "invoices": 2}},
            "2": {"result": {"summary_file": "out/q3-summary.csv", "total": "18240.00"}}
        }
    })
}

pub fn files() -> Files {
    let mut files = Files::new();
    for program in ["extract", "summarize", "verify_csv"] {
        files.insert(
            &format!("programs/{program}/main.py"),
            format!("# {program}\nprint('{{\"status\": \"complete\", \"result\": {{}}}}')\n")
                .into_bytes(),
        );
    }
    files.insert(
        "assets/columns.json",
        b"[\"invoice\", \"total\"]\n".to_vec(),
    );
    files
}

pub fn bytes(value: &Value) -> Vec<u8> {
    serde_json::to_vec_pretty(value).unwrap()
}

/// The textual form of the same capsule.
pub fn textual() -> (Value, String) {
    (
        json!({
            "schema_version": 1,
            "form": "textual",
            "mu": {"name": "pdf-invoice-totals", "description": "Extract totals from invoice PDFs into a CSV summary"},
            "source": {"skills": [{"name": "pdf", "path": SKILL, "tree_sha256": "0".repeat(64)}]}
        }),
        "# Summarise invoice totals\n\nWhen to use: a request for totals of a folder of invoice PDFs.\n\n## Steps\n\n1. List the PDFs of the requested quarter.\n2. Extract one row per invoice.\n3. Write the summary CSV.\n\n## Verify\n\n1. Open the CSV and confirm one row per invoice.\n".to_string(),
    )
}
