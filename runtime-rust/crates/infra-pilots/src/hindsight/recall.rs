use super::wire;
use crate::strict;
use anyhow::{Context, Result, bail};
use regex::Regex;
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use std::{
    collections::{BTreeMap, BTreeSet},
    io::Read,
    ops::Range,
    sync::Arc,
    time::{Duration, Instant},
};
use uuid::Uuid;
pub const PROFILE: &str = "hindsight-source-pilot-v1";
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Scope {
    pub team_id: Uuid,
    pub project_id: Uuid,
}
impl Scope {
    pub fn new(team: &str, project: &str) -> Result<Self> {
        Ok(Self {
            team_id: strict::uuid(team)?,
            project_id: strict::uuid(project)?,
        })
    }
    pub fn key(&self) -> String {
        strict::sha(
            &serde_json::to_vec(&[self.team_id.to_string(), self.project_id.to_string()]).unwrap(),
        )
    }
    pub fn bank_id(&self) -> String {
        format!("harness-{}", self.key())
    }
    pub fn tag(&self) -> String {
        format!("harness-scope:{}", self.key())
    }
}
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SourceBinding {
    pub scope: Scope,
    pub document_id: String,
    pub path: String,
    pub source_sha256: String,
}
impl SourceBinding {
    pub fn validate(&self) -> Result<()> {
        if !Regex::new(r"^[A-Za-z0-9][A-Za-z0-9._-]{0,127}$")?.is_match(&self.document_id)
            || !strict::digest(&self.source_sha256)
        {
            bail!("binding");
        }
        strict::path(&self.path, 512)
    }
    pub fn from_bytes(scope: Scope, path: &str, raw: &[u8]) -> Result<Self> {
        if raw.len() > 131072 {
            bail!("source_bound");
        }
        let digest = strict::sha(raw);
        let id = strict::sha(&wire::value(&[scope.key(), path.into(), digest.clone()])?);
        let b = Self {
            scope,
            document_id: format!("source-{id}"),
            path: path.into(),
            source_sha256: digest,
        };
        b.validate()?;
        Ok(b)
    }
    pub fn validate_integrity(&self) -> Result<()> {
        self.validate()?;
        let want = format!(
            "source-{}",
            strict::sha(&wire::value(&[
                self.scope.key(),
                self.path.clone(),
                self.source_sha256.clone()
            ])?)
        );
        if want != self.document_id {
            bail!("approval");
        }
        Ok(())
    }
    pub fn image(&self) -> Value {
        json!({"document_id":self.document_id,"path":self.path,"source_sha256":self.source_sha256})
    }
    pub fn image_wire(&self) -> Result<Vec<u8>> {
        wire::object(vec![
            ("document_id", wire::value(&self.document_id)?),
            ("path", wire::value(&self.path)?),
            ("source_sha256", wire::value(&self.source_sha256)?),
        ])
    }
}
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Catalogue {
    pub scope: Scope,
    pub sources: Vec<SourceBinding>,
}
impl Catalogue {
    pub fn new(scope: Scope, sources: Vec<SourceBinding>) -> Result<Self> {
        if sources.len() > 32
            || sources.iter().any(|s| s.scope != scope)
            || sources
                .iter()
                .map(|s| &s.document_id)
                .collect::<BTreeSet<_>>()
                .len()
                != sources.len()
        {
            bail!("catalogue");
        }
        for s in &sources {
            s.validate()?;
        }
        Ok(Self { scope, sources })
    }
}
#[derive(Debug, Clone)]
pub struct Passage {
    pub document_id: String,
    pub path: String,
    pub source_sha256: String,
    pub text: String,
}
pub struct Evidence {
    pub binding: SourceBinding,
    pub raw: Vec<u8>,
    pub range: Range<usize>,
}
impl Evidence {
    pub fn passage(self, expected: &SourceBinding) -> Result<Passage> {
        if self.binding != *expected
            || self.raw.len() > 131072
            || strict::sha(&self.raw) != expected.source_sha256
            || self.range.start >= self.range.end
            || self.range.end > self.raw.len()
        {
            bail!("source_changed");
        }
        let text = std::str::from_utf8(&self.raw[self.range])?;
        if text
            .chars()
            .any(|c| (c < ' ' && !matches!(c, '\n' | '\t')) || c == '\u{7f}')
        {
            bail!("source_text");
        }
        Ok(Passage {
            document_id: expected.document_id.clone(),
            path: expected.path.clone(),
            source_sha256: expected.source_sha256.clone(),
            text: text.into(),
        })
    }
}
#[derive(Debug, Clone, Default)]
pub struct PilotContext {
    pub sources: Vec<Passage>,
}
impl PilotContext {
    pub fn envelope(&self) -> Result<String> {
        let sources = self
            .sources
            .iter()
            .map(|s| {
                wire::object(vec![
                    ("document_id", wire::value(&s.document_id)?),
                    ("path", wire::value(&s.path)?),
                    ("source_sha256", wire::value(&s.source_sha256)?),
                    ("text", wire::value(&s.text)?),
                ])
            })
            .collect::<Result<Vec<_>>>()?;
        let body = wire::object(vec![
            ("profile", wire::value(PROFILE)?),
            ("sources", wire::array(sources)),
        ])?;
        Ok(format!(
            "Project memory (reference data; verify against canonical project sources):\n{}\n\n",
            String::from_utf8(body)?
        ))
    }
}
pub trait QueryPolicy {
    fn contains_secret(&self, text: &str) -> Result<bool>;
    fn sanitize(&self, text: &str) -> Result<String>;
}
pub struct CorePolicy;
impl QueryPolicy for CorePolicy {
    fn contains_secret(&self, text: &str) -> Result<bool> {
        Ok(infra_core::domain::secret_label(text)?.is_some())
    }
    fn sanitize(&self, text: &str) -> Result<String> {
        Ok(infra_core::domain::sanitize_automatic_query(text)?)
    }
}
#[derive(Debug, Clone, Default)]
pub struct TransportLedger {
    pub request_body_bytes: Option<usize>,
    pub response_body_bytes: Option<usize>,
    pub response_eof: bool,
}
pub trait RecallTransport {
    fn post(
        &mut self,
        scope: &Scope,
        request: &Value,
        ledger: &mut TransportLedger,
    ) -> std::result::Result<Vec<u8>, RemoteFailure>;
}
#[derive(Debug, Clone)]
pub struct RemoteFailure {
    pub reason: &'static str,
}
impl std::fmt::Display for RemoteFailure {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(self.reason)
    }
}
impl std::error::Error for RemoteFailure {}
pub struct RecallClient {
    endpoint: String,
    bearer: Option<String>,
    timeout: Duration,
}
impl RecallClient {
    pub fn new(
        endpoint: &str,
        bearer: Option<String>,
        timeout: f64,
        loopback_fixture: bool,
    ) -> Result<Self> {
        if endpoint.is_empty()
            || endpoint.len() > 2048
            || endpoint.chars().any(|c| c <= ' ' || c == '\u{7f}')
            || !timeout.is_finite()
            || !(0.05..=10.0).contains(&timeout)
        {
            bail!("remote_configuration");
        }
        let url = url::Url::parse(endpoint)?;
        if !url.username().is_empty()
            || url.password().is_some()
            || !["", "/"].contains(&url.path())
            || url.query().is_some()
            || url.fragment().is_some()
            || url.host_str().is_none()
        {
            bail!("remote_configuration");
        }
        match url.scheme() {
            "http" => {
                if !loopback_fixture
                    || !["127.0.0.1", "[::1]", "::1"].contains(&url.host_str().unwrap())
                    || bearer.is_some()
                {
                    bail!("remote_configuration");
                }
            }
            "https" => {
                if loopback_fixture
                    || bearer.as_ref().is_none_or(|s| {
                        s.is_empty()
                            || s.len() > 4096
                            || s.bytes().any(|b| !(33..=126).contains(&b))
                    })
                {
                    bail!("remote_configuration");
                }
            }
            _ => bail!("remote_configuration"),
        };
        Ok(Self {
            endpoint: endpoint.strip_suffix('/').unwrap_or(endpoint).to_owned(),
            bearer,
            timeout: Duration::from_secs_f64(timeout),
        })
    }
}
impl RecallTransport for RecallClient {
    fn post(
        &mut self,
        scope: &Scope,
        request: &Value,
        ledger: &mut TransportLedger,
    ) -> std::result::Result<Vec<u8>, RemoteFailure> {
        let fail = |reason| RemoteFailure { reason };
        let raw = serde_json::to_vec(request).map_err(|_| fail("request_bound"))?;
        if raw.len() > 8192 {
            return Err(fail("request_bound"));
        }
        ledger.request_body_bytes = Some(raw.len());
        let start = Instant::now();
        let agent = ureq::AgentBuilder::new()
            .try_proxy_from_env(false)
            .redirects(0)
            .max_idle_connections(0)
            .timeout(self.timeout)
            .build();
        let url = format!(
            "{}/v1/default/banks/{}/memories/recall",
            self.endpoint,
            scope.bank_id()
        );
        let mut request = agent
            .post(&url)
            .set("Content-Type", "application/json")
            .set("Accept", "application/json")
            .set("Accept-Encoding", "identity")
            .set("Connection", "close");
        if let Some(token) = &self.bearer {
            request = request.set("Authorization", &format!("Bearer {token}"));
        }
        let response = match request.send_bytes(&raw) {
            Ok(r) => r,
            Err(ureq::Error::Status(_, r)) => r,
            Err(_) => {
                return Err(fail(if start.elapsed() >= self.timeout {
                    "timeout"
                } else {
                    "transport_failure"
                }));
            }
        };
        let status = response.status();
        if (300..400).contains(&status) {
            return Err(fail("redirect"));
        }
        if [401, 403].contains(&status) {
            return Err(fail("authorization"));
        }
        if status != 200 {
            return Err(fail("http_failure"));
        }
        if response
            .header("Content-Encoding")
            .unwrap_or("identity")
            .to_lowercase()
            != "identity"
            || response
                .header("Content-Type")
                .unwrap_or("")
                .split(';')
                .next()
                .unwrap_or("")
                .trim()
                .to_lowercase()
                != "application/json"
        {
            return Err(fail("response_encoding"));
        }
        let declared = response
            .header("Content-Length")
            .and_then(|s| s.parse::<usize>().ok());
        let mut reader = response.into_reader();
        let mut out = vec![];
        ledger.response_body_bytes = Some(0);
        loop {
            if start.elapsed() >= self.timeout {
                return Err(fail("timeout"));
            }
            let mut b = [0; 4096];
            let want = (65537 - out.len()).min(b.len());
            let n = reader.read(&mut b[..want]).map_err(|e| {
                fail(match e.kind() {
                    std::io::ErrorKind::TimedOut | std::io::ErrorKind::WouldBlock => "timeout",
                    std::io::ErrorKind::UnexpectedEof => "incomplete_response",
                    _ => "transport_failure",
                })
            })?;
            ledger.response_body_bytes = Some(out.len() + n);
            if start.elapsed() >= self.timeout {
                return Err(fail("timeout"));
            }
            if n == 0 {
                if declared.is_some_and(|n| n != out.len()) {
                    return Err(fail("incomplete_response"));
                }
                ledger.response_eof = true;
                return Ok(out);
            }
            out.extend_from_slice(&b[..n]);
            if out.len() > 65536 {
                return Err(fail("response_bound"));
            }
        }
    }
}
#[derive(Debug, Clone)]
pub struct ReferenceOutcome {
    pub references: Vec<SourceBinding>,
    pub diagnostics: Value,
}
impl ReferenceOutcome {
    pub fn native_payload(&self) -> Result<Value> {
        if self.references.len() > 8 {
            bail!("reference_bound");
        }
        Ok(
            json!({"schema_version":1,"references":self.references.iter().map(|r|json!({"path":r.path,"source_sha256":r.source_sha256})).collect::<Vec<_>>()}),
        )
    }
}
fn diagnostics(
    ledger: &TransportLedger,
    attempts: usize,
    count: usize,
    rejected: usize,
    outcome: &str,
    start: Instant,
) -> Value {
    json!({"profile":PROFILE,"attempts":attempts,"resolved":count,"rejected":rejected,"outcome":outcome,"elapsed_seconds":start.elapsed().as_secs_f64(),"remote_tokens":null,"remote_usd":null,"provider_tokens":null,"total_cost_complete":false,"request_body_bytes":ledger.request_body_bytes.filter(|n|*n<=8192),"response_body_bytes":ledger.response_body_bytes.filter(|n|*n<=65537),"response_eof":ledger.response_eof})
}
pub fn recall_references<T: RecallTransport, P: QueryPolicy, A: FnMut() -> bool>(
    query: &str,
    client: &mut T,
    catalogue: &Catalogue,
    policy: &P,
    active: A,
) -> ReferenceOutcome {
    recall_selected(query, client, catalogue, policy, active, 8)
}
fn recall_selected<T: RecallTransport, P: QueryPolicy, A: FnMut() -> bool>(
    query: &str,
    client: &mut T,
    catalogue: &Catalogue,
    policy: &P,
    mut active: A,
    limit: usize,
) -> ReferenceOutcome {
    let start = Instant::now();
    let mut ledger = TransportLedger::default();
    let mut attempts = 0;
    let mut rejected = 0;
    let result = (|| -> Result<(Vec<SourceBinding>, &'static str)> {
        Catalogue::new(catalogue.scope.clone(), catalogue.sources.clone())?;
        if !active() {
            return Ok((vec![], "cancelled"));
        }
        if query.len() > 2000 {
            return Ok((vec![], "query_bound"));
        }
        if policy.contains_secret(query)? {
            return Ok((vec![], "query_rejected"));
        }
        let query = policy.sanitize(query)?.trim().to_owned();
        if query.is_empty() {
            return Ok((vec![], "query_empty"));
        }
        if catalogue.sources.is_empty() {
            return Ok((vec![], "catalogue_empty"));
        }
        let request = json!({"query":query,"types":["world"],"budget":"low","max_tokens":512,"trace":false,"include":{"entities":null},"tags":[catalogue.scope.tag()],"tags_match":"all_strict"});
        attempts = 1;
        let raw = match client.post(&catalogue.scope, &request, &mut ledger) {
            Ok(raw) => raw,
            Err(e) => return Ok((vec![], e.reason)),
        };
        if raw.len() > 65536 {
            return Ok((vec![], "response_bound"));
        }
        let value = strict::decode(&raw, 65536, 16, 4096)?;
        let results = value["results"].as_array().context("response_schema")?;
        if results.len() > 32 {
            return Ok((vec![], "response_schema"));
        }
        let bindings = catalogue
            .sources
            .iter()
            .map(|b| (&b.document_id, b))
            .collect::<BTreeMap<_, _>>();
        let mut refs = vec![];
        let mut seen = BTreeSet::new();
        for item in results {
            if !active() {
                return Ok((vec![], "cancelled"));
            }
            if !item.is_object() {
                return Ok((vec![], "response_schema"));
            }
            let document = item["document_id"].as_str().unwrap_or("");
            let binding = bindings.get(&document.to_owned());
            if let Some(binding) = binding
                && item["metadata"]["harness_scope"] == catalogue.scope.key()
                && item["metadata"]["source_sha256"] == binding.source_sha256
                && item["type"] == "world"
            {
                if seen.insert(document.to_string()) {
                    if refs.len() < limit {
                        refs.push((*binding).clone());
                    } else {
                        rejected += 1;
                    }
                }
                continue;
            }
            rejected += 1;
        }
        if !active() {
            return Ok((vec![], "cancelled"));
        }
        let outcome = if refs.is_empty() {
            "no_admitted_sources"
        } else {
            "resolved"
        };
        Ok((refs, outcome))
    })();
    let (refs, outcome) = result.unwrap_or_else(|_| (vec![], "unavailable"));
    ReferenceOutcome {
        diagnostics: diagnostics(&ledger, attempts, refs.len(), rejected, outcome, start),
        references: refs,
    }
}
pub struct PilotOutcome {
    pub context: Arc<PilotContext>,
    pub diagnostics: Value,
}
#[expect(
    clippy::too_many_arguments,
    reason = "Keep the public pilot contract explicit: transport, policy, admission, cancellation, and separate byte/character limits are independently supplied."
)]
pub fn recall_context<
    T: RecallTransport,
    P: QueryPolicy,
    A: FnMut() -> bool,
    F: FnMut(&SourceBinding) -> Result<Option<Evidence>>,
>(
    query: &str,
    client: &mut T,
    catalogue: &Catalogue,
    policy: &P,
    local: Arc<PilotContext>,
    mut admit: F,
    mut active: A,
    byte_limit: usize,
    char_limit: usize,
) -> Result<PilotOutcome> {
    if !(256..=8000).contains(&byte_limit) || !(256..=8000).contains(&char_limit) {
        bail!("context_budget");
    }
    let fits = |c: &PilotContext| -> Result<bool> {
        let text = c.envelope()?;
        Ok(text.len() <= byte_limit && text.chars().count() <= char_limit)
    };
    if !fits(&local)? {
        bail!("local_baseline_bound");
    }
    let outcome = recall_selected(query, client, catalogue, policy, &mut active, 32);
    let mut diagnostics = outcome.diagnostics;
    let mut proposed = local.as_ref().clone();
    let mut seen = proposed
        .sources
        .iter()
        .map(|s| s.document_id.clone())
        .collect::<BTreeSet<_>>();
    let mut accepted = 0;
    let mut reason = diagnostics["outcome"]
        .as_str()
        .unwrap_or("unavailable")
        .to_string();
    let result = (|| -> Result<()> {
        for binding in outcome.references {
            if !active() {
                reason = "cancelled".into();
                return Ok(());
            }
            if !seen.insert(binding.document_id.clone()) {
                continue;
            }
            let Some(evidence) = admit(&binding)? else {
                continue;
            };
            let passage = match evidence.passage(&binding) {
                Ok(p) => p,
                Err(_) => continue,
            };
            let mut next = proposed.clone();
            next.sources.push(passage);
            if fits(&next)? {
                proposed = next;
                accepted += 1;
            }
        }
        if !active() {
            reason = "cancelled".into();
        } else if accepted > 0 {
            reason = "augmented".into();
        } else if reason == "resolved" {
            reason = "no_admitted_sources".into();
        }
        Ok(())
    })();
    if result.is_err() {
        reason = "admission_failure".into();
    }
    let augmented = reason == "augmented";
    diagnostics["accepted"] = json!(if augmented { accepted } else { 0 });
    diagnostics["outcome"] = json!(reason);
    diagnostics["fallback"] = json!(!augmented);
    diagnostics["transport_metadata_complete"] = json!(
        diagnostics["attempts"] == 0
            || !diagnostics["request_body_bytes"].is_null()
                && !diagnostics["response_body_bytes"].is_null()
    );
    Ok(PilotOutcome {
        context: if augmented { Arc::new(proposed) } else { local },
        diagnostics,
    })
}
pub struct SourceHintProvider<T, P> {
    pub client: T,
    pub catalogue: Catalogue,
    pub policy: P,
    pub project_id: String,
    pub bank: String,
    pub workspace: String,
}
impl<T: RecallTransport, P: QueryPolicy> SourceHintProvider<T, P> {
    pub fn resolve<A: FnMut() -> bool>(
        &mut self,
        query: &str,
        project: &str,
        bank: &str,
        workspace: &str,
        active: A,
    ) -> ReferenceOutcome {
        if [
            self.project_id.as_str(),
            self.bank.as_str(),
            self.workspace.as_str(),
        ]
        .iter()
        .any(|s| s.is_empty())
            || (project, bank, workspace)
                != (
                    self.project_id.as_str(),
                    self.bank.as_str(),
                    self.workspace.as_str(),
                )
        {
            return ReferenceOutcome {
                references: vec![],
                diagnostics: json!({"attempts":0,"outcome":"unavailable","request_body_bytes":null,"response_body_bytes":null,"response_eof":null}),
            };
        }
        recall_references(
            query,
            &mut self.client,
            &self.catalogue,
            &self.policy,
            active,
        )
    }
}
