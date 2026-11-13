//! A record with a long title can be promoted. The chunk took its file name
//! from the whole title, and publication writes `.{name}.{uuid}.tmp` beside
//! it, so a real 200-character title failed with "File name too long" and
//! the record could never become memory.
use infra_core::{BankService, BrainRepository, CreateRecord, Roots, RuntimeContext, UpdateRecord};
use std::{fs, path::PathBuf};

struct Root(PathBuf);
impl Drop for Root {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.0);
    }
}
fn context(root: &PathBuf, actor: &str) -> RuntimeContext {
    RuntimeContext::new(Roots::installed(root).unwrap(), actor, "governed").unwrap()
}

#[test]
fn a_record_with_a_long_title_becomes_a_chunk_with_a_bounded_name() {
    let root = Root(std::env::temp_dir().join(format!(
        "infra-promotion-long-title-{}",
        infra_core::domain::uuid4()
    )));
    fs::create_dir_all(root.0.join("src")).unwrap();
    fs::write(root.0.join("src/refund.go"), "package refund\n").unwrap();
    let title = format!(
        "Refund recipients come from the captured payment: {}",
        "payments derives customer_id at capture and notifications resolves it to contact details "
            .repeat(3)
    );
    let title = title.trim();
    assert!(title.len() > 250);
    let content =
        "Payments records the customer identity when a payment is captured; refunds reuse it.";
    let brain = BrainRepository::new(context(&root.0, "author")).unwrap();
    let record = brain
        .create(CreateRecord {
            record_type: "finding".into(),
            external_id: "LONG-TITLE".into(),
            title: title.into(),
            goal: Some(title.into()),
            sources: vec!["src/refund.go".into()],
            authority: "verified".into(),
            attestation: Some("person".into()),
            ..CreateRecord::default()
        })
        .unwrap();
    let id = record["id"].as_str().unwrap().to_string();
    brain
        .update(
            &id,
            UpdateRecord {
                expected_revision: record["revision"].as_u64(),
                progress: Some(content.into()),
                transition_to: Some("resolved".into()),
                reason: "Checked against the code".into(),
                attestation: Some("person".into()),
                ..Default::default()
            },
        )
        .unwrap();
    let proposal = BankService::new(context(&root.0, "author"))
        .propose(&[id], title, content, false)
        .unwrap();
    let reviewer = BankService::new(context(&root.0, "reviewer"));
    let promotion = proposal["id"].as_str().unwrap();
    reviewer.review(promotion, Some("reviewer"), true).unwrap();
    let applied = reviewer.apply(promotion).unwrap();
    assert_eq!(applied["status"], "applied", "{applied:#}");
    let memory = applied["destination_memory_id"].as_str().unwrap();
    let name = fs::read_dir(root.0.join("memory-bank/chunks"))
        .unwrap()
        .map(|entry| entry.unwrap().file_name().into_string().unwrap())
        .find(|name| name.starts_with(memory))
        .unwrap();
    assert!(
        name.len() <= memory.len() + 1 + infra_core::bank::codec::CHUNK_SLUG_LIMIT + 3,
        "{name}"
    );
    assert!(name.starts_with(&format!(
        "{memory}-refund-recipients-come-from-the-captured-payment"
    )));
    // The whole title stays in the chunk.
    let text = fs::read_to_string(root.0.join("memory-bank/chunks").join(&name)).unwrap();
    assert!(text.contains(title), "{text}");
}
