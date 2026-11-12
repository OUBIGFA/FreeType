"""Check publication boundaries using real temporary Git indexes."""

import hashlib
import json
import os
from pathlib import Path
import subprocess
import sys
import tempfile
import unittest


ROOT = Path(__file__).resolve().parents[2]
CHECKER = ROOT / "tools" / "check_repo.py"


class RepositoryCheckTests(unittest.TestCase):
    def setUp(self):
        scratch = Path(os.environ.get("FREETYPE_TEST_TEMP", ROOT / "_temp" / "repository-tests"))
        scratch.mkdir(parents=True, exist_ok=True)
        self.repo = Path(tempfile.mkdtemp(prefix="case-", dir=scratch))
        self.git("init", "-q")
        self.git("config", "core.autocrlf", "false")
        self.git("config", "core.ignorecase", "false")
        self.write(".gitignore", (ROOT / ".gitignore").read_text(encoding="utf-8"))
        self.write("README.md", "# Example\n\n[Guide](references/guide.md)\n")
        self.write("references/guide.md", "# Guide\n")
        self.git("add", ".")

    def git(self, *args, data=None):
        return subprocess.run(
            ["git", "-C", str(self.repo), *args],
            input=data, capture_output=True, check=True,
        ).stdout

    def write(self, path, content):
        target = self.repo / path
        target.parent.mkdir(parents=True, exist_ok=True)
        if isinstance(content, bytes):
            target.write_bytes(content)
        else:
            target.write_text(content, encoding="utf-8", newline="\n")

    def run_check(self):
        return subprocess.run(
            [sys.executable, "-X", "utf8", str(CHECKER)],
            cwd=self.repo, capture_output=True, text=True, encoding="utf-8",
        )

    def assert_rejected(self, marker):
        result = self.run_check()
        self.assertNotEqual(result.returncode, 0, result.stdout)
        self.assertIn(marker, result.stdout + result.stderr)

    def register_binary(self, path, content):
        self.write("references/repository-exceptions.json", json.dumps({
            "binaries": [{
                "path": path,
                "reason": "Public synthetic fixture for repository tests",
                "source": "https://example.invalid/synthetic-fixture",
                "license": "CC0-1.0",
                "bytes": len(content),
                "sha256": hashlib.sha256(content).hexdigest(),
            }],
        }) + "\n")
        self.git("add", "references/repository-exceptions.json")

    def test_clean_index_with_unicode_link_passes(self):
        self.write("references/中文 名称.md", "# 夹具\n")
        self.write("README.md", "[Guide](references/%E4%B8%AD%E6%96%87%20%E5%90%8D%E7%A7%B0.md)\n")
        self.git("add", ".")
        result = self.run_check()
        self.assertEqual(result.returncode, 0, result.stdout + result.stderr)

    def test_public_technical_docs_are_allowed(self):
        self.write("docs/ABI.md", "# Public ABI contract\n")
        self.git("add", "docs/ABI.md")
        result = self.run_check()
        self.assertEqual(result.returncode, 0, result.stdout + result.stderr)

    def test_empty_index_is_rejected(self):
        self.git("read-tree", "--empty")
        self.assert_rejected("empty Git index")

    def test_local_boundaries_cannot_be_force_added(self):
        for path in (
            "docs/HANDOFF.md", "docs/images/example.png", "analysis/report.md",
            "nested/AGENTS.md", "nested/CLAUDE.md", "nested/.agents/state.json",
            "nested/.claude/state.json", "nested/.local/input.txt",
            "nested/output/build.txt", "nested/_temp/private.txt",
            ".github/workflows/telegram-bot.yml",
        ):
            with self.subTest(path=path):
                self.write(path, "private\n")
                self.git("add", "-f", path)
                self.assert_rejected("forbidden")
                self.git("rm", "--cached", path)

    def test_later_unignore_cannot_expose_nested_scratch(self):
        ignore = (self.repo / ".gitignore").read_text(encoding="utf-8")
        self.write(".gitignore", ignore + "!references/_temp/\n!references/_temp/**\n")
        self.git("add", ".gitignore")
        self.assert_rejected("ignore boundary")

    def test_unstaged_ignore_change_is_rejected(self):
        self.write(".gitignore", "_temp/\n")
        self.assert_rejected(".gitignore differs")

    def test_untracked_nested_ignore_cannot_change_the_policy(self):
        self.write("references/.gitignore", "guide.md\n")
        self.assert_rejected("untracked .gitignore")

    def test_missing_tracked_link_is_rejected(self):
        self.write("README.md", "[Missing](references/local-only.md)\n")
        self.write("references/local-only.md", "local\n")
        self.git("add", "README.md")
        self.assert_rejected("missing tracked link")

    def test_local_exclude_cannot_hide_missing_project_rule(self):
        ignore = (self.repo / ".gitignore").read_text(encoding="utf-8")
        self.write(".gitignore", ignore.replace(".local/\n", ""))
        self.write(".git/info/exclude", ".local/\n")
        self.git("add", ".gitignore")
        self.assert_rejected("ignore boundary")

    def test_source_evidence_checks_staged_bytes(self):
        content = b"expected evidence\n"
        self.write("evidence.txt", b"changed evidence\n")
        self.git("add", "evidence.txt")
        self.write("evidence.txt", content)
        self.write("third_party/manifests/upstreams.json", json.dumps({
            "sources": [{"revision": "1" * 40, "trackedEvidence": [{
                "path": "evidence.txt", "bytes": len(content),
                "sha256": hashlib.sha256(content).hexdigest(),
            }]}],
        }))
        self.git("add", "third_party/manifests/upstreams.json")
        self.assert_rejected("source evidence mismatch")

    def test_cargo_inventory_detects_changed_lock(self):
        self.write("Cargo.lock", 'version = 3\n[[package]]\nname = "example"\nversion = "1.0.0"\n')
        result = subprocess.run(
            [sys.executable, "-X", "utf8", str(ROOT / "tools" / "update_cargo_inventory.py")],
            cwd=self.repo, capture_output=True, text=True, encoding="utf-8",
        )
        self.assertEqual(result.returncode, 0, result.stdout + result.stderr)
        self.git("add", "Cargo.lock", "third_party/manifests/cargo-lock.json")
        checked = self.run_check()
        self.assertEqual(checked.returncode, 0, checked.stdout + checked.stderr)
        self.write("Cargo.lock", 'version = 3\n[[package]]\nname = "example"\nversion = "2.0.0"\n')
        self.git("add", "Cargo.lock")
        self.assert_rejected("Cargo.lock registration mismatch")

    def test_link_examples_in_code_fences_are_not_links(self):
        self.write("README.md", "# Examples\n\n~~~markdown\n[Label](not-a-real-file.md)\n~~~\n")
        self.git("add", "README.md")
        result = self.run_check()
        self.assertEqual(result.returncode, 0, result.stdout + result.stderr)

    def test_staged_secret_is_found_after_worktree_cleanup(self):
        secret = "ghp_" + "A" * 36
        self.write("config.txt", secret + "\n")
        self.git("add", "config.txt")
        self.write("config.txt", "redacted\n")
        result = self.run_check()
        self.assertNotEqual(result.returncode, 0)
        self.assertIn("credential pattern", result.stdout + result.stderr)
        self.assertNotIn(secret, result.stdout + result.stderr)

    def test_machine_private_path_is_rejected_without_echoing_it(self):
        private_path = "C:" + "/Users/" + "PrivatePerson/workspace"
        self.write("config.txt", private_path + "\n")
        self.git("add", "config.txt")
        result = self.run_check()
        self.assertNotEqual(result.returncode, 0)
        self.assertIn("machine-private path", result.stdout + result.stderr)
        self.assertNotIn(private_path, result.stdout + result.stderr)

    def test_invalid_utf8_json_bom_and_nul_are_rejected(self):
        for content, marker in (
            (b"\xff", "UTF-8"), (b"{bad}\n", "invalid JSON"),
            (b"\xef\xbb\xbf{}\n", "BOM"), (b"\x00", "NUL"),
        ):
            with self.subTest(marker=marker):
                self.write("data.json", content)
                self.git("add", "data.json")
                self.assert_rejected(marker)

    def test_binary_suffix_alone_does_not_allow_a_binary(self):
        self.write("references/private.png", b"\x89PNG\x00\xff")
        self.git("add", "references/private.png")
        self.assert_rejected("unregistered binary")

    def test_exact_binary_registration_passes(self):
        content = b"\x89PNG\x00\xff"
        self.write("references/public.png", content)
        self.register_binary("references/public.png", content)
        self.git("add", "references/public.png")
        result = self.run_check()
        self.assertEqual(result.returncode, 0, result.stdout + result.stderr)

    def test_binary_registration_cannot_cover_another_path(self):
        content = b"\x89PNG\x00\xff"
        self.write("references/public.png", content)
        self.write("references/private.png", content)
        self.register_binary("references/public.png", content)
        self.git("add", "references/public.png", "references/private.png")
        self.assert_rejected("unregistered binary")

    def test_binary_digest_is_checked_against_the_index(self):
        content = b"\x89PNG\x00\xff"
        self.write("references/public.png", content + b"changed")
        self.git("add", "references/public.png")
        self.register_binary("references/public.png", content)
        self.write("references/public.png", content)
        self.assert_rejected("binary registration mismatch")

    def test_invalid_binary_registration_is_rejected(self):
        self.write("references/repository-exceptions.json", '{"binaries": [{"path": "*.png"}]}\n')
        self.git("add", "references/repository-exceptions.json")
        self.assert_rejected("invalid binary registration")

    def test_large_staged_file_is_rejected(self):
        self.write("large.txt", b"x" * (10 * 1024 * 1024 + 1))
        self.git("add", "large.txt")
        self.assert_rejected("10 MiB")

    def test_case_conflict_is_rejected(self):
        blob = self.git("hash-object", "-w", "--stdin", data=b"fixture\n").decode().strip()
        self.git("update-index", "--add", "--cacheinfo", f"100644,{blob},readme.md")
        self.assert_rejected("case collision")

    def test_symlink_is_rejected(self):
        blob = self.git("hash-object", "-w", "--stdin", data=b"README.md\n").decode().strip()
        self.git("update-index", "--add", "--cacheinfo", f"120000,{blob},link.md")
        self.assert_rejected("unsupported mode")

    def test_submodule_without_review_is_rejected(self):
        self.git("update-index", "--add", "--cacheinfo", f"160000,{'1' * 40},vendor/example")
        self.assert_rejected("submodule")

    def test_windows_reserved_path_is_rejected(self):
        self.git("config", "core.protectNTFS", "false")
        blob = self.git("hash-object", "-w", "--stdin", data=b"fixture\n").decode().strip()
        self.git("update-index", "--add", "--cacheinfo", f"100644,{blob},references/CON.txt")
        self.assert_rejected("Windows-incompatible path")


if __name__ == "__main__":
    unittest.main()

