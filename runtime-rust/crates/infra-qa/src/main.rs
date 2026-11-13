fn main() {
    let mut args = std::env::args().skip(1);
    let command = args.next().unwrap_or_default();
    match infra_qa::run(&command, &args.collect::<Vec<_>>()) {
        Ok(code) => std::process::exit(code),
        Err(e) => {
            eprintln!("QA operation refused: {e:#}");
            std::process::exit(2)
        }
    }
}
