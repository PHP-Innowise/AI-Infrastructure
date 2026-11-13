//! Owned command output; delivery receipts are completed only after stdout is written.
use infra_core::{Result, RuntimeError, observability::RenderFinalizer};
use serde_json::Value;
use std::io::Write;
#[derive(Clone, Debug, Default)]
pub struct Output {
    pub code: i32,
    pub stdout: Vec<u8>,
    pub stderr: Vec<u8>,
    pub after_delivery: Option<RenderFinalizer>,
}
impl Output {
    pub fn text(value: impl Into<String>) -> Self {
        Self {
            code: 0,
            stdout: value.into().into_bytes(),
            ..Default::default()
        }
    }
    pub fn json(value: &Value) -> Result<Self> {
        let mut bytes = serde_json::to_vec(value)?;
        bytes.push(b'\n');
        Ok(Self {
            code: 0,
            stdout: bytes,
            ..Default::default()
        })
    }
    pub fn error(code: i32, message: impl AsRef<str>) -> Self {
        Self {
            code,
            stderr: format!("{}\n", message.as_ref()).into_bytes(),
            ..Default::default()
        }
    }
    pub fn envelope(value: Value) -> Result<Self> {
        if let Some(render) = value.get("_cli") {
            let code = render["code"]
                .as_i64()
                .and_then(|value| i32::try_from(value).ok())
                .ok_or_else(|| RuntimeError::invalid("Invalid command status"))?;
            let (stdout, stderr) = (
                render["stdout"]
                    .as_str()
                    .ok_or_else(|| RuntimeError::invalid("Invalid command stdout"))?,
                render["stderr"]
                    .as_str()
                    .ok_or_else(|| RuntimeError::invalid("Invalid command stderr"))?,
            );
            return Ok(Self {
                code,
                stdout: stdout.as_bytes().to_vec(),
                stderr: stderr.as_bytes().to_vec(),
                ..Default::default()
            });
        }
        if value["stdio_inherited"] == true {
            return Ok(Self {
                code: value["exit_code"]
                    .as_i64()
                    .and_then(|value| i32::try_from(value).ok())
                    .unwrap_or(0),
                ..Default::default()
            });
        }
        Self::json(&value)
    }
    pub fn write(mut self) -> std::io::Result<i32> {
        {
            let mut stdout = std::io::stdout().lock();
            stdout.write_all(&self.stdout)?;
            stdout.flush()?;
        }
        if let Some(finalizer) = self.after_delivery.take() {
            finalizer.finish();
        }
        {
            let mut stderr = std::io::stderr().lock();
            stderr.write_all(&self.stderr)?;
            stderr.flush()?;
        }
        Ok(self.code)
    }
}
