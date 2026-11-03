use memory_kernel_probe::{MAX_INPUT_BYTES, ProtocolError, evaluate};
use std::io::{self, Read, Write};
use std::process::ExitCode;

fn run() -> Result<Vec<u8>, ProtocolError> {
    let mut arguments = std::env::args_os().skip(1);
    match (arguments.next(), arguments.next()) {
        (None, None) => {}
        (Some(argument), None) if argument == "--version" => {
            return Ok(concat!(
                "{\"protocol\":1,\"version\":\"",
                env!("CARGO_PKG_VERSION"),
                "\"}\n"
            )
            .as_bytes()
            .to_vec());
        }
        _ => return Err(ProtocolError::InvalidArguments),
    }
    let mut input = Vec::new();
    io::stdin()
        .lock()
        .take((MAX_INPUT_BYTES + 1) as u64)
        .read_to_end(&mut input)
        .map_err(|_| ProtocolError::Io)?;
    evaluate(&input)
}

fn main() -> ExitCode {
    let (output, status) = match run() {
        Ok(output) => (output, 0),
        Err(error) => (error.response().to_vec(), 2),
    };
    let mut stdout = io::stdout().lock();
    if stdout
        .write_all(&output)
        .and_then(|_| stdout.flush())
        .is_err()
    {
        // The response channel itself may be unavailable. Never emit input or
        // diagnostics to stderr, including on a closed stdout pipe.
        return ExitCode::from(2);
    }
    ExitCode::from(status)
}
