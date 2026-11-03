//! Windows capture keeps native exit separate from Job cleanup and output EOF.
//! Capture-only read handles use OVERLAPPED operations with bounded retirement.
use super::{NativeExit, Outcome, Request, Stream, cancelled, overflow, timeout, unconfirmed};
use crate::{HarnessError, Result, process::CaptureChild};
use std::{
    sync::mpsc,
    time::{Duration, Instant},
};

fn admit(deadline: Instant, interrupted: &mut impl FnMut() -> bool) -> Result<()> {
    if interrupted() {
        return Err(cancelled());
    }
    if Instant::now() >= deadline {
        return Err(timeout());
    }
    Ok(())
}

fn poll_writer(receiver: &mpsc::Receiver<std::io::Result<()>>, done: &mut bool) -> Result<()> {
    if *done {
        return Ok(());
    }
    match receiver.try_recv() {
        Ok(result) => {
            *done = true;
            match result {
                Ok(()) => Ok(()),
                Err(error) if error.kind() == std::io::ErrorKind::BrokenPipe => Ok(()),
                Err(error) => Err(error.into()),
            }
        }
        Err(mpsc::TryRecvError::Empty) => Ok(()),
        Err(mpsc::TryRecvError::Disconnected) => {
            *done = true;
            Err(HarnessError::unavailable(
                "Captured stdin worker stopped without a result.",
            ))
        }
    }
}

async fn finish_writer(
    receiver: &mpsc::Receiver<std::io::Result<()>>,
    done: &mut bool,
) -> Result<()> {
    let deadline = Instant::now() + Duration::from_secs(1);
    loop {
        poll_writer(receiver, done)?;
        if *done {
            return Ok(());
        }
        if Instant::now() >= deadline {
            CaptureChild::stop_admission();
            return Err(unconfirmed());
        }
        tokio::time::sleep(Duration::from_millis(10)).await;
    }
}

pub(super) async fn run_async(
    request: Request<'_>,
    mut interrupted: impl FnMut() -> bool + Send,
    mut observed_exit: impl FnMut(NativeExit) + Send,
    mut event: impl FnMut(Stream, &[u8]) -> Result<()> + Send,
) -> Result<Outcome> {
    let started = Instant::now();
    if request.output_limit == 0 {
        return Err(HarnessError::invalid(
            "Captured output limit must be positive.",
        ));
    }
    admit(request.deadline, &mut interrupted)?;
    let mut child = CaptureChild::spawn(request.argv, request.cwd, request.environment)?;
    // Every fallible action after launch stays inside this scope. Setup
    // failure still performs explicit cleanup and propagates unconfirmed
    // custody instead of relying on a destructor that cannot return its error.
    let setup = (|| -> Result<_> {
        let mut stdin = child
            .stdin
            .take()
            .ok_or_else(|| HarnessError::unavailable("Captured stdin pipe is missing."))?;
        let stdout = child
            .stdout
            .take()
            .ok_or_else(|| HarnessError::unavailable("Captured stdout pipe is missing."))?;
        let stderr = child
            .stderr
            .take()
            .ok_or_else(|| HarnessError::unavailable("Captured stderr pipe is missing."))?;
        let input = request.input.to_vec();
        let (writer_send, writer_result) = mpsc::sync_channel(1);
        // The guardian closes its own stdin after launch. Job cleanup closes
        // every native reader, releasing backpressured input without a join.
        std::thread::Builder::new()
            .name("infra-capture-stdin".into())
            .spawn(move || {
                let result = std::io::Write::write_all(&mut stdin, &input);
                drop(stdin);
                let _ = writer_send.send(result);
            })?;
        Ok((stdout, stderr, writer_result))
    })();
    let (mut stdout, mut stderr, writer_result) = match setup {
        Ok(setup) => setup,
        Err(error) => {
            let (cleanup, native) = child.finish().await;
            if let Ok(Some(code)) = &native {
                observed_exit(NativeExit {
                    code: Some(*code),
                    signal: None,
                    shell_code: *code,
                });
            }
            if matches!(&cleanup, Err(error) if error.code == "cleanup-unconfirmed")
                || matches!(&native, Err(error) if error.code == "cleanup-unconfirmed")
            {
                return Err(unconfirmed());
            }
            return Err(error);
        }
    };
    let mut writer_done = false;
    let mut total = 0usize;
    let mut exit_observed = false;
    let execution: Result<i32> = async {
        loop {
            admit(request.deadline, &mut interrupted)?;
            poll_writer(&writer_result, &mut writer_done)?;
            let native_exit = child.poll_native_exit()?;
            if let Some(code) = native_exit
                && !exit_observed
            {
                exit_observed = true;
                observed_exit(NativeExit {
                    code: Some(code),
                    signal: None,
                    shell_code: code,
                });
                admit(request.deadline, &mut interrupted)?;
            }
            for (stream, pipe) in [(Stream::Stdout, &mut stdout), (Stream::Stderr, &mut stderr)] {
                admit(request.deadline, &mut interrupted)?;
                if let Some(bytes) = pipe.poll()? {
                    total = total
                        .checked_add(bytes.len())
                        .filter(|value| *value <= request.output_limit)
                        .ok_or_else(overflow)?;
                    event(stream, &bytes)?;
                    admit(request.deadline, &mut interrupted)?;
                }
            }
            if let Some(code) = native_exit
                && stdout.eof()
                && stderr.eof()
            {
                admit(request.deadline, &mut interrupted)?;
                break Ok(code);
            }
            let pause = request
                .deadline
                .saturating_duration_since(Instant::now())
                .min(Duration::from_millis(10));
            tokio::time::sleep(pause).await;
        }
    }
    .await;

    // Native exit does not trigger this cleanup. Both EOFs, an admitted stop,
    // callback error, cap or deadline must end the execution loop first.
    let (cleanup, native) = child.finish().await;
    if let Ok(Some(code)) = &native
        && !exit_observed
    {
        observed_exit(NativeExit {
            code: Some(*code),
            signal: None,
            shell_code: *code,
        });
    }
    // Every OVERLAPPED and buffer remains owned until actual I/O completion.
    // CancelIoEx alone is never permission to free or reuse them.
    let (stdout_retired, stderr_retired, writer) = tokio::join!(
        stdout.settle(),
        stderr.settle(),
        finish_writer(&writer_result, &mut writer_done),
    );
    if matches!(&native, Err(error) if error.code == "cleanup-unconfirmed") {
        return Err(unconfirmed());
    }
    for result in [&cleanup, &stdout_retired, &stderr_retired, &writer] {
        if let Err(error) = result
            && error.code == "cleanup-unconfirmed"
        {
            return Err(unconfirmed());
        }
    }
    let code = execution?;
    cleanup?;
    native?;
    stdout_retired?;
    stderr_retired?;
    writer?;
    Ok(Outcome {
        exit: Some(code),
        signal: None,
        shell_code: code,
        seconds: started.elapsed().as_secs_f64(),
    })
}
