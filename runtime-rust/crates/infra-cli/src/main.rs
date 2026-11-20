fn main() {
    let arguments = std::env::args().skip(1).collect::<Vec<_>>();
    // A warm SkillCache session stages this executable under the names of the
    // plan executor and the program tool. The guardian of every process they
    // start is this same file under the same name, so its internal commands
    // are dispatched as usual.
    if let Some(tool) = infra_cli::staged_tool(std::env::args_os().next().as_deref())
        && !matches!(
            arguments.first().map(String::as_str),
            Some("internal-guard" | "internal")
        )
    {
        std::process::exit(infra_cli::skillcache(tool, &arguments));
    }
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
