fn main() {
    let arguments = std::env::args().skip(1).collect::<Vec<_>>();
    let result = infra_cli::run(&arguments);
    let output = match result {
        Ok(output) => output,
        Err(error) => {
            let code = if error.kind == infra_core::ErrorKind::InvalidInput {
                2
            } else {
                1
            };
            infra_cli::output::Output::error(code, format!("infra-engine: {error}"))
        }
    };
    let code = match output.write() {
        Ok(code) => code,
        Err(error) if error.kind() == std::io::ErrorKind::BrokenPipe => 0,
        Err(_) => 1,
    };
    std::process::exit(code);
}
