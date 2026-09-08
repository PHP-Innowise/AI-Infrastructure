"""Offline project skill installation checks; no downloads or native CLI calls."""

import io
import json
import os
from pathlib import Path
import sys
import tarfile
import tempfile
import unittest
from unittest.mock import patch

sys.path.insert(0, str(Path(__file__).resolve().parents[1] / "harness/src"))
from harness import sessions, skills


class HarnessSkillTests(unittest.TestCase):
    def setUp(self):
        temporary = tempfile.TemporaryDirectory()
        self.addCleanup(temporary.cleanup)
        self.root = Path(temporary.name)
        self.project = self.root / "project"
        self.project.mkdir()
        with patch.object(sessions.providers, "discover_providers", return_value=[
            {"id": "codex", "name": "Fixture Codex", "available": True,
             "executable": "/never-launched/fixture-cli"},
        ]), patch.object(sessions.Sessions, "_worker", return_value=None):
            self.store = sessions.Sessions(self.root / "state", [self.project])
        self.addCleanup(self.store.close)
        self.manager = skills.SkillManager(self.store)
        self.addCleanup(self.manager.close)
        native = patch.object(self.manager, "_run", side_effect=AssertionError("Unexpected native CLI call"))
        self.native = native.start()
        self.addCleanup(native.stop)
        self.project_id = next(iter(self.store.projects))
        self.source_id = self.manager.catalog()["sources"][0]["id"]
        self.fixture = {
            "alpha": {"name": "Alpha fixture", "description": "Selected fixture skill", "files": {
                "SKILL.md": b"---\nname: alpha\ndescription: Selected fixture skill\n---\n# Alpha\n",
                "references/guide.md": "# Fixture guide\nПроверенный текст\n".encode("utf-8"),
            }},
            "beta": {"name": "Beta fixture", "description": "Unselected fixture skill", "files": {
                "SKILL.md": b"---\nname: beta\ndescription: Unselected fixture skill\n---\n# Beta\n",
            }},
        }
        loader = patch.object(self.manager, "_load", return_value=self.fixture)
        self.load = loader.start()
        self.addCleanup(loader.stop)
        self.manager.discover(self.source_id)

    def options(self, **changes):
        return {"project_id": self.project_id, "source_id": self.source_id,
                "skills": ["alpha"], "agents": ["codex", "cursor"], **changes}

    def create_options(self, **changes):
        return {"project_id": self.project_id, "agents": ["codex"], "name": "project-notes",
                "description": "Use when recording project notes.",
                "instructions": "# Project notes\n\nRead the project instructions before editing.\n", **changes}

    def project_files(self):
        return {str(path.relative_to(self.project)): path.read_bytes()
                for path in self.project.rglob("*") if path.is_file() and not path.is_symlink()}

    def test_create_skill_serializes_unicode_and_installs_chosen_tools_without_native_dependencies(self):
        self.manager.loaded.clear()
        self.load.reset_mock()
        description = '  Проверка "кавычки": #метка\n\tnext: value  '
        instructions = '\r\n# Шаги\r\n\r\n- Read `AGENTS.md`.\r\n- Сохранить заметку.\r\n'
        options = self.create_options(name="  review-php8  ", description=description,
                                      instructions=instructions, agents=["claude", "codex", "cursor"])
        database_before = list(self.store.db.iterdump())
        with patch.object(skills.shutil, "which", return_value=None), \
                patch.object(skills, "urlopen", side_effect=AssertionError("Unexpected download")) as download:
            self.assertFalse(self.manager.catalog()["available"])
            preview = self.manager.create_preview(options)
            self.assertEqual((preview["source_id"], preview["name"]), ("local", "review-php8"))
            self.assertTrue(preview["can_install"])
            content = preview["content"]
            frontmatter, body = content.split("\n---\n", 1)
            lines = frontmatter.splitlines()
            self.assertEqual(lines[0], "---")
            self.assertEqual(json.loads(lines[1].removeprefix("name: ")), "review-php8")
            self.assertEqual(len(lines), 3)
            self.assertTrue(lines[2].startswith("description: "))
            self.assertEqual(json.loads(lines[2].removeprefix("description: ")),
                             'Проверка "кавычки": #метка next: value')
            self.assertEqual(body.lstrip("\n"), "# Шаги\n\n- Read `AGENTS.md`.\n- Сохранить заметку.\n")
            expected = {f"{base}/skills/review-php8/SKILL.md": content.encode("utf-8")
                        for base in (".claude", ".agents", ".cursor")}
            self.assertEqual({entry["path"] for entry in preview["files"]}, set(expected))
            self.assertEqual({entry["status"] for entry in preview["files"]}, {"new"})
            self.assertEqual(self.project_files(), {})
            result = self.manager.install(preview["preview_id"])
            self.assertTrue(result["ok"])
            self.assertEqual(set(result["installed"]), set(expected))
            self.assertEqual(self.project_files(), expected)
            self.assertEqual({entry["agent"] for entry in self.manager.installed(self.project_id)["installed"]},
                             {"claude", "codex", "cursor"})
            download.assert_not_called()
        self.load.assert_not_called()
        self.native.assert_not_called()
        self.assertEqual(list(self.store.db.iterdump()), database_before)
        self.assertEqual(self.store.jobs.qsize(), 0)

    def test_create_skill_rejects_invalid_fields_and_limits_without_writes_or_previews(self):
        invalid = [{"name": value} for value in (
            None, [], "", " ", "Upper", "under_score", "with.dot", "double--dash", "-leading", "trailing-",
            "a/b", "a\\b", "é", "a" * 65, "alpha\nbeta",
        )]
        invalid += [{"description": value} for value in (None, [], "", " \t\n", "x" * 1025, "<tag>", "x > y")]
        invalid += [{"instructions": value} for value in (None, [], False, "", " \t\n", "é" * 16001,
                                                            "bad\0body", "bad\x01body", "bad\x7fbody",
                                                            "bad\x85body", "bad\ud800body", "bad\ufffe", "bad\uffff")]
        invalid += [{"agents": value} for value in (None, "codex", [], ["other"], ["codex", "codex"], [None])]
        invalid += [{"project_id": "unregistered"}, {"project_id": {}}, {"source_id": "local"}]
        database_before = list(self.store.db.iterdump())
        for changes in invalid:
            with self.subTest(changes=changes), self.assertRaises(sessions.SessionError):
                self.manager.create_preview(self.create_options(**changes))
        for key in self.create_options():
            data = self.create_options()
            del data[key]
            with self.subTest(missing=key), self.assertRaises(sessions.SessionError):
                self.manager.create_preview(data)
        self.assertEqual(self.manager.previews, {})
        self.assertEqual(self.project_files(), {})
        self.assertEqual(list(self.store.db.iterdump()), database_before)
        boundary = self.manager.create_preview(self.create_options(
            name="a" * 64, description="é" * 1024, instructions="é" * 16000,
        ))
        self.assertTrue(boundary["can_install"])
        self.assertEqual(self.project_files(), {})
        self.native.assert_not_called()

    def test_created_skill_preview_is_immutable_and_repeat_or_conflict_keeps_existing_files(self):
        original = self.create_options()
        submitted = {**original, "agents": list(original["agents"])}
        preview = self.manager.create_preview(submitted)
        content = preview["content"].encode("utf-8")
        submitted.update(name="changed-name", description="Changed description", instructions="Changed body")
        submitted["agents"].append("cursor")
        self.assertTrue(self.manager.install(preview["preview_id"])["ok"])
        target = ".agents/skills/project-notes/SKILL.md"
        self.assertEqual(self.project_files(), {target: content})
        modified = (self.project / target).stat().st_mtime_ns
        repeat = self.manager.create_preview(original)
        self.assertTrue(repeat["can_install"])
        self.assertEqual(repeat["files"], [{"path": target, "status": "identical"}])
        saved = self.manager.install(repeat["preview_id"])
        self.assertEqual((saved["installed"], saved["unchanged"]), ([], [target]))
        self.assertEqual((self.project / target).stat().st_mtime_ns, modified)
        conflict = self.manager.create_preview({**original, "instructions": "Different new instructions"})
        self.assertFalse(conflict["can_install"])
        self.assertEqual(conflict["files"], [{"path": target, "status": "conflict"}])
        with self.assertRaises(sessions.SessionError):
            self.manager.install(conflict["preview_id"])
        self.assertEqual(self.project_files(), {target: content})
        self.assertEqual((self.project / target).stat().st_mtime_ns, modified)

    def test_discovery_preview_and_repeat_install_copy_only_selected_project_files(self):
        catalog = self.manager.catalog()
        self.assertEqual({agent["id"] for agent in catalog["agents"]}, {"claude", "codex", "cursor"})
        self.assertIsInstance(catalog["available"], bool)
        self.assertIsInstance(catalog["detail"], str)
        discovered = self.manager.discover(self.source_id)
        self.assertEqual(discovered["source_id"], self.source_id)
        self.assertEqual({item["id"] for item in discovered["skills"]}, {"alpha", "beta"})
        self.assertTrue(all(isinstance(item["description"], str) for item in discovered["skills"]))
        self.assertEqual(self.project_files(), {})
        before = list(self.store.db.iterdump())
        options = self.options()
        preview = self.manager.preview(options)
        expected = {f"{directory}/alpha/{name}": content
                    for directory in (".agents/skills", ".cursor/skills")
                    for name, content in self.fixture["alpha"]["files"].items()}
        self.assertTrue(preview["can_install"])
        self.assertEqual({item["path"] for item in preview["files"]}, set(expected))
        self.assertEqual({item["status"] for item in preview["files"]}, {"new"})
        self.assertEqual(self.project_files(), {})
        self.assertEqual(list(self.store.db.iterdump()), before)

        # Mutating the request after preview cannot broaden the approved selection.
        options["agents"].append("claude")
        options["skills"].append("beta")
        result = self.manager.install(preview["preview_id"])
        self.assertTrue(result["ok"])
        self.assertEqual(set(result["installed"]), set(expected))
        self.assertEqual(result["unchanged"], [])
        self.assertEqual(self.project_files(), expected)
        with self.assertRaises(sessions.SessionError):
            self.manager.install(preview["preview_id"])
        installed = self.manager.installed(self.project_id)["installed"]
        self.assertEqual({item["agent"] for item in installed}, {"codex", "cursor"})
        self.assertTrue(all(item["name"] == "alpha" for item in installed))

        mtimes = {name: (self.project / name).stat().st_mtime_ns for name in expected}
        repeat = self.manager.preview(self.options())
        self.assertTrue(repeat["can_install"])
        self.assertEqual({item["status"] for item in repeat["files"]}, {"identical"})
        result = self.manager.install(repeat["preview_id"])
        self.assertEqual(result["installed"], [])
        self.assertEqual(set(result["unchanged"]), set(expected))
        self.assertEqual({name: (self.project / name).stat().st_mtime_ns for name in expected}, mtimes)
        self.assertEqual(list(self.store.db.iterdump()), before)
        self.native.assert_not_called()

    def test_conflicts_are_previewed_and_late_changes_are_revalidated_before_any_write(self):
        target = self.project / ".agents/skills/alpha/SKILL.md"
        target.parent.mkdir(parents=True)
        target.write_bytes(b"Existing project-owned skill\n")
        preview = self.manager.preview(self.options())
        self.assertFalse(preview["can_install"])
        self.assertEqual(next(item["status"] for item in preview["files"]
                              if item["path"] == ".agents/skills/alpha/SKILL.md"), "conflict")
        with self.assertRaises(sessions.SessionError):
            self.manager.install(preview["preview_id"])
        self.assertEqual(self.project_files(), {".agents/skills/alpha/SKILL.md": b"Existing project-owned skill\n"})

        target.unlink()
        preview = self.manager.preview(self.options())
        self.assertTrue(preview["can_install"])
        target.write_bytes(b"Written after preview\n")
        before = self.project_files()
        with self.assertRaises(sessions.SessionError):
            self.manager.install(preview["preview_id"])
        self.assertEqual(self.project_files(), before)

    def test_failed_atomic_publication_keeps_previous_files_and_removes_temporary_file(self):
        existing = self.project / ".agents/skills/alpha/references/guide.md"
        existing.parent.mkdir(parents=True)
        existing.write_bytes(self.fixture["alpha"]["files"]["references/guide.md"])
        (self.project / "project-owned.txt").write_bytes(b"Keep this project file\n")
        before = self.project_files()
        modified = {name: (self.project / name).stat().st_mtime_ns for name in before}
        preview = self.manager.preview(self.options(agents=["codex"]))
        self.assertTrue(preview["can_install"])
        with patch.object(skills.os, "link", side_effect=OSError("Fixture publication failure")) as publish:
            with self.assertRaises(sessions.SessionError):
                self.manager.install(preview["preview_id"])
        publish.assert_called_once()
        self.assertEqual(self.project_files(), before)
        self.assertEqual({name: (self.project / name).stat().st_mtime_ns for name in before}, modified)
        self.assertFalse((self.project / ".agents/skills/alpha/SKILL.md").exists())
        self.assertEqual({path.name for path in existing.parent.parent.iterdir()}, {"references"})

    def test_invalid_selections_never_create_project_files_or_native_calls(self):
        invalid = [
            {"project_id": "unregistered"}, {"project_id": {}}, {"source_id": "unknown"},
            {"source_id": []}, {"skills": []}, {"skills": "alpha"}, {"skills": ["unknown"]},
            {"skills": ["../alpha"]}, {"skills": [None]}, {"skills": ["alpha", "alpha"]},
            {"agents": []}, {"agents": "codex"}, {"agents": ["unknown"]}, {"agents": [None]},
            {"agents": ["codex", "codex"]}, {"extra": "unexpected"},
        ]
        for changes in invalid:
            with self.subTest(changes=changes), self.assertRaises(sessions.SessionError):
                self.manager.preview(self.options(**changes))
        for source in ("unknown", [], None):
            with self.subTest(source=source), self.assertRaises(sessions.SessionError):
                self.manager.discover(source)
        for preview_id in ("unknown", [], None):
            with self.subTest(preview_id=preview_id), self.assertRaises(sessions.SessionError):
                self.manager.install(preview_id)
        self.assertEqual(self.project_files(), {})
        self.assertEqual(self.store.jobs.qsize(), 0)
        self.native.assert_not_called()

    def test_queued_or_running_sessions_block_installation(self):
        for status in ("queued", "running"):
            with self.subTest(status=status):
                preview = self.manager.preview(self.options())
                session = self.store.create({"project_id": self.project_id, "provider": "codex",
                                             "prompt": "Never executed fixture"})
                self.store._status(session["id"], status)
                with self.assertRaises(sessions.SessionError):
                    self.manager.install(preview["preview_id"])
                self.assertEqual(self.project_files(), {})
                self.store._status(session["id"], "cancelled")
                self.store.jobs.get_nowait()
                self.store.jobs.task_done()

    def test_existing_links_and_nonregular_targets_are_conflicts_even_when_bytes_match(self):
        outside = self.root / "outside"
        outside.mkdir()
        original = self.fixture["alpha"]["files"]["SKILL.md"]
        secret = outside / "SKILL.md"
        secret.write_bytes(original)
        for agent, base, kind in (("codex", ".agents", "symlink"),
                                  ("cursor", ".cursor", "hardlink"),
                                  ("claude", ".claude", "fifo")):
            with self.subTest(kind=kind):
                target = self.project / base / "skills/alpha/SKILL.md"
                target.parent.mkdir(parents=True)
                if kind == "symlink":
                    target.symlink_to(secret)
                elif kind == "hardlink":
                    os.link(secret, target)
                else:
                    os.mkfifo(target)
                preview = self.manager.preview(self.options(agents=[agent]))
                self.assertFalse(preview["can_install"])
                self.assertEqual(next(item["status"] for item in preview["files"]
                                      if item["path"].endswith("/SKILL.md")), "conflict")
                with self.assertRaises(sessions.SessionError):
                    self.manager.install(preview["preview_id"])
                self.assertFalse((target.parent / "references").exists())
        self.assertEqual(secret.read_bytes(), original)
        self.assertEqual(self.manager.installed(self.project_id)["installed"], [])

    def test_destination_symlink_and_replaced_project_invalidate_existing_previews(self):
        preview = self.manager.preview(self.options(agents=["codex"]))
        outside = self.root / "outside"
        outside.mkdir()
        (self.project / ".agents").symlink_to(outside, target_is_directory=True)
        with self.assertRaises(sessions.SessionError):
            self.manager.install(preview["preview_id"])
        self.assertEqual(list(outside.iterdir()), [])
        (self.project / ".agents").unlink()

        preview = self.manager.preview(self.options(agents=["codex"]))
        self.project.rename(self.root / "original-project")
        self.project.mkdir()
        with self.assertRaises(sessions.SessionError):
            self.manager.install(preview["preview_id"])
        self.assertEqual(list(self.project.iterdir()), [])
        self.assertEqual(list((self.root / "original-project").iterdir()), [])

    def test_previews_expire_and_only_the_latest_32_are_retained(self):
        expired = self.manager.preview(self.options())
        self.manager.previews[expired["preview_id"]]["created"] -= 901
        with self.assertRaises(sessions.SessionError):
            self.manager.install(expired["preview_id"])
        previews = [self.manager.preview(self.options()) for _ in range(33)]
        self.assertEqual(len(self.manager.previews), 32)
        with self.assertRaises(sessions.SessionError):
            self.manager.install(previews[0]["preview_id"])
        self.assertEqual(self.project_files(), {})
        self.assertTrue(self.manager.install(previews[-1]["preview_id"])["ok"])

    def change_options(self, **changes):
        return {'project_id': self.project_id, 'agent': 'codex', 'name': 'alpha', 'operation': 'update', **changes}

    def test_tracked_update_remove_and_restart_keep_other_tools_and_local_skills(self):
        self.fixture['alpha']['revision'] = 'a' * 40
        self.manager.install(self.manager.preview(self.options())['preview_id'])
        original_cursor = {name: body for name, body in self.project_files().items() if name.startswith('.cursor/')}
        self.manager.close()
        self.manager = skills.SkillManager(self.store)
        self.addCleanup(self.manager.close)
        entry = next(item for item in self.manager.installed(self.project_id)['installed'] if item['agent'] == 'codex')
        self.assertEqual((entry['state'], entry['revision'], entry['managed']), ('clean', 'a' * 40, True))
        latest = {'alpha': {**self.fixture['alpha'], 'revision': 'b' * 40, 'files': {
            'SKILL.md': b'---\nname: alpha\ndescription: Updated\n---\n# Updated alpha\n',
            'scripts/check.sh': b'#!/bin/sh\nexit 0\n'}, 'modes': {'scripts/check.sh': 0o755}}}
        with patch.object(self.manager, '_load', return_value=latest) as load:
            before = self.project_files()
            preview = self.manager.change_preview(self.change_options())
            self.assertEqual(load.call_count, 1)
            self.assertEqual(self.project_files(), before)
            self.assertTrue(preview['can_apply'])
            self.assertEqual({row['status'] for row in preview['files']}, {'update', 'delete', 'new'})
            self.assertIn('+description: Updated', next(row['diff'] for row in preview['files'] if row['path'].endswith('SKILL.md')))
            # A later discovery/response mutation cannot change reviewed bytes.
            preview['files'].clear()
            latest['alpha']['files']['SKILL.md'] = b'not reviewed'
            result = self.manager.apply_change(preview['preview_id'])
        self.assertTrue(result['ok'])
        target = self.project / '.agents/skills/alpha'
        self.assertIn(b'# Updated alpha', (target / 'SKILL.md').read_bytes())
        self.assertFalse((target / 'references/guide.md').exists())
        self.assertEqual((target / 'scripts/check.sh').stat().st_mode & 0o777, 0o755)
        self.assertEqual({name: body for name, body in self.project_files().items() if name.startswith('.cursor/')}, original_cursor)
        with self.assertRaises(sessions.SessionError):
            self.manager.apply_change(preview['preview_id'])
        entry = next(item for item in self.manager.installed(self.project_id)['installed'] if item['agent'] == 'codex')
        self.assertEqual((entry['revision'], entry['state']), ('b' * 40, 'clean'))
        removal = self.manager.change_preview(self.change_options(operation='remove'))
        self.assertTrue(removal['can_apply'])
        self.assertTrue(all(row['status'] == 'delete' for row in removal['files']))
        self.assertTrue(self.manager.apply_change(removal['preview_id'])['ok'])
        self.assertFalse((target / 'SKILL.md').exists())
        self.assertEqual(len(self.manager.installed(self.project_id)['installed']), 1)
        # Create skill also gains durable local tracking, and can be removed offline.
        local = self.manager.create_preview(self.create_options())
        self.manager.install(local['preview_id'])
        with patch.object(self.manager, '_load', side_effect=AssertionError('No network for local removal')):
            preview = self.manager.change_preview(self.change_options(name='project-notes', operation='remove'))
            self.assertTrue(self.manager.apply_change(preview['preview_id'])['ok'])

    def test_tracking_adopts_exact_legacy_copies_and_blocks_edits_extra_files_and_links(self):
        for relative, body in self.fixture['alpha']['files'].items():
            path = self.project / '.agents/skills/alpha' / relative
            path.parent.mkdir(parents=True, exist_ok=True); path.write_bytes(body)
        self.assertFalse(self.manager.installed(self.project_id)['installed'][0]['managed'])
        with self.assertRaises(sessions.SessionError):
            self.manager.change_preview(self.change_options(operation='remove'))
        preview = self.manager.preview(self.options(agents=['codex']))
        self.assertTrue(self.manager.install(preview['preview_id'])['ok'])
        self.assertTrue(self.manager.installed(self.project_id)['installed'][0]['managed'])
        target = self.project / '.agents/skills/alpha/SKILL.md'
        original = target.read_bytes()
        target.write_bytes(original + b'Local change\n')
        for operation in ('update', 'remove'):
            preview = self.manager.change_preview(self.change_options(operation=operation))
            self.assertFalse(preview['can_apply'])
            self.assertIn('conflict', {row['status'] for row in preview['files']})
            with self.assertRaises(sessions.SessionError):
                self.manager.apply_change(preview['preview_id'])
            self.assertTrue(target.read_bytes().endswith(b'Local change\n'))
        target.write_bytes(original)
        extra = target.parent / 'personal-notes.md'; extra.write_bytes(b'Keep me\n')
        self.assertEqual(self.manager.installed(self.project_id)['installed'][0]['state'], 'modified')
        self.assertFalse(self.manager.change_preview(self.change_options(operation='remove'))['can_apply'])
        extra.unlink()
        outside = self.root / 'outside.md'; outside.write_bytes(original)
        target.unlink(); target.symlink_to(outside)
        with self.assertRaises(sessions.SessionError):
            self.manager.change_preview(self.change_options(operation='remove'))
        target.unlink(); os.link(outside, target)
        self.assertEqual(self.manager.installed(self.project_id)['installed'][0]['state'], 'unsafe')
        with self.assertRaises(sessions.SessionError):
            self.manager.change_preview(self.change_options())
        self.assertEqual(outside.read_bytes(), original)

    def test_change_previews_reject_stale_files_modes_directories_records_and_active_sessions(self):
        self.manager.install(self.manager.preview(self.options(agents=['codex']))['preview_id'])
        target = self.project / '.agents/skills/alpha/SKILL.md'
        original = target.read_bytes()
        for mutation in ('content', 'mode', 'directory', 'root', 'record', 'queued', 'expired'):
            preview = self.manager.change_preview(self.change_options(operation='remove'))
            if mutation == 'content':
                target.write_bytes(original + b'late edit')
            elif mutation == 'mode':
                target.chmod(0o755)
            elif mutation == 'directory':
                target.parent.rename(target.parent.with_name('moved'))
                import shutil
                shutil.copytree(target.parent.with_name('moved'), target.parent)
            elif mutation == 'root':
                self.project.rename(self.root / 'moved-project'); self.project.mkdir()
            elif mutation == 'record':
                project = self.store.project(self.project_id); records = self.manager._records(project)
                records['.agents/skills/alpha']['revision'] = 'different-receipt'
                self.manager._save_records(project, records)
            elif mutation == 'queued':
                session = self.store.create({'project_id':self.project_id,'provider':'codex','prompt':'do not run'})
            else:
                self.manager.previews[preview['preview_id']]['created'] -= 901
            before = self.project_files()
            with self.subTest(mutation=mutation), self.assertRaises(sessions.SessionError):
                self.manager.apply_change(preview['preview_id'])
            self.assertEqual(before, self.project_files())
            if mutation == 'content': target.write_bytes(original)
            elif mutation == 'mode': target.chmod(0o644)
            elif mutation == 'directory':
                shutil.rmtree(target.parent); target.parent.with_name('moved').rename(target.parent)
            elif mutation == 'root':
                self.project.rmdir(); (self.root / 'moved-project').rename(self.project)
            elif mutation == 'queued':
                self.store._status(session['id'], 'cancelled'); self.store.jobs.get_nowait(); self.store.jobs.task_done()

    def test_partial_update_reports_written_files_without_advancing_tracking(self):
        self.manager.install(self.manager.preview(self.options(agents=['codex']))['preview_id'])
        self.fixture['alpha']['files'] = {'SKILL.md': b'new body', 'added.md': b'new reference'}
        preview = self.manager.change_preview(self.change_options())
        before = self.manager._records(self.store.project(self.project_id))
        real_replace = os.replace
        def fail_skill(source, destination, **kwargs):
            if destination == 'SKILL.md':
                raise OSError('simulated disk failure')
            return real_replace(source, destination, **kwargs)
        with patch.object(skills.os, 'replace', side_effect=fail_skill):
            result = self.manager.apply_change(preview['preview_id'])
        self.assertFalse(result['ok'])
        self.assertTrue(result['changed'])
        self.assertEqual(self.manager._records(self.store.project(self.project_id)), before)
        self.assertEqual(self.manager.installed(self.project_id)['installed'][0]['state'], 'modified')
        self.assertFalse(any('.harness-skill-' in str(p) for p in self.project.rglob('*')))

    def test_refresh_and_revision_resolution_are_bounded_and_fail_without_changes(self):
        self.load.reset_mock()
        self.manager.discover(self.source_id)
        self.load.assert_not_called()
        self.manager.discover(self.source_id, refresh=True)
        self.load.assert_called_once()
        with self.assertRaises(sessions.SessionError):
            self.manager.discover(self.source_id, refresh='yes')
        def reply(request, **kwargs):
            self.assertIn('/commits/HEAD', request.full_url)
            response = io.BytesIO(b'a' * 40 + b'\n'); response.geturl = lambda: request.full_url
            return response
        with patch.object(skills, 'urlopen', side_effect=reply):
            self.assertEqual(self.manager._revision('owner/repo'), 'a' * 40)
        self.manager.install(self.manager.preview(self.options(agents=['codex']))['preview_id'])
        before = self.project_files()
        with patch.object(self.manager, '_load', side_effect=OSError('network unavailable')):
            with self.assertRaises(sessions.SessionError):
                self.manager.change_preview(self.change_options())
            self.assertTrue(self.manager.change_preview(self.change_options(operation='remove'))['can_apply'])
        self.assertEqual(self.project_files(), before)
        for changes in ({'agent':[]}, {'name':'../bad'}, {'operation':'delete'}, {'extra':True}):
            with self.assertRaises(sessions.SessionError):
                self.manager.change_preview(self.change_options(**changes))

    def test_source_archives_reject_traversal_links_and_nonregular_members(self):
        for index, (name, member_type, linkname) in enumerate((
            ("../escape.md", tarfile.REGTYPE, ""),
            (str(self.root / "absolute-escape.md"), tarfile.REGTYPE, ""),
            ("repo/symlink", tarfile.SYMTYPE, "../../escape.md"),
            ("repo/hardlink", tarfile.LNKTYPE, "repo/SKILL.md"),
            ("repo/pipe", tarfile.FIFOTYPE, ""),
        )):
            with self.subTest(name=name):
                stream = io.BytesIO()
                with tarfile.open(fileobj=stream, mode="w:gz") as archive:
                    member = tarfile.TarInfo(name)
                    member.type = member_type
                    member.linkname = linkname
                    member.size = 7 if member_type == tarfile.REGTYPE else 0
                    archive.addfile(member, io.BytesIO(b"fixture") if member.size else None)
                destination = self.root / "extracted" / str(index)
                destination.mkdir(parents=True)
                with self.assertRaises(sessions.SessionError):
                    skills.extract_source(stream.getvalue(), destination)
        self.assertFalse((self.root / "escape.md").exists())
        self.assertFalse((self.root / "absolute-escape.md").exists())

    def test_source_archive_omits_root_policy_alias_but_rejects_links_inside_skills(self):
        body = self.fixture["alpha"]["files"]["SKILL.md"]
        for index, link_path in enumerate(("repo/AGENTS.md", "repo/skills/alpha/reference")):
            with self.subTest(link_path=link_path):
                stream = io.BytesIO()
                with tarfile.open(fileobj=stream, mode="w:gz") as archive:
                    link = tarfile.TarInfo(link_path)
                    link.type = tarfile.SYMTYPE
                    link.linkname = "CLAUDE.md"
                    archive.addfile(link)
                    member = tarfile.TarInfo("repo/skills/alpha/SKILL.md")
                    member.size = len(body)
                    archive.addfile(member, io.BytesIO(body))
                destination = self.root / f"policy-archive-{index}"
                destination.mkdir()
                if index:
                    with self.assertRaises(sessions.SessionError):
                        skills.extract_source(stream.getvalue(), destination)
                else:
                    skills.extract_source(stream.getvalue(), destination)
                    self.assertEqual((destination / "repo/skills/alpha/SKILL.md").read_bytes(), body)
                self.assertFalse(os.path.lexists(destination / link_path))

    def test_native_staging_shape_and_executable_modes_survive_selected_installation(self):
        self.fixture["alpha"]["files"]["scripts/check.sh"] = b"#!/bin/sh\nexit 0\n"
        stream = io.BytesIO()
        with tarfile.open(fileobj=stream, mode="w:gz") as archive:
            body = self.fixture["alpha"]["files"]["SKILL.md"]
            member = tarfile.TarInfo("fixture-repo/skills/alpha/SKILL.md")
            member.size = len(body)
            archive.addfile(member, io.BytesIO(body))
        archive_bytes = stream.getvalue()
        calls = []

        def download(request, **_):
            response = io.BytesIO(archive_bytes)
            response.geturl = lambda: request.full_url
            return response

        def native(command, cwd):
            cwd = Path(cwd)
            calls.append((command, cwd))
            self.assertNotEqual(cwd, self.project)
            self.assertTrue(cwd.is_relative_to(self.store.state_dir))
            self.assertIn("skills@1.5.23", command)
            self.assertNotIn("--global", command)
            folder = cwd / ".agents/skills/alpha"
            if "add" in command:
                self.assertIn("--copy", command)
                self.assertEqual(command[command.index("--skill") + 1], "*")
                self.assertEqual(command[command.index("--agent") + 1], "codex")
                source = Path(command[command.index("add") + 1])
                self.assertTrue((source / "fixture-repo/skills/alpha/SKILL.md").is_file())
                for name, body in self.fixture["alpha"]["files"].items():
                    target = folder / name
                    target.parent.mkdir(parents=True, exist_ok=True)
                    target.write_bytes(body)
                    target.chmod(0o755 if name == "scripts/check.sh" else 0o644)
                return "Fixture staging completed"
            self.assertIn("list", command)
            self.assertIn("--json", command)
            return json.dumps([{"name": "Alpha fixture", "path": str(folder), "scope": "project",
                                "agents": ["codex"], "description": "Ignored CLI display value"}])

        with patch.object(skills, "urlopen", side_effect=download) as downloaded, \
                patch.object(skills.shutil, "which", return_value="/never-launched/npx"), \
                patch.object(self.manager, "_revision", return_value="a" * 40), \
                patch.object(self.manager, "_run", side_effect=native):
            loaded = skills.SkillManager._load(self.manager, self.source_id)
        self.assertEqual(loaded["alpha"]["files"], self.fixture["alpha"]["files"])
        self.assertEqual(loaded["alpha"]["name"], "Alpha fixture")
        self.assertEqual(loaded["alpha"]["description"], "Selected fixture skill")
        self.assertEqual(loaded["alpha"]["modes"]["scripts/check.sh"], 0o755)
        self.assertEqual(loaded["alpha"]["modes"]["SKILL.md"], 0o644)
        self.assertEqual(len(calls), 2)
        self.assertTrue(all(not cwd.exists() for _, cwd in calls))
        downloaded.assert_called_once()
        self.assertEqual(self.project_files(), {})
        self.manager.loaded[self.source_id] = loaded
        preview = self.manager.preview(self.options(agents=["codex"]))
        self.assertTrue(self.manager.install(preview["preview_id"])["ok"])
        destination = self.project / ".agents/skills/alpha"
        self.assertTrue((destination / "scripts/check.sh").stat().st_mode & 0o100)
        self.assertEqual((destination / "SKILL.md").stat().st_mode & 0o111, 0)


if __name__ == "__main__":
    unittest.main()
