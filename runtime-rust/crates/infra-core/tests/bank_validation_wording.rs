//! A broken chunk is reported in the words the retired validator used: the
//! sentence is all a person repairing the bank is told, and it reaches a
//! refused promotion too. A chunk that correctly reached the end of its life
//! warns instead of failing the bank.
use infra_core::{BankService, Roots, RuntimeContext};
use serde_json::{Value, json};
use std::{fs, path::PathBuf};

struct Fixture {
    root: PathBuf,
    bank: BankService,
}
impl Fixture {
    fn new() -> Self {
        let root = std::env::temp_dir().join(format!(
            "infra-bank-wording-{}",
            infra_core::domain::uuid4()
        ));
        fs::create_dir_all(root.join("memory-bank/chunks")).unwrap();
        fs::create_dir_all(root.join("src")).unwrap();
        fs::write(
            root.join("src/Refund.php"),
            "<?php\nfinal class Refund {}\n",
        )
        .unwrap();
        fs::write(root.join("memory-bank/README.md"), "# Memory Bank\n").unwrap();
        let context =
            RuntimeContext::new(Roots::installed(&root).unwrap(), "local", "governed").unwrap();
        Self {
            root,
            bank: BankService::new(context),
        }
    }
    /// Writes the chunks and an index with a row for each `indexed` one.
    fn bank(&self, chunks: &[(&Value, String)], indexed: &[&Value]) {
        let directory = self.root.join("memory-bank/chunks");
        fs::remove_dir_all(&directory).unwrap();
        fs::create_dir(&directory).unwrap();
        for (metadata, text) in chunks {
            fs::write(directory.join(file(metadata)), text).unwrap();
        }
        let mut index = String::from(
            "# Memory Index\n\n| ID | Title | Type | Scope | Tags | Status | Last Verified | File |\n| --- | --- | --- | --- | --- | --- | --- | --- |\n",
        );
        for m in indexed {
            let list = |key: &str| {
                m[key]
                    .as_array()
                    .unwrap()
                    .iter()
                    .map(|item| item.as_str().unwrap())
                    .collect::<Vec<_>>()
                    .join(", ")
            };
            index.push_str(&format!(
                "| {} | {} | {} | {} | {} | {} | {} | chunks/{} |\n",
                m["id"].as_str().unwrap(),
                m["title"].as_str().unwrap(),
                m["type"].as_str().unwrap(),
                list("scope"),
                list("tags"),
                m["status"].as_str().unwrap(),
                m["last_verified"].as_str().unwrap(),
                file(m)
            ));
        }
        fs::write(self.root.join("memory-bank/INDEX.md"), index).unwrap();
    }
    fn report(&self) -> (Vec<String>, Vec<String>) {
        self.bank.report().unwrap()
    }
}
impl Drop for Fixture {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.root);
    }
}

fn chunk(n: u32, changes: Value) -> Value {
    let mut metadata = json!({
        "id": format!("MEM-20261104-abcde{n:03x}"), "title": format!("Fact {n}"),
        "type": "decision", "status": "active", "scope": ["billing"], "tags": ["billing"],
        "created": "2026-01-01", "last_verified": "2026-01-01", "review_after": "2999-01-01",
        "sources": ["src/Refund.php"], "supersedes": [], "superseded_by": null
    });
    for (key, value) in changes.as_object().unwrap() {
        metadata[key] = value.clone();
    }
    metadata
}
fn file(metadata: &Value) -> String {
    format!("{}-fact.md", metadata["id"].as_str().unwrap())
}
fn text(metadata: &Value) -> String {
    format!(
        "---\n{}\n---\n\nRefund keeps the invoice date.\n",
        serde_json::to_string_pretty(metadata).unwrap()
    )
}

#[test]
fn a_broken_chunk_names_what_to_repair() {
    let fixture = Fixture::new();
    let path = |metadata: &Value| format!("memory-bank/chunks/{}", file(metadata));
    let mut keyless = chunk(1, json!({}));
    keyless.as_object_mut().unwrap().remove("tags");
    keyless.as_object_mut().unwrap().remove("scope");
    let extra = chunk(2, json!({"layer": "application"}));
    let overdue = chunk(
        3,
        json!({"created": "2020-01-01", "last_verified": "2020-01-01", "review_after": "2020-06-01"}),
    );
    let digest = chunk(
        4,
        json!({"source_digests": [{"path": "src/Invoice.php", "sha256": "0".repeat(64)}]}),
    );
    let conflicted = chunk(5, json!({}));
    let conflict = text(&conflicted).replace(
        "  \"last_verified\": \"2026-01-01\",\n",
        "<<<<<<< HEAD\n  \"last_verified\": \"2026-01-01\",\n=======\n  \"last_verified\": \"2026-01-02\",\n>>>>>>> theirs\n",
    );
    let comma = chunk(6, json!({}));
    let trailing = text(&comma).replacen("\n}\n---", ",\n}\n---", 1);
    fixture.bank(
        &[
            (&keyless, text(&keyless)),
            (&extra, text(&extra)),
            (&overdue, text(&overdue)),
            (&digest, text(&digest)),
            (&conflicted, conflict),
            (&comma, trailing.clone()),
        ],
        &[],
    );
    let (errors, warnings) = fixture.report();
    // Python counts from the frontmatter's first character; the comma ends
    // the last field's line.
    let raw = &trailing[4..];
    let position = raw.find(",\n}\n---").unwrap();
    let line = raw[..position].matches('\n').count() + 1;
    let column = position - raw[..position].rfind('\n').unwrap();
    assert_eq!(
        errors,
        [
            format!("{}: missing metadata keys: scope, tags", path(&keyless)),
            format!("{}: unexpected metadata keys: layer", path(&extra)),
            format!("{}: active chunk is overdue for review", path(&overdue)),
            format!(
                "{}: source_digests may only cover paths listed in sources",
                path(&digest)
            ),
            format!(
                "{}: unresolved Git merge conflict in the frontmatter; keep one side or merge both, without the markers",
                path(&conflicted)
            ),
            format!(
                "{}: frontmatter is not valid JSON-compatible YAML: Illegal trailing comma before end of object: line {line} column {column} (char {position})",
                path(&comma)
            ),
        ],
    );
    assert!(warnings.is_empty(), "{warnings:?}");
}

#[test]
fn the_index_and_lineage_are_checked_field_by_field_and_a_retired_chunk_only_warns() {
    let fixture = Fixture::new();
    let old = chunk(1, json!({}));
    let new = chunk(2, json!({"supersedes": ["MEM-20261104-abcde001"]}));
    let mut renamed = new.clone();
    renamed["title"] = "Fact two, renamed".into();
    let retired = chunk(
        3,
        json!({"status": "archived", "sources": ["src/Gone.php"]}),
    );
    let broken = chunk(4, json!({"type": "fact"}));
    fixture.bank(
        &[
            (&old, text(&old)),
            (&new, text(&new)),
            (&retired, text(&retired)),
            (&broken, text(&broken)),
        ],
        &[&old, &renamed, &retired, &broken],
    );
    let (errors, warnings) = fixture.report();
    assert_eq!(
        errors,
        [
            "memory-bank/chunks/MEM-20261104-abcde004-fact.md: type must be one of: architecture, constraint, convention, decision, domain, integration, operations",
            "memory-bank/INDEX.md: MEM-20261104-abcde002 title differs from chunk metadata",
            "MEM-20261104-abcde002: MEM-20261104-abcde001 must have superseded status",
            "MEM-20261104-abcde002: MEM-20261104-abcde001 must point back with superseded_by",
        ],
    );
    assert_eq!(
        warnings,
        [
            "memory-bank/chunks/MEM-20261104-abcde003-fact.md: cited source no longer exists: src/Gone.php (chunk is archived and no longer retrievable)",
            "memory-bank/INDEX.md: MEM-20261104-abcde004 index row retained for invalid chunk (chunks/MEM-20261104-abcde004-fact.md)",
        ],
    );
    // Errors alone decide a write: a warning never blocks one.
    assert_eq!(fixture.bank.validate().unwrap(), errors);
}
