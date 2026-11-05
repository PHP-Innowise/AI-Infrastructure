//! Prompt-only next-turn delivery; the provider process has no live stdin here.
use super::*;

pub(super) fn stamp(lease: &Lease) -> String {
    store::sha(format!("{lease:?}").as_bytes())
}

impl Sessions {
    pub(super) fn start_inbox(self: &Arc<Self>) {
        let runner = Arc::downgrade(self);
        let wake = self.inbox_wake.clone();
        let done = self.inbox_done.clone();
        tokio::spawn(async move {
            let mut retry = false;
            loop {
                if retry {
                    tokio::select! {
                        _ = wake.notified() => {},
                        _ = tokio::time::sleep(std::time::Duration::from_millis(250)) => {},
                    }
                } else {
                    wake.notified().await;
                }
                let Some(runner) = runner.upgrade() else {
                    break;
                };
                if runner.stopping.load(Ordering::Acquire) {
                    break;
                }
                match runner.dispatch_inbox().await {
                    Ok(waiting) => retry = waiting,
                    Err(_) => {
                        runner.stop_failed(&Arc::new(AtomicBool::new(false)));
                        break;
                    }
                }
            }
            done.store(true, Ordering::Release);
        });
    }
    fn inbox_open(&self) -> Result<()> {
        if !self.ready.load(Ordering::Acquire) || self.stopping.load(Ordering::Acquire) {
            return Err(HarnessError::conflict(
                "The session inbox is not accepting work.",
            ));
        }
        if let Some(reason) = lock(&self.held)?.as_ref() {
            return Err(HarnessError::conflict(reason.clone()));
        }
        Ok(())
    }
    pub(super) async fn accept_inbox(
        &self,
        sid: &str,
        request: &str,
        prompt: &str,
    ) -> Result<Value> {
        let prompt = crate::inbox::validate_request(request, prompt)?;
        if let Some(receipt) = self.store.inbox_existing(sid, request, &prompt)? {
            return Ok(receipt);
        }
        let session = self.store.get(sid)?;
        let frozen = crate::inbox::authority(&session)?;
        let generation = self.store.generation(sid)?;
        self.commands()?
            .check(session["provider"].as_str().unwrap_or(""), &prompt, None)?;
        let lease = self
            .job_lease(Job {
                sid: sid.into(),
                prompt: prompt.clone(),
                generation: generation.clone(),
                kind: JobKind::Run,
            })
            .await;
        if !lease.parallel() {
            return Err(HarnessError::conflict(
                "Inbox follow-ups require a verified, unchanged Git workspace.",
            ));
        }
        let _admission = lock(&self.admission)?;
        if let Some(receipt) = self.store.inbox_existing(sid, request, &prompt)? {
            return Ok(receipt);
        }
        self.inbox_open()?;
        let running: bool = self.store.connection()?.query_row(
            "SELECT EXISTS(SELECT 1 FROM launches WHERE id=?1 AND session_id=?2 AND kind='native' AND status='running')",
            params![generation,sid], |row| row.get(0),
        )?;
        if !running {
            return Err(HarnessError::conflict(
                "Wait for an ordinary native turn to start before queuing a follow-up.",
            ));
        }
        let receipt =
            self.store
                .inbox_accept(sid, request, &prompt, &generation, &frozen, &stamp(&lease))?;
        self.inbox_wake.notify_one();
        Ok(receipt)
    }
    pub(super) async fn dispatch_inbox(&self) -> Result<bool> {
        if !self.ready.load(Ordering::Acquire) || self.stopping.load(Ordering::Acquire) {
            return Ok(false);
        }
        let mut retry = false;
        for item in self.store.inbox_pending()? {
            if self.stopping.load(Ordering::Acquire) {
                break;
            }
            let sid = store::text(&item["session_id"], "session_id", 128)?;
            let request = store::text(&item["request_id"], "request_id", 128)?;
            let source = store::text(&item["source_generation"], "source_generation", 128)?;
            let session = match self.store.get(&sid) {
                Ok(session) => session,
                Err(error) if error.status == 404 => {
                    self.store.inbox_block(
                        &sid,
                        &request,
                        "The source session no longer exists.",
                    )?;
                    continue;
                }
                Err(error) => return Err(error),
            };
            if self.store.generation(&sid)? != source {
                self.store.inbox_block(
                    &sid,
                    &request,
                    "Another turn replaced the source generation.",
                )?;
                continue;
            }
            if active(&session) {
                continue;
            }
            if session["status"] != "completed" {
                self.store.inbox_block(
                    &sid,
                    &request,
                    "The source turn requires review before continuing.",
                )?;
                continue;
            }
            if item["source_ready"] != true {
                continue;
            }
            if lock(&self.active)?.contains_key(&sid) {
                retry = true;
                continue;
            }
            let frozen = match crate::inbox::authority(&session) {
                Ok(frozen) => frozen,
                Err(_) => {
                    self.store
                        .inbox_block(&sid, &request, "The session authority changed.")?;
                    continue;
                }
            };
            let prompt = item["prompt"].as_str().unwrap_or("");
            self.commands()?
                .check(session["provider"].as_str().unwrap_or(""), prompt, None)?;
            let lease = self
                .job_lease(Job {
                    sid: sid.clone(),
                    prompt: prompt.into(),
                    generation: source,
                    kind: JobKind::Run,
                })
                .await;
            if !lease.parallel() {
                self.store
                    .inbox_block(&sid, &request, "The execution scope is unavailable.")?;
                continue;
            }
            let Ok(_operation) = self.operation.try_lock() else {
                retry = true;
                continue;
            };
            let _admission = lock(&self.admission)?;
            if self.inbox_open().is_err() {
                return Ok(false);
            }
            if lock(&self.active)?.contains_key(&sid) {
                retry = true;
                continue;
            }
            if self.queue.capacity() == 0 {
                retry = true;
                break;
            }
            match self
                .store
                .inbox_claim(&sid, &request, &stamp(&lease), &frozen)?
            {
                Some(turn) => {
                    let job = Job {
                        sid: turn.session["id"].as_str().unwrap_or("").into(),
                        prompt: turn.prompt,
                        generation: turn.generation.clone(),
                        kind: JobKind::Run,
                    };
                    if self.queue.try_send(job).is_err() {
                        self.store.inbox_notification_failed(
                            &sid,
                            &turn.generation,
                            "The runner stopped before the assigned turn was notified.",
                        )?;
                        return Err(HarnessError::unavailable(
                            "The assigned inbox turn could not reach the runner.",
                        ));
                    }
                }
                None => retry = true,
            }
        }
        Ok(retry)
    }
}
