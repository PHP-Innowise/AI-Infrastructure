pub mod codec;
pub mod export;
pub mod maintenance;
pub mod promotion;
pub mod publication;
pub mod retirement;
pub mod validation;
use crate::{BrainRepository, Result, RuntimeConfig, RuntimeContext};
use serde_json::Value;

#[derive(Clone, Debug)]
pub struct BankService {
    pub context: RuntimeContext,
}
impl BankService {
    pub fn new(context: RuntimeContext) -> Self {
        Self { context }
    }
    pub fn brain(&self) -> Result<BrainRepository> {
        BrainRepository::new(self.context.clone())
    }
    pub fn config(&self) -> Result<RuntimeConfig> {
        RuntimeConfig::load(&self.context.roots)
    }
    pub fn validate(&self) -> Result<Vec<String>> {
        validation::validate_bank(self)
    }
    /// Errors, then warnings that do not fail the bank.
    pub fn report(&self) -> Result<(Vec<String>, Vec<String>)> {
        validation::report(self)
    }
    pub fn index(&self) -> Result<Value> {
        codec::reindex(self)
    }
    pub fn audit(&self, provenance: bool) -> Result<Value> {
        maintenance::audit(self, provenance)
    }
    pub fn reverify(&self, id: &str, review_after: Option<&str>) -> Result<Value> {
        retirement::reverify(self, id, review_after)
    }
    pub fn retire(
        &self,
        id: &str,
        valid_to: &str,
        successor: Option<&str>,
        reason: Option<&str>,
    ) -> Result<Value> {
        retirement::retire(self, id, valid_to, successor, reason)
    }
    pub fn quarantine(&self, id: &str, reason: Option<&str>) -> Result<Value> {
        self.retire(
            id,
            &codec::today().to_string(),
            None,
            Some(reason.unwrap_or("quarantined")),
        )
    }
    pub fn propose(
        &self,
        ids: &[String],
        title: &str,
        content: &str,
        automatic: bool,
    ) -> Result<Value> {
        promotion::propose(self, ids, title, content, automatic)
    }
    pub fn review(&self, id: &str, reviewer: Option<&str>, approve: bool) -> Result<Value> {
        promotion::review(self, id, reviewer, approve)
    }
    pub fn apply(&self, id: &str) -> Result<Value> {
        promotion::apply(self, id)
    }
    pub fn auto_promote(
        &self,
        limit: usize,
        max_attempts: Option<usize>,
        only: Option<&[String]>,
    ) -> Result<Value> {
        promotion::auto_promote(self, limit, max_attempts, only)
    }
    pub fn consolidate(&self, apply: bool, ticket: Option<&str>) -> Result<Value> {
        maintenance::consolidate(self, apply, ticket)
    }
    pub fn compact(&self, only_external_id: Option<&str>) -> Result<Value> {
        maintenance::compact(self, only_external_id)
    }
    pub fn auto_consolidate(&self) -> Result<Value> {
        maintenance::auto_consolidate(self)
    }
    pub fn auto_compact(&self) -> Result<Value> {
        maintenance::auto_compact(self)
    }
    pub fn retry_pending(&self) -> Result<Value> {
        maintenance::retry_pending(self)
    }
    pub fn export(
        &self,
        destination: &std::path::Path,
        include_archive: bool,
        include_superseded: bool,
        force: bool,
    ) -> Result<Value> {
        export::export(
            self,
            destination,
            include_archive,
            include_superseded,
            force,
        )
    }
}

pub mod consolidation;
pub mod repository;
