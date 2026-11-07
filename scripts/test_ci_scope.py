import importlib.util
from pathlib import Path
import unittest

spec = importlib.util.spec_from_file_location("ci_scope", Path(__file__).with_name("ci-scope.py"))
ci_scope = importlib.util.module_from_spec(spec)
spec.loader.exec_module(ci_scope)


class ScopeTests(unittest.TestCase):
    def test_accepted_film_change_is_website_only(self):
        self.assertTrue(ci_scope.website_only(["docs/download/index.html", "docs/download/download.js", "docs/download/styles.css", "docs/download/assets/brand-mg.mp4", "docs/download/assets/brand-mg.webm", "docs/download/assets/brand-mg.zh.vtt"]))

    def test_build_license_and_unknown_inputs_require_full_checks(self):
        for path in ["Cargo.lock", "Cargo.toml", "crates/pkms-core/src/lib.rs", "apps/desktop/package.json", "packages/contracts/src/generated/types.ts", "scripts/build-user-docs.ps1", ".github/workflows/windows-checks.yml", "LICENSE", "THIRD-PARTY-NOTICES.md", "CHANGELOG.md", "third-party/example/data", "docs/new-unclassified-input.dat", "docs/download/input.rs"]:
            with self.subTest(path=path):
                self.assertFalse(ci_scope.website_only(["docs/download/styles.css", path]))

    def test_empty_and_unknown_events_are_full(self):
        self.assertFalse(ci_scope.website_only([]))
        self.assertTrue(ci_scope.scope("workflow_dispatch", {}, lambda *args: b"")["full"])
        self.assertTrue(ci_scope.scope("push", {"before": "0" * 40, "after": "b" * 40}, lambda *args: b"")["full"])
        self.assertTrue(ci_scope.scope("pull_request", {}, lambda *args: b"")["full"])

    def test_push_checks_both_deleted_and_added_paths(self):
        calls = []
        def git(*args):
            calls.append(args)
            return b"Cargo.toml\0docs/download/moved.html\0"
        result = ci_scope.scope("push", {"before": "a" * 40, "after": "b" * 40}, git)
        self.assertTrue(result["full"])
        self.assertIn("--no-renames", calls[0])

    def test_pr_diff_starts_at_merge_base(self):
        calls = []
        def git(*args):
            calls.append(args)
            return b"c" * 40 + b"\n" if args[0] == "merge-base" else b"docs/user-guide.md\0"
        result = ci_scope.scope("pull_request", {"pull_request": {"base": {"sha": "a" * 40}, "head": {"sha": "b" * 40}}}, git)
        self.assertFalse(result["full"])
        self.assertIn("c" * 40, calls[1])

    def test_unavailable_history_does_not_skip_validation(self):
        def git(*args):
            raise OSError("history unavailable")
        self.assertTrue(ci_scope.scope("push", {"before": "a" * 40, "after": "b" * 40}, git)["full"])


if __name__ == "__main__":
    unittest.main()
