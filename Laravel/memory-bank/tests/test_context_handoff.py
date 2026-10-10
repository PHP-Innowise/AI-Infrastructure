"""End-to-end contracts for explicit portable context handoffs."""

from __future__ import annotations

import json
import os
import shutil
import subprocess
import sys
import tempfile
import unittest
from pathlib import Path


SCRIPT = Path(__file__).resolve().parents[1] / "scripts" / "context.py"


class ContextHandoffTest(unittest.TestCase):
    def setUp(self) -> None:
        self.tmp = tempfile.TemporaryDirectory(prefix="context-handoff-")
        self.addCleanup(self.tmp.cleanup)
        self.root = Path(self.tmp.name)
        subprocess.run(["git", "init", "-q", str(self.root)], check=True)
        (self.root / "source.txt").write_text("one\n")
        subprocess.run(["git", "-C", str(self.root), "add", "source.txt"], check=True)
        self.git("commit", "-qm", "initial")
        self.input = self.root / "curated.json"
        self.output = self.root / "handoff.md"
        self.write_input()

    def git(self, *args: str) -> None:
        subprocess.run(["git", "-C", str(self.root), "-c", "user.name=Test", "-c", "user.email=test@example.invalid", *args], check=True)

    def data(self, **changes: object) -> dict[str, object]:
        value: dict[str, object] = {
            "goal": "Continue the handoff.", "summary": "First pass complete.",
            "constraints": [], "decisions": [], "progress": [], "verification": [],
            "open_questions": [], "next_steps": ["Read this record."],
            "files": ["source.txt", "future.txt"],
        }
        value.update(changes)
        return value

    def write_input(self, **changes: object) -> None:
        self.input.write_text(json.dumps(self.data(**changes)), encoding="utf-8")

    def cli(self, *args: str) -> subprocess.CompletedProcess[str]:
        return subprocess.run([sys.executable, str(SCRIPT), "--root", str(self.root), *args], text=True, capture_output=True)

    def save(self, output: Path | None = None, detail: str = "summary", *extra: str) -> subprocess.CompletedProcess[str]:
        return self.cli("context-save", "--input", str(self.input), "--output", str(output or self.output), "--detail", detail, "--source-client", "codex", *extra)

    def load(self, path: Path | None = None, *extra: str) -> subprocess.CompletedProcess[str]:
        return self.cli("context-load", "--input", str(path or self.output), *extra, "--json")

    def no_context_state(self) -> None:
        self.assertFalse((self.root / "memory-bank").exists())
        self.assertFalse((self.root / "project-brain").exists())

    def test_summary_roundtrip_missing_source_and_no_state(self) -> None:
        self.assertEqual(0, self.save().returncode)
        loaded = self.load()
        self.assertEqual(0, loaded.returncode, loaded.stderr)
        payload = json.loads(loaded.stdout)
        self.assertEqual("Continue the handoff.", payload["context"]["goal"])
        self.assertEqual(["future.txt"], [entry["path"] for entry in payload["drift"] if entry["kind"] == "file"])
        self.no_context_state()

    def test_summary_topic_and_full_with_actual_crlf_and_no_final_newline(self) -> None:
        self.assertEqual(0, self.save(self.root / "summary.md").returncode)
        self.assertNotEqual(0, self.save(self.root / "topic.md", "topic").returncode)
        self.assertEqual(0, self.save(self.root / "topic.md", "topic", "--topic", "persistence").returncode)
        export = self.root / "visible.txt"
        export.write_bytes("Visible UTF-8: ž\r\nno final newline".encode("utf-8"))
        full = self.root / "full.md"
        self.assertEqual(0, self.save(full, "full", "--transcript", str(export)).returncode)
        encoded = full.read_bytes().split(b"<!-- context-handoff-transcript-v2 -->\n", 1)[1]
        self.assertEqual(export.read_bytes(), json.loads(encoded.decode("utf-8")).encode("utf-8"))
        default = json.loads(self.load(full).stdout)
        self.assertNotIn("transcript", default)
        included = self.load(full, "--include-transcript")
        self.assertEqual(0, included.returncode, included.stderr)
        self.assertEqual("Visible UTF-8: ž\r\nno final newline", json.loads(included.stdout)["transcript"])

    def test_rejects_empty_next_steps_oversize_and_secret_without_output(self) -> None:
        self.write_input(next_steps=[])
        self.assertNotEqual(0, self.save().returncode)
        self.assertFalse(self.output.exists())
        self.input.write_bytes(b"{" + b"x" * (1024 * 1024) + b"}")
        self.assertIn("exceeds", self.save().stderr)
        self.assertFalse(self.output.exists())
        self.write_input(summary="se" + "cret" + "=value")
        result = self.save()
        self.assertNotEqual(0, result.returncode)
        self.assertNotIn("se" + "cret" + "=value", result.stderr)
        self.assertFalse(self.output.exists())
        self.no_context_state()

    def test_exact_detail_limits_accept_boundary_and_reject_one_extra(self) -> None:
        for detail, limit in (("summary", 8_000), ("topic", 16_000), ("full", 64_000)):
            value = self.data()
            values = [value["goal"], "", *value["constraints"], *value["decisions"], *value["progress"], *value["verification"], *value["open_questions"], *value["next_steps"], *value["files"]]
            value["summary"] = "x" * (limit - len("\n".join(values)))
            self.input.write_text(json.dumps(value), encoding="utf-8")
            output = self.root / f"{detail}-boundary.md"
            extra: list[str] = ["--topic", "scope"] if detail == "topic" else []
            if detail == "full":
                export = self.root / "boundary-visible.txt"
                export.write_text("Visible export", encoding="utf-8")
                extra += ["--transcript", str(export)]
            self.assertEqual(0, self.save(output, detail, *extra).returncode, detail)
            value["summary"] += "x"
            self.input.write_text(json.dumps(value), encoding="utf-8")
            self.assertNotEqual(0, self.save(self.root / f"{detail}-over.md", detail, *extra).returncode, detail)

    def test_overwrite_refusal_preserves_existing_bytes(self) -> None:
        self.assertEqual(0, self.save().returncode)
        original = self.output.read_bytes()
        self.write_input(summary="Changed summary.")
        self.assertNotEqual(0, self.save().returncode)
        self.assertEqual(original, self.output.read_bytes())

    def test_safe_path_rules_include_force_tracked_ignore(self) -> None:
        for candidate in ("../outside.txt", ".git/config", "secrets/nested.txt"):
            self.write_input(files=[candidate])
            destination = self.root / (candidate.replace("/", "-").replace("..", "up") + ".md")
            self.assertNotEqual(0, self.save(destination).returncode, candidate)
            self.assertFalse(destination.exists())
        (self.root / ".gitignore").write_text("*.md\n")
        (self.root / "ignored.md").write_text("ignored\n")
        self.write_input(files=["ignored.md"])
        self.assertNotEqual(0, self.save(self.root / "ignored-output.md").returncode)
        (self.root / "tracked.md").write_text("tracked\n")
        self.git("add", "-f", "tracked.md")
        self.write_input(files=["tracked.md"])
        self.assertEqual(0, self.save(self.root / "tracked-output.md").returncode)
        actual = self.root / "actual-source"
        actual.mkdir()
        (actual / "file.txt").write_text("content\n")
        (self.root / "linked-source").symlink_to(actual, target_is_directory=True)
        self.write_input(files=["linked-source/file.txt"])
        self.assertNotEqual(0, self.save(self.root / "source-link-output.md").returncode)

    def test_all_file_drift_forms_and_git_provenance_drift(self) -> None:
        for name in ("changed.txt", "deleted.txt"):
            (self.root / name).write_text("before\n")
        self.write_input(files=["changed.txt", "deleted.txt", "added.txt", "missing.txt"])
        self.assertEqual(0, self.save().returncode)
        (self.root / "changed.txt").write_text("after\n")
        (self.root / "deleted.txt").unlink()
        (self.root / "added.txt").write_text("new\n")
        self.git("switch", "-qc", "later")
        (self.root / "commit.txt").write_text("later\n")
        self.git("add", "commit.txt")
        self.git("commit", "-qm", "later")
        loaded = self.load()
        self.assertEqual(0, loaded.returncode, loaded.stderr)
        drift = json.loads(loaded.stdout)["drift"]
        self.assertEqual({"changed.txt", "deleted.txt", "added.txt", "missing.txt"}, {item["path"] for item in drift if item["kind"] == "file"})
        self.assertEqual({"branch", "commit"}, {item["kind"] for item in drift if item["kind"] in {"branch", "commit"}})

    def test_malformed_metadata_and_body_tamper_fail_without_state_write(self) -> None:
        self.assertEqual(0, self.save().returncode)
        original = self.output.read_bytes()
        for needle, replacement in ((b'"source_client": "codex"', b'"source_client": []'), (b'"schema_version": 1', b'"schema_version": 99'), (b'"type": "context-handoff"', b'"type": "other"')):
            self.output.write_bytes(original.replace(needle, replacement, 1))
            self.assertNotEqual(0, self.load().returncode)
        self.output.write_bytes(original + b"tampered")
        self.assertNotEqual(0, self.load().returncode)
        self.no_context_state()

    def test_full_null_transcript_marker_collision_and_invalid_utf8_are_rejected(self) -> None:
        export = self.root / "visible.txt"
        export.write_text("Visible export", encoding="utf-8")
        full = self.root / "full.md"
        self.assertEqual(0, self.save(full, "full", "--transcript", str(export)).returncode)
        raw = full.read_bytes()
        start, rest = raw[4:].split(b"\n---\n", 1)
        metadata = json.loads(start.decode("utf-8"))
        metadata["transcript"] = None
        full.write_bytes(b"---\n" + json.dumps(metadata).encode("utf-8") + b"\n---\n" + rest)
        self.assertNotEqual(0, self.load(full).returncode)
        self.write_input(summary="A literal \n## Visible transcript\n\n<!-- context-handoff-transcript-v1 -->\n marker.")
        collision = self.root / "collision.md"
        self.assertEqual(0, self.save(collision).returncode)
        self.assertEqual(0, self.load(collision).returncode)
        self.input.write_bytes(b"\xff")
        self.assertNotEqual(0, self.save(self.root / "bad-input.md").returncode)
        self.write_input()
        bad_export = self.root / "bad.txt"
        bad_export.write_bytes(b"\xff")
        self.assertNotEqual(0, self.save(self.root / "bad-export.md", "full", "--transcript", str(bad_export)).returncode)
        self.assertEqual(0, self.save(self.root / "valid.md").returncode)
        (self.root / "valid.md").write_bytes(b"\xff")
        self.assertNotEqual(0, self.load(self.root / "valid.md").returncode)
        self.no_context_state()

    def test_human_include_transcript_prints_exact_visible_text(self) -> None:
        export = self.root / "visible.txt"
        export.write_bytes(b"Visible text\r\nlast line")
        self.assertEqual(0, self.save(self.output, "full", "--transcript", str(export)).returncode)
        result = subprocess.run([sys.executable, str(SCRIPT), "--root", str(self.root), "context-load", "--input", str(self.output), "--include-transcript"], capture_output=True)
        self.assertEqual(0, result.returncode, result.stderr)
        self.assertIn(b"Visible text\r\nlast line", result.stdout)

    def test_input_and_output_symlinks_are_rejected(self) -> None:
        target = self.root / "target.json"
        target.write_text(self.input.read_text(), encoding="utf-8")
        link = self.root / "linked.json"
        link.symlink_to(target)
        self.assertNotEqual(0, self.cli("context-save", "--input", str(link), "--output", str(self.output), "--detail", "summary", "--source-client", "codex").returncode)
        directory = self.root / "actual"
        directory.mkdir()
        linked_dir = self.root / "linked-dir"
        linked_dir.symlink_to(directory, target_is_directory=True)
        self.assertNotEqual(0, self.save(linked_dir / "handoff.md").returncode)

    def test_relative_output_is_root_relative_and_protected_targets_reject_normalization(self) -> None:
        elsewhere = Path(self.tmp.name) / "elsewhere"
        elsewhere.mkdir()
        result = subprocess.run(
            [sys.executable, str(SCRIPT), "--root", str(self.root), "context-save", "--input", str(self.input), "--output", "tasks/relative.md", "--detail", "summary", "--source-client", "codex"],
            cwd=elsewhere, text=True, capture_output=True,
        )
        self.assertEqual(0, result.returncode, result.stderr)
        self.assertTrue((self.root / "tasks/relative.md").is_file())
        protected = self.root / "tasks/../memory-bank/forbidden.md"
        self.assertNotEqual(0, self.save(protected).returncode)
        dangling = self.root / "dangling.md"
        target = self.root / "missing-target.md"
        dangling.symlink_to(target)
        self.assertNotEqual(0, self.save(dangling).returncode)
        self.assertFalse(target.exists())

    def test_curated_context_refuses_personal_data_like_every_authored_memory(self) -> None:
        self.write_input(summary="Reported by " + "person" + "@" + "example.org" + " yesterday.")
        result = self.save()
        self.assertNotEqual(0, result.returncode)
        self.assertIn("personal data", result.stderr)
        self.assertNotIn("example.org", result.stderr)
        self.assertFalse(self.output.exists())

    def test_a_handoff_checked_out_with_crlf_line_endings_still_loads(self) -> None:
        """Git for Windows' default core.autocrlf=true turns a committed handoff's LF into CRLF."""
        tasks = self.root / "tasks/TASK-001"
        summary = tasks / "context-save-1.md"
        self.assertEqual(0, self.save(summary).returncode)
        lf_export = self.root / "lf.txt"
        lf_export.write_bytes(b"Visible line one\nline two\n")
        self.assertEqual(0, self.save(tasks / "context-save-2.md", "full", "--transcript", str(lf_export)).returncode)
        crlf_export = self.root / "crlf.txt"
        crlf_export.write_bytes(b"Exported on Windows\r\nsecond line\r\n")
        self.assertEqual(0, self.save(tasks / "context-save-3.md", "full", "--transcript", str(crlf_export)).returncode)
        # Normalize on commit as well as checkout, as in a Laravel skeleton.
        (self.root / ".gitattributes").write_text("* text=auto\n")
        self.git("add", ".gitattributes", "tasks")
        self.git("commit", "-qm", "handoffs")
        clone = Path(self.tmp.name + "-clone")
        self.addCleanup(shutil.rmtree, clone, True)
        subprocess.run(["git", "-c", "core.autocrlf=true", "clone", "-q", str(self.root), str(clone)], check=True)
        self.assertIn(b"\r\n", (clone / "tasks/TASK-001/context-save-1.md").read_bytes())
        expected = {"context-save-1.md": None, "context-save-2.md": "Visible line one\nline two\n",
                    "context-save-3.md": "Exported on Windows\r\nsecond line\r\n"}
        for name, transcript in expected.items():
            with self.subTest(name=name):
                loaded = subprocess.run([sys.executable, str(SCRIPT), "--root", str(clone), "context-load", "--input",
                                         f"tasks/TASK-001/{name}", "--include-transcript", "--json"],
                                        text=True, capture_output=True)
                self.assertEqual(0, loaded.returncode, loaded.stderr)
                payload = json.loads(loaded.stdout)
                self.assertEqual("Continue the handoff.", payload["context"]["goal"])
                self.assertEqual(transcript, payload.get("transcript"))
        # Converting the line endings is not a licence to change the text.
        tampered = (clone / "tasks/TASK-001/context-save-1.md")
        tampered.write_bytes(tampered.read_bytes().replace(b"First pass complete.", b"First pass completed"))
        self.assertNotEqual(0, subprocess.run([sys.executable, str(SCRIPT), "--root", str(clone), "context-load",
                                               "--input", str(tampered)], capture_output=True).returncode)

    def test_topic_and_task_id_follow_the_personal_data_policy(self) -> None:
        email = "jane.doe" + "@" + "example.com"
        for extra in (("--topic", f"call {email} about invoices"), ("--topic", "invoices", "--task-id", email)):
            with self.subTest(extra=extra):
                result = self.save(self.output, "topic", *extra)
                self.assertNotEqual(0, result.returncode)
                self.assertIn("personal data", result.stderr)
                self.assertNotIn(email, result.stderr)
                self.assertFalse(self.output.exists())
        self.assertEqual(0, self.save(self.output, "topic", "--topic", "invoices", "--task-id", "TASK-001").returncode)
        # A label edited into a saved handoff is refused on load as well.
        raw = self.output.read_bytes()
        self.output.write_bytes(raw.replace(b'"topic": "invoices"', b'"topic": "%s"' % email.encode(), 1))
        loaded = self.load()
        self.assertNotEqual(0, loaded.returncode)
        self.assertIn("personal data", loaded.stderr)

    def test_a_project_outside_git_saves_and_loads_cited_files(self) -> None:
        project = Path(self.tmp.name + "-plain")
        project.mkdir()
        self.addCleanup(shutil.rmtree, project, True)
        self.assertNotEqual(0, subprocess.run(["git", "-C", str(project), "rev-parse", "--git-dir"],
                                              capture_output=True).returncode)
        (project / "index.php").write_text("<?php echo 1;\n")
        self.input.write_text(json.dumps(self.data(files=["index.php"])), encoding="utf-8")

        def plain(*args: str) -> subprocess.CompletedProcess[str]:
            return subprocess.run([sys.executable, str(SCRIPT), "--root", str(project), *args], text=True, capture_output=True)

        saved = plain("context-save", "--input", str(self.input), "--output", "tasks/handoff.md",
                      "--detail", "summary", "--source-client", "claude")
        self.assertEqual(0, saved.returncode, saved.stderr)
        (project / "index.php").write_text("<?php echo 2;\n")
        loaded = plain("context-load", "--input", "tasks/handoff.md", "--json")
        self.assertEqual(0, loaded.returncode, loaded.stderr)
        self.assertEqual(["index.php"], [item["path"] for item in json.loads(loaded.stdout)["drift"] if item["kind"] == "file"])
        # A handoff saved in a checkout loads from a copy without its .git.
        self.write_input(files=["source.txt"])
        self.assertEqual(0, self.save().returncode)
        copy = Path(self.tmp.name + "-copy")
        shutil.copytree(self.root, copy, ignore=shutil.ignore_patterns(".git"))
        self.addCleanup(shutil.rmtree, copy, True)
        copied = subprocess.run([sys.executable, str(SCRIPT), "--root", str(copy), "context-load", "--input",
                                 "handoff.md", "--json"], text=True, capture_output=True)
        self.assertEqual(0, copied.returncode, copied.stderr)

    def test_attached_layout_reads_the_project_and_never_writes_into_the_state(self) -> None:
        state = Path(self.tmp.name + "-state")
        state.mkdir()
        self.addCleanup(shutil.rmtree, state, True)
        edition = SCRIPT.parents[2]
        environment = {
            key: value for key, value in os.environ.items()
            if not key.startswith(("ACCELERATOR_", "GIT_"))
        }
        environment.update(ACCELERATOR_HOME=str(edition), ACCELERATOR_STATE_DIR=str(state),
                           ACCELERATOR_PROJECT_DIR=str(self.root))
        self.write_input(files=["source.txt"])

        def attached(*args: str) -> subprocess.CompletedProcess[str]:
            return subprocess.run([sys.executable, str(SCRIPT), "--root", str(state), *args],
                                  text=True, capture_output=True, env=environment)

        saved = attached("context-save", "--input", "curated.json", "--output", "tasks/TASK-001/handoff.md",
                         "--detail", "summary", "--source-client", "claude", "--json")
        self.assertEqual(0, saved.returncode, saved.stderr)
        handoff = self.root / "tasks/TASK-001/handoff.md"
        self.assertTrue(handoff.is_file())
        loaded = attached("context-load", "--input", "tasks/TASK-001/handoff.md", "--json")
        self.assertEqual(0, loaded.returncode, loaded.stderr)
        self.assertEqual([], [item for item in json.loads(loaded.stdout)["drift"] if item["kind"] == "file"])
        refused = attached("context-save", "--input", "curated.json", "--output", str(state / "handoff.md"),
                           "--detail", "summary", "--source-client", "claude")
        self.assertNotEqual(0, refused.returncode)
        self.assertFalse((state / "handoff.md").exists())


if __name__ == "__main__":
    unittest.main()
