//! The engine names the sources it was compiled from, and assembly carries
//! that name into the descriptor a Harness update compares with its clone.
use infra_tooling::source_identity;
use serde_json::Value;
use std::{path::Path, process::Command};

fn engine(arguments: &[&str], directory: &Path) -> Value {
    let output = Command::new(env!("CARGO_BIN_EXE_infra-engine"))
        .args(arguments)
        .current_dir(directory)
        .output()
        .unwrap();
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    serde_json::from_slice(&output.stdout).unwrap()
}

#[test]
fn the_engine_and_its_assembled_descriptor_name_the_tree_it_was_built_from() {
    let repository = Path::new(env!("CARGO_MANIFEST_DIR"))
        .ancestors()
        .nth(3)
        .unwrap();
    let identity = source_identity::engine(repository).unwrap();
    let capabilities = engine(&["capabilities", "--json"], repository);
    const {
        assert!(
            !cfg!(feature = "onnx") || infra_pilots::onnx::compiled(),
            "the requested inference feature must reach the worker dependency"
        );
    }
    assert_eq!(
        capabilities["capabilities"]
            .as_array()
            .unwrap()
            .iter()
            .filter(|capability| capability.as_str() == Some("onnx-inference-v1"))
            .count(),
        usize::from(infra_pilots::onnx::compiled()),
        "advertise the compiled dependency, including unified workspace features"
    );
    assert_eq!(capabilities["capability_profile"], "infra-engine-v1");
    assert_eq!(capabilities["source"]["scheme"], source_identity::SCHEME);
    assert_eq!(
        capabilities["source"]["sha256"],
        identity.sha256.as_str(),
        "the build script embedded the identity of another tree"
    );
    // Without --write assembly only reports the descriptor it would write.
    let assembly = engine(&["repo", "artifacts", "--edition", "PHP Core"], repository);
    assert_eq!(assembly["write"], false);
    assert_eq!(
        assembly["artifacts"][0]["artifact"]["source"],
        capabilities["source"]
    );
    assert_eq!(
        assembly["artifacts"][0]["artifact"]["capabilities"], capabilities["capabilities"],
        "release admission must preserve optional inference capabilities"
    );
}
