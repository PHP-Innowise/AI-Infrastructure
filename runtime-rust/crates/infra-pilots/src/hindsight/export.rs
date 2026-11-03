use super::{
    recall::{Catalogue, Scope, SourceBinding},
    screen::{PublicPolicy, active, screen, source_path},
    wire,
};
use crate::strict;
use anyhow::{Context, Result, bail};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use std::{collections::BTreeSet, path::Path};
#[cfg(unix)]
use std::{
    fs::{self, File, OpenOptions},
    io::Read,
};
pub const PROFILE: &str = "hindsight-public-export-v1";
#[derive(Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct PublicApproval {
    pub scope: Scope,
    pub path: String,
    pub source_sha256: String,
    pub classification: String,
}
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct IoObservation {
    pub source_read_bytes: u64,
    pub source_read_calls: u64,
    pub opened_descriptors: u64,
    pub closed_descriptors: u64,
    pub close_errors: u64,
}
#[derive(Debug, Clone)]
pub struct ExportError {
    pub reason: &'static str,
    pub observation: Option<IoObservation>,
}
impl std::fmt::Display for ExportError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(self.reason)
    }
}
impl std::error::Error for ExportError {}
fn fixed_error(error: anyhow::Error, observation: Option<IoObservation>) -> anyhow::Error {
    let message = error.to_string();
    let reason = match message.as_str() {
        "approval" => "approval",
        "source_path" => "source_path",
        "source_bound" => "source_bound",
        "source_text" => "source_text",
        "source_changed" => "source_changed",
        "private_content" => "private_content",
        "file_kind" => "file_kind",
        "unsupported" => "unsupported",
        "cancelled" => "cancelled",
        "request_bound" => "request_bound",
        "packet_bound" => "packet_bound",
        "previous_catalogue" => "previous_catalogue",
        _ => "unavailable",
    };
    ExportError {
        reason,
        observation,
    }
    .into()
}
#[derive(Clone)]
pub struct RetainDraft {
    pub binding: SourceBinding,
    pub source_id: String,
    pub body: Vec<u8>,
}
pub fn retain_body(binding: &SourceBinding, source_id: &str, text: &str) -> Result<Vec<u8>> {
    let metadata = wire::object(vec![
        ("harness_scope", wire::value(&binding.scope.key())?),
        ("source_sha256", wire::value(&binding.source_sha256)?),
        ("source_id", wire::value(source_id)?),
        ("export_profile", wire::value(PROFILE)?),
    ])?;
    let item = wire::object(vec![
        ("content", wire::value(text)?),
        ("document_id", wire::value(&binding.document_id)?),
        ("timestamp", wire::value("unset")?),
        ("context", wire::value("public canonical source")?),
        ("metadata", metadata),
        ("tags", wire::value(&[binding.scope.tag()])?),
        ("update_mode", wire::value("replace")?),
    ])?;
    wire::object(vec![
        ("items", wire::array(vec![item])),
        ("async", wire::value(&false)?),
        ("document_tags", wire::value(&[binding.scope.tag()])?),
    ])
}
fn text_integrity(raw: &[u8]) -> Result<&str> {
    if raw.len() > 131072 {
        bail!("source_bound");
    }
    let text = std::str::from_utf8(raw)?;
    if text.trim().is_empty() || regex::Regex::new(r"[\p{C}&&[^\r\n\t]]")?.is_match(text) {
        bail!("source_text");
    }
    Ok(text)
}
impl RetainDraft {
    pub fn text(&self) -> Result<String> {
        self.binding.validate_integrity()?;
        if self.source_id
            != format!(
                "logical-{}",
                strict::sha(&wire::value(&[
                    self.binding.scope.key(),
                    self.binding.path.clone()
                ])?)
            )
            || self.body.is_empty()
            || self.body.len() > 262144
        {
            bail!("approval");
        }
        let v = strict::decode(&self.body, 262144, 16, 4096)?;
        let text = v["items"][0]["content"].as_str().context("approval")?;
        text_integrity(text.as_bytes())?;
        if strict::sha(text.as_bytes()) != self.binding.source_sha256
            || self.body != retain_body(&self.binding, &self.source_id, text)?
        {
            bail!("approval");
        }
        Ok(text.into())
    }
    pub fn validate<P: PublicPolicy>(&self, policy: &P, a: &mut dyn FnMut() -> bool) -> Result<()> {
        source_path(&self.binding.path, policy, a)?;
        screen(&self.text()?, &self.binding.path, policy, a)
    }
    pub fn method(&self) -> &'static str {
        "POST"
    }
    pub fn resource(&self) -> String {
        format!(
            "/v1/default/banks/{}/memories",
            self.binding.scope.bank_id()
        )
    }
}
#[derive(Clone)]
pub struct PublicExport {
    pub catalogue: Catalogue,
    pub retain_drafts: Vec<RetainDraft>,
    pub serialized: Vec<u8>,
    pub io: IoObservation,
}
pub(crate) fn packet(c: &Catalogue, drafts: &[RetainDraft]) -> Result<Vec<u8>> {
    let mut sources = vec![];
    for d in drafts {
        sources.push(wire::object(vec![
            ("source_id", wire::value(&d.source_id)?),
            ("path", wire::value(&d.binding.path)?),
            ("document_id", wire::value(&d.binding.document_id)?),
            ("source_sha256", wire::value(&d.binding.source_sha256)?),
            ("classification", wire::value("operator-attested-public")?),
            ("retain_request", d.body.clone()),
        ])?);
    }
    wire::object(vec![
        ("schema_version", wire::value(&1)?),
        ("profile", wire::value(PROFILE)?),
        ("bank_id", wire::value(&c.scope.bank_id())?),
        ("scope", wire::value(&c.scope.key())?),
        ("sources", wire::array(sources)),
    ])
}
#[cfg(unix)]
fn capture(
    root: &Path,
    approval: &PublicApproval,
    a: &mut dyn FnMut() -> bool,
    io: &mut IoObservation,
) -> Result<Vec<u8>> {
    use std::{
        ffi::CString,
        os::{
            fd::{AsRawFd, FromRawFd, IntoRawFd},
            unix::fs::{MetadataExt, OpenOptionsExt},
        },
    };
    fn image(m: &fs::Metadata) -> (u64, u64, u32, u64, u64, i64, i64, i64, i64) {
        (
            m.dev(),
            m.ino(),
            m.mode(),
            m.nlink(),
            m.size(),
            m.mtime(),
            m.mtime_nsec(),
            m.ctime(),
            m.ctime_nsec(),
        )
    }
    let mut files = vec![];
    let mut locations = vec![];
    let result = (|| -> Result<Vec<u8>> {
        let rootfile = OpenOptions::new()
            .read(true)
            .custom_flags(libc::O_DIRECTORY | libc::O_NOFOLLOW | libc::O_CLOEXEC)
            .open("/")?;
        files.push(rootfile);
        io.opened_descriptors += 1;
        let parts = root
            .components()
            .skip(1)
            .map(|c| c.as_os_str().to_string_lossy().into_owned())
            .chain(
                approval
                    .path
                    .split('/')
                    .take(approval.path.split('/').count() - 1)
                    .map(str::to_owned),
            )
            .collect::<Vec<_>>();
        for part in parts {
            active(a)?;
            let parent = files.last().unwrap().as_raw_fd();
            let name = CString::new(part.clone())?;
            let fd = unsafe {
                libc::openat(
                    parent,
                    name.as_ptr(),
                    libc::O_RDONLY | libc::O_DIRECTORY | libc::O_NOFOLLOW | libc::O_CLOEXEC,
                )
            };
            if fd < 0 {
                return Err(std::io::Error::last_os_error().into());
            }
            files.push(unsafe { File::from_raw_fd(fd) });
            io.opened_descriptors += 1;
            locations.push((files.len() - 2, part, files.len() - 1));
        }
        let parent = files.last().unwrap().as_raw_fd();
        let leaf = approval.path.rsplit('/').next().unwrap();
        let name = CString::new(leaf)?;
        let fd = unsafe {
            libc::openat(
                parent,
                name.as_ptr(),
                libc::O_RDONLY | libc::O_NOFOLLOW | libc::O_NONBLOCK | libc::O_CLOEXEC,
            )
        };
        if fd < 0 {
            return Err(std::io::Error::last_os_error().into());
        }
        files.push(unsafe { File::from_raw_fd(fd) });
        io.opened_descriptors += 1;
        let file = files.last_mut().unwrap();
        let before = file.metadata()?;
        if !before.is_file() || before.nlink() != 1 {
            bail!("file_kind");
        }
        if before.len() > 131072 {
            bail!("source_bound");
        }
        let mut raw = vec![];
        loop {
            active(a)?;
            let mut b = [0; 8192];
            let want = (131073 - raw.len()).min(b.len());
            let n = file.read(&mut b[..want])?;
            io.source_read_calls += 1;
            io.source_read_bytes += n as u64;
            if n == 0 {
                break;
            }
            raw.extend_from_slice(&b[..n]);
            if raw.len() > 131072 {
                bail!("source_bound");
            }
        }
        let after = file.metadata()?;
        let named = fs::symlink_metadata(root.join(&approval.path))?;
        if image(&before) != image(&after)
            || image(&after) != image(&named)
            || strict::sha(&raw) != approval.source_sha256
        {
            bail!("source_changed");
        }
        for (parent, name, child) in &locations {
            let held = files[*child].metadata()?;
            let name = CString::new(name.as_str())?;
            let mut st = std::mem::MaybeUninit::<libc::stat>::uninit();
            if unsafe {
                libc::fstatat(
                    files[*parent].as_raw_fd(),
                    name.as_ptr(),
                    st.as_mut_ptr(),
                    libc::AT_SYMLINK_NOFOLLOW,
                )
            } != 0
            {
                return Err(std::io::Error::last_os_error().into());
            }
            let st = unsafe { st.assume_init() };
            #[expect(
                clippy::unnecessary_cast,
                reason = "libc stat field widths vary across Unix targets; normalize the named image to MetadataExt types without changing signed cast semantics."
            )]
            let named_image = (st.st_dev as u64, st.st_ino as u64, st.st_mode as u32);
            if named_image != (held.dev(), held.ino(), held.mode()) {
                bail!("source_changed");
            }
        }
        Ok(raw)
    })();
    let mut failed = false;
    for file in files.into_iter().rev() {
        let fd = file.into_raw_fd();
        if unsafe { libc::close(fd) } == 0 {
            io.closed_descriptors += 1;
        } else {
            io.close_errors += 1;
            failed = true;
        }
    }
    if failed {
        bail!("unavailable");
    }
    result
}
#[cfg(not(unix))]
fn capture(
    _: &Path,
    _: &PublicApproval,
    _: &mut dyn FnMut() -> bool,
    _: &mut IoObservation,
) -> Result<Vec<u8>> {
    bail!("unsupported")
}
impl PublicExport {
    pub fn validate<P: PublicPolicy>(&self, policy: &P, a: &mut dyn FnMut() -> bool) -> Result<()> {
        active(a)?;
        Catalogue::new(self.catalogue.scope.clone(), self.catalogue.sources.clone())?;
        if self.retain_drafts.len() != self.catalogue.sources.len()
            || self.retain_drafts.len() > 32
            || self
                .catalogue
                .sources
                .iter()
                .map(|s| &s.path)
                .collect::<BTreeSet<_>>()
                .len()
                != self.catalogue.sources.len()
        {
            bail!("approval");
        }
        let mut total = 0;
        let mut min_reads = 0;
        for (d, b) in self.retain_drafts.iter().zip(&self.catalogue.sources) {
            if &d.binding != b {
                bail!("approval");
            }
            d.validate(policy, a)?;
            let bytes = d.text()?.len();
            total += bytes;
            min_reads += bytes.div_ceil(8192) + 1;
        }
        if total > 524288
            || self.serialized.len() > 1048576
            || self.serialized != packet(&self.catalogue, &self.retain_drafts)?
            || self.io.source_read_bytes != total as u64
            || self.io.source_read_calls < min_reads as u64
            || self.io.source_read_calls > 524288 + 32
            || self.io.opened_descriptors < (2 * self.retain_drafts.len()) as u64
            || self.io.opened_descriptors > 11000
            || self.io.closed_descriptors != self.io.opened_descriptors
            || self.io.close_errors != 0
        {
            bail!("approval");
        }
        active(a)
    }
    pub fn diagnostics(&self) -> Value {
        json!({"schema_version":1,"profile":PROFILE,"classification":"operator-attested-public","source_count":self.catalogue.sources.len(),"source_read_bytes":self.io.source_read_bytes,"source_read_calls":self.io.source_read_calls,"opened_descriptors":self.io.opened_descriptors,"closed_descriptors":self.io.closed_descriptors,"close_errors":self.io.close_errors,"export_packet_bytes":self.serialized.len(),"retain_draft_body_bytes":self.retain_drafts.iter().map(|d|d.body.len()).sum::<usize>(),"executed_transport_calls":0,"remote_tokens":null,"provider_tokens":null,"remote_usd":null,"total_cost_complete":false,"remote_cost_stages":{"retain":null,"extraction":null,"embeddings":null,"consolidation":null,"storage":null,"delete":null,"reconciliation":null,"retries":null}})
    }
}
pub fn build_public_export<P: PublicPolicy>(
    workspace: &Path,
    scope: Scope,
    approvals: &[PublicApproval],
    policy: &P,
    a: &mut dyn FnMut() -> bool,
) -> Result<PublicExport> {
    let mut io = IoObservation::default();
    let result = (|| -> Result<PublicExport> {
        if !workspace.is_absolute()
            || workspace
                .components()
                .any(|c| matches!(c, std::path::Component::ParentDir))
            || workspace.components().count() > 64
            || workspace.to_string_lossy().len() > 4096
            || workspace.to_string_lossy().chars().any(char::is_control)
            || approvals.len() > 32
            || approvals
                .iter()
                .map(|a| &a.path)
                .collect::<BTreeSet<_>>()
                .len()
                != approvals.len()
        {
            bail!("approval");
        }
        let mut drafts = vec![];
        let mut total = 0;
        for approval in approvals {
            active(a)?;
            if approval.scope != scope
                || approval.classification != "public"
                || !strict::digest(&approval.source_sha256)
            {
                bail!("approval");
            }
            source_path(&approval.path, policy, a)?;
            let raw = capture(workspace, approval, a, &mut io)?;
            total += raw.len();
            if total > 524288 {
                bail!("source_bound");
            }
            let text = text_integrity(&raw)?;
            screen(text, &approval.path, policy, a)?;
            let binding = SourceBinding::from_bytes(scope.clone(), &approval.path, &raw)?;
            let source_id = format!(
                "logical-{}",
                strict::sha(&wire::value(&[scope.key(), approval.path.clone()])?)
            );
            let body = retain_body(&binding, &source_id, text)?;
            if body.len() > 262144 {
                bail!("request_bound");
            }
            drafts.push(RetainDraft {
                binding,
                source_id,
                body,
            });
        }
        let catalogue = Catalogue::new(scope, drafts.iter().map(|d| d.binding.clone()).collect())?;
        let serialized = packet(&catalogue, &drafts)?;
        let export = PublicExport {
            catalogue,
            retain_drafts: drafts,
            serialized,
            io: io.clone(),
        };
        export.validate(policy, a)?;
        Ok(export)
    })();
    result.map_err(|error| fixed_error(error, Some(io)))
}
#[derive(Clone)]
pub struct IngestionPlan {
    pub current: Catalogue,
    pub current_retain_drafts: Vec<RetainDraft>,
    pub retain_candidates: Vec<RetainDraft>,
    pub retire_candidates: Vec<SourceBinding>,
    pub reconciliation: Vec<SourceBinding>,
    pub input_match_count: usize,
}
impl IngestionPlan {
    pub fn diagnostics(&self) -> Value {
        json!({"schema_version":1,"profile":PROFILE,"state":"reconciliation_required","input_match_count":self.input_match_count,"retain_candidate_count":self.retain_candidates.len(),"retire_candidate_count":self.retire_candidates.len(),"reconciliation_draft_count":self.reconciliation.len(),"authorized_mutations":0,"remote_noop":null,"executed_transport_calls":0,"remote_cost_complete":false})
    }
    pub fn reconciliation_drafts(&self) -> Vec<(String, String)> {
        self.reconciliation
            .iter()
            .map(|s| {
                (
                    "GET".into(),
                    format!(
                        "/v1/default/banks/{}/documents/{}",
                        s.scope.bank_id(),
                        s.document_id
                    ),
                )
            })
            .collect()
    }
}
pub fn plan_public_ingestion<P: PublicPolicy>(
    export: &PublicExport,
    previous: Option<&Catalogue>,
    policy: &P,
    a: &mut dyn FnMut() -> bool,
) -> Result<IngestionPlan> {
    export.validate(policy, a)?;
    let empty = Catalogue::new(export.catalogue.scope.clone(), vec![])?;
    let previous = previous.unwrap_or(&empty);
    if previous.scope != export.catalogue.scope
        || previous.sources.iter().any(|b| b.scope != previous.scope)
    {
        bail!("previous_catalogue");
    }
    for b in &previous.sources {
        active(a)?;
        b.validate_integrity()?;
    }
    let old = previous
        .sources
        .iter()
        .map(|s| &s.document_id)
        .collect::<BTreeSet<_>>();
    let new = export
        .catalogue
        .sources
        .iter()
        .map(|s| &s.document_id)
        .collect::<BTreeSet<_>>();
    let mut reconciliation = previous.sources.clone();
    reconciliation.extend(
        export
            .catalogue
            .sources
            .iter()
            .filter(|s| !old.contains(&s.document_id))
            .cloned(),
    );
    Ok(IngestionPlan {
        current: export.catalogue.clone(),
        current_retain_drafts: export.retain_drafts.clone(),
        retain_candidates: export
            .retain_drafts
            .iter()
            .filter(|d| !old.contains(&d.binding.document_id))
            .cloned()
            .collect(),
        retire_candidates: previous
            .sources
            .iter()
            .filter(|s| !new.contains(&s.document_id))
            .cloned()
            .collect(),
        reconciliation,
        input_match_count: new.intersection(&old).count(),
    })
}
