use std::{
    collections::{BTreeMap, BTreeSet},
    fmt,
    time::{Duration, Instant},
};
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ScoreFailure {
    pub reason: &'static str,
}
impl fmt::Display for ScoreFailure {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.reason)
    }
}
impl std::error::Error for ScoreFailure {}
pub trait ExcerptScorer {
    fn score(
        &mut self,
        query: &str,
        passages: &[String],
        remaining: Duration,
    ) -> Result<Vec<f64>, ScoreFailure>;
}
#[derive(Debug, Clone, PartialEq)]
pub struct Section {
    pub hash: String,
    pub heading: String,
    pub text: String,
}
#[derive(Debug, Clone, PartialEq)]
pub struct Candidate {
    pub path: String,
    pub title: String,
    pub source_sha256: String,
    pub layer: String,
    pub kind: String,
    pub selection: String,
    pub distinctive: bool,
    pub estimated_tokens: u64,
    pub lexical: Section,
    pub alternatives: Vec<Section>,
}
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Membership {
    pub procedural: Vec<String>,
    pub semantic: Vec<String>,
    pub episodic: Vec<String>,
    pub selected: Vec<String>,
}
pub trait ExcerptAdmission {
    fn sanitize(&self, text: &str) -> String;
    fn estimate_tokens(&self, title: &str, snippet: &str) -> u64;
    fn snippet_window_chars(&self) -> usize;
    fn automatic_link(&self, selection: &str) -> bool;
    /// Reuse the production packer; no mutation may be published before both packs agree.
    fn pack_membership(&self, candidates: &[Candidate]) -> Result<Membership, ScoreFailure>;
}
pub struct Pilot {
    pub deadline: Instant,
    pub stats: BTreeMap<String, u64>,
}
impl Pilot {
    pub fn new(deadline: Instant) -> Self {
        Self {
            deadline,
            stats: BTreeMap::new(),
        }
    }
    fn tick(&mut self, key: &str) {
        *self.stats.entry(key.into()).or_default() += 1;
    }
    pub fn select<S: ExcerptScorer, A: ExcerptAdmission>(
        &mut self,
        admission: &A,
        scorer: &mut S,
        query: &str,
        lexical: &[Candidate],
    ) -> Vec<Candidate> {
        let reserve = Duration::from_secs(1);
        if self.deadline.saturating_duration_since(Instant::now()) <= reserve {
            self.tick("fallback:deadline-reserve");
            return lexical.to_vec();
        }
        let safe = admission.sanitize(query);
        if safe.is_empty() || safe.chars().count() > 1024 {
            self.tick("fallback:query-bound");
            return lexical.to_vec();
        }
        let mut groups = Vec::<(usize, Vec<Section>)>::new();
        let mut passages = vec![];
        for (i, item) in lexical.iter().enumerate() {
            if item.layer == "procedural"
                || item.distinctive
                || item.kind == "brain-history"
                || admission.automatic_link(&item.selection) && item.kind == "brain-task"
            {
                continue;
            }
            let mut options = vec![item.lexical.clone()];
            let mut seen = BTreeSet::from([(item.lexical.hash.clone(), item.lexical.text.clone())]);
            for option in item.alternatives.iter().take(7) {
                let text = admission
                    .sanitize(&option.text)
                    .split_whitespace()
                    .collect::<Vec<_>>()
                    .join(" ");
                if text.is_empty()
                    || text.chars().count() > admission.snippet_window_chars()
                    || admission.estimate_tokens(&item.title, &text) > item.estimated_tokens
                {
                    self.tick("filtered-options");
                    continue;
                }
                if seen.insert((option.hash.clone(), text.clone())) {
                    options.push(Section {
                        text,
                        ..option.clone()
                    });
                }
            }
            options.truncate(32usize.saturating_sub(passages.len()));
            if options.len() > 1 {
                for option in &mut options {
                    option.text = admission
                        .sanitize(&option.text)
                        .split_whitespace()
                        .collect::<Vec<_>>()
                        .join(" ");
                    passages.push(option.text.clone());
                }
                groups.push((i, options));
            }
        }
        if groups.is_empty() {
            self.tick("no-alternatives");
            return lexical.to_vec();
        }
        if passages.iter().any(String::is_empty) {
            self.tick("fallback:screened-input");
            return lexical.to_vec();
        }
        self.tick("worker-calls");
        let remaining = self
            .deadline
            .saturating_duration_since(Instant::now())
            .saturating_sub(reserve);
        let scores = match scorer.score(&safe, &passages, remaining) {
            Ok(v) if v.len() == passages.len() && v.iter().all(|v| v.is_finite()) => v,
            Ok(_) => {
                self.tick("fallback:invalid-output");
                return lexical.to_vec();
            }
            Err(e) => {
                self.tick(&format!("fallback:{}", e.reason));
                return lexical.to_vec();
            }
        };
        let mut proposed = lexical.to_vec();
        let mut offset = 0;
        let mut changes = 0;
        for (i, options) in groups {
            let mut chosen = 0;
            for index in 1..options.len() {
                if scores[offset + index] > scores[offset + chosen] {
                    chosen = index;
                }
            }
            offset += options.len();
            if chosen != 0 {
                let selected = options[chosen].clone();
                let tokens = admission.estimate_tokens(&proposed[i].title, &selected.text);
                if tokens > proposed[i].estimated_tokens {
                    self.tick("fallback:snippet-budget");
                    return lexical.to_vec();
                }
                proposed[i].lexical = selected;
                proposed[i].estimated_tokens = tokens;
                changes += 1;
            }
        }
        let base = admission.pack_membership(lexical);
        let next = admission.pack_membership(&proposed);
        match (base, next) {
            (Ok(a), Ok(b)) if a == b => {
                *self.stats.entry("proposed-excerpts".into()).or_default() += changes;
                *self.stats.entry("changed-excerpts".into()).or_default() += changes;
                proposed
            }
            _ => {
                self.tick("fallback:pack-change");
                lexical.to_vec()
            }
        }
    }
}
