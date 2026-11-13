//! Embeds the content identity of the sources this engine compiles; the
//! Harness restarts after an update only on an engine whose identity matches
//! the clone (see `infra-tooling/src/source_identity.rs`).
#[allow(dead_code)]
#[path = "../infra-tooling/src/source_identity.rs"]
mod source_identity;

fn main() {
    let manifest =
        std::path::PathBuf::from(std::env::var_os("CARGO_MANIFEST_DIR").unwrap_or_default());
    // runtime-rust/crates/infra-cli lies three levels below the repository.
    let identity = match manifest.ancestors().nth(3).map(source_identity::engine) {
        Some(Ok(identity)) => {
            for path in &identity.watched {
                println!("cargo::rerun-if-changed={}", path.display());
            }
            identity.sha256
        }
        Some(Err(error)) => {
            println!("cargo::warning=infra-engine records no source identity: {error}");
            String::new()
        }
        None => String::new(),
    };
    println!("cargo::rustc-env=INFRA_ENGINE_SOURCE_SHA256={identity}");
}
