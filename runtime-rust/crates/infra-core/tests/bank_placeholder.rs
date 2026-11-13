//! Native empty-bank placeholders are exact data, not a dotfile exemption.
use infra_core::{BankService, Roots, RuntimeContext};
use std::{fs, path::PathBuf};

struct Fixture {
    root: PathBuf,
    bank: BankService,
}
impl Fixture {
    fn new() -> Self {
        let root = std::env::temp_dir().join(format!(
            "infra-bank-placeholder-{}",
            infra_core::domain::uuid4()
        ));
        fs::create_dir_all(root.join("memory-bank/chunks")).unwrap();
        fs::write(root.join("memory-bank/README.md"), "# Memory Bank\n").unwrap();
        fs::write(root.join("memory-bank/INDEX.md"), "# Memory Index\n").unwrap();
        let context =
            RuntimeContext::new(Roots::installed(&root).unwrap(), "local", "governed").unwrap();
        Self {
            root,
            bank: BankService::new(context),
        }
    }
    fn placeholder(&self) -> PathBuf {
        self.root.join("memory-bank/chunks/.gitkeep")
    }
    fn valid(&self) -> bool {
        self.bank.validate().unwrap().is_empty()
    }
    fn unexpected(&self) -> bool {
        self.bank
            .validate()
            .unwrap()
            .iter()
            .any(|error| error.contains("unexpected chunk entry"))
    }
}
impl Drop for Fixture {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.root);
    }
}

#[test]
fn exact_empty_placeholder_is_accepted_but_content_and_other_dotfiles_are_not() {
    let fixture = Fixture::new();
    assert!(fixture.valid(), "an empty bank is valid");
    fs::write(fixture.placeholder(), []).unwrap();
    assert!(
        fixture.valid(),
        "the real native seed placeholder must be valid"
    );
    fs::write(fixture.placeholder(), b"\n").unwrap();
    assert!(
        fixture.unexpected(),
        "even whitespace makes a placeholder nonempty"
    );
    fs::remove_file(fixture.placeholder()).unwrap();
    fs::write(fixture.root.join("memory-bank/chunks/.keep"), []).unwrap();
    assert!(fixture.unexpected(), "unrelated dotfiles remain unexpected");
    fs::remove_file(fixture.root.join("memory-bank/chunks/.keep")).unwrap();
    fs::write(
        fixture.root.join("memory-bank/chunks/invalid-name.md"),
        "Untracked memory.\n",
    )
    .unwrap();
    assert!(
        fixture.unexpected(),
        "ordinary unknown files remain unexpected"
    );
    fs::remove_file(fixture.root.join("memory-bank/chunks/invalid-name.md")).unwrap();
    fs::create_dir(fixture.root.join("memory-bank/chunks/nested")).unwrap();
    assert!(fixture.unexpected(), "nested directories remain unexpected");
}

#[test]
fn a_placeholder_directory_is_not_a_file_exemption() {
    let fixture = Fixture::new();
    fs::create_dir(fixture.placeholder()).unwrap();
    assert!(fixture.unexpected());
}

#[cfg(unix)]
#[test]
fn a_placeholder_symlink_or_hardlink_does_not_gain_admission() {
    let fixture = Fixture::new();
    let outside = fixture.root.join("outside-placeholder");
    fs::write(&outside, []).unwrap();
    std::os::unix::fs::symlink(&outside, fixture.placeholder()).unwrap();
    assert!(
        fixture.unexpected(),
        "an empty linked target is still refused"
    );
    fs::remove_file(fixture.placeholder()).unwrap();
    fs::hard_link(&outside, fixture.placeholder()).unwrap();
    assert!(
        fixture.unexpected(),
        "an empty multiply-linked inode is still refused"
    );
}
