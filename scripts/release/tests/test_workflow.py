"""Portable workflow input and wiring regressions (YAML syntax is checked separately)."""

import os
from pathlib import Path
import sys
import tempfile
import unittest
from unittest.mock import patch

sys.path.insert(0, str(Path(__file__).resolve().parents[1]))
import release_metadata as metadata
import workflow_plan
from test_metadata import COMMIT, SOURCE_ROOT


class WorkflowInputTests(unittest.TestCase):
    def run_plan(self, event="workflow_dispatch", publish="false", tag="", ref="refs/heads/main", sha=COMMIT, event_commit=COMMIT):
        with tempfile.TemporaryDirectory() as temporary:
            output = Path(temporary) / "outputs"
            environment = {"GITHUB_EVENT_NAME": event, "INPUT_PUBLISH": publish, "INPUT_RELEASE_TAG": tag,
                           "GITHUB_REF": ref, "GITHUB_SHA": sha, "GITHUB_REPOSITORY": metadata.REPOSITORY, "GITHUB_OUTPUT": str(output)}
            with patch.dict(os.environ, environment, clear=True), \
                    patch.object(workflow_plan, "git", return_value=event_commit), \
                    patch.object(workflow_plan, "checked_tag", return_value=COMMIT) as checked, \
                    patch.object(workflow_plan, "source_metadata") as source, \
                    patch.object(workflow_plan, "GitHub"), \
                    patch.object(workflow_plan, "no_existing_release") as no_existing:
                workflow_plan.main()
                return output.read_text(), checked.call_args, source.call_args, no_existing.call_count

    def test_default_dispatch_is_build_only_without_api_or_tag_checkout(self):
        output, checkout, source, checks = self.run_plan()
        self.assertIn("publish=false\ntag=\ncommit=" + COMMIT, output)
        self.assertIsNone(checkout)
        self.assertEqual(checks, 0)

    def test_manual_publish_requires_exact_workflow_tag_and_event_commit(self):
        output, checked, source, checks = self.run_plan(publish="true", tag="v1.2.3", ref="refs/tags/v1.2.3")
        self.assertIn("publish=true\ntag=v1.2.3\n", output)
        self.assertEqual(checked.args[-2:], ("v1.2.3", COMMIT))
        self.assertEqual(source.args[-1], "v1.2.3")
        self.assertEqual(checks, 1)

    def test_tag_push_uses_event_tag_and_binds_commit(self):
        output, checked, source, checks = self.run_plan(event="push", ref="refs/tags/v1.2.3")
        self.assertIn("publish=true\ntag=v1.2.3\n", output)
        self.assertEqual(checked.args[-2:], ("v1.2.3", COMMIT))
        self.assertEqual(source.args[-1], "v1.2.3")

    def test_branch_or_different_workflow_tag_cannot_publish_requested_tag(self):
        for ref in ("refs/heads/main", "refs/tags/v9.9.9", "refs/tags/1.2.3"):
            with self.subTest(ref=ref), self.assertRaises(metadata.ReleaseError):
                self.run_plan(publish="true", tag="v1.2.3", ref=ref)

    def test_missing_malformed_or_different_event_commit_is_refused(self):
        for changes in ({"sha": ""}, {"sha": "--evil"}, {"event_commit": "b" * 40}):
            with self.subTest(changes=changes), self.assertRaises(metadata.ReleaseError):
                self.run_plan(publish="true", tag="v1.2.3", ref="refs/tags/v1.2.3", **changes)

    def test_pull_request_branch_publish_missing_or_unsafe_tag_refused(self):
        for arguments in ({"event": "pull_request"}, {"event": "pull_request_target"},
                          {"event": "push", "ref": "refs/heads/main"}, {"publish": "true"},
                          {"publish": "true", "tag": "main"}, {"publish": "true", "tag": "--orphan"},
                          {"publish": "true", "tag": "v1.2.3\ncommit=evil"}, {"tag": "v1.2.3"}):
            with self.subTest(arguments=arguments), self.assertRaises(metadata.ReleaseError):
                self.run_plan(**arguments)


class WorkflowWiringTests(unittest.TestCase):
    def test_documented_install_copy_cannot_restore_unsafe_workflow(self):
        for name in ("release.yml", "nightly.yml"):
            with self.subTest(workflow=name):
                self.assertEqual((SOURCE_ROOT / ".github/workflows" / name).read_bytes(),
                                 (SOURCE_ROOT / "docs/ci/workflows" / name).read_bytes())

    def test_release_permissions_triggers_and_artifact_scope(self):
        content = (SOURCE_ROOT / ".github/workflows/release.yml").read_text()
        self.assertIn("permissions:\n  contents: read", content)
        self.assertEqual(content.count("contents: write"), 1)
        self.assertIn("    needs: [prepare, build]", content)
        self.assertIn("    if: success() && needs.prepare.outputs.publish == 'true'", content)
        self.assertNotIn("pull_request", content)
        self.assertNotIn("continue-on-error", content)
        self.assertNotIn("bun-version: latest", content)
        self.assertIn('bun-version: "1.4.2"', content)
        self.assertIn("cancel-in-progress: false", content)
        for target in ("mac-arm64", "mac-x64"):
            self.assertIn("name: verified-" + target + "-${{ github.run_id }}-${{ github.run_attempt }}", content)
        self.assertNotIn("merge-multiple", content.replace("# No wildcard, other workflow/run, merge-multiple, cached or previous-attempt inputs.", ""))
        self.assertNotIn("--clobber", content)
        # Every build signs its updater bundle; developer artifacts carry the archive + signature too.
        self.assertIn("TAURI_SIGNING_PRIVATE_KEY: ${{ secrets.TAURI_SIGNING_PRIVATE_KEY }}", content)
        self.assertIn("TAURI_SIGNING_PRIVATE_KEY_PASSWORD: ${{ secrets.TAURI_SIGNING_PRIVATE_KEY_PASSWORD }}", content)
        self.assertIn("release/bundle/macos/*.app.tar.gz\n", content)
        self.assertIn("release/bundle/macos/*.app.tar.gz.sig\n", content)

    def test_nightly_workflow_wiring(self):
        content = (SOURCE_ROOT / ".github/workflows/nightly.yml").read_text()
        self.assertIn("permissions:\n  contents: read", content)
        self.assertEqual(content.count("contents: write"), 1)
        self.assertIn('cron: "0 3 * * *"', content)
        self.assertIn("workflow_dispatch", content)
        self.assertNotIn("pull_request", content)
        self.assertNotIn("continue-on-error", content)
        self.assertIn('bun-version: "1.4.2"', content)
        self.assertIn("cancel-in-progress: false", content)
        self.assertIn("if: needs.plan.outputs.build == 'true'", content)
        self.assertIn("    needs: [plan, build]", content)
        self.assertIn('PUBLISH_RELEASE: "false"', content)
        self.assertIn("BLUEY_BUILD_VERSION: ${{ needs.plan.outputs.version }}", content)
        self.assertIn("TAURI_SIGNING_PRIVATE_KEY: ${{ secrets.TAURI_SIGNING_PRIVATE_KEY }}", content)
        self.assertNotIn("APPLE_", content)
        self.assertIn("scripts/release/nightly.py plan", content)
        self.assertIn("scripts/release/nightly.py publish", content)
        for target in ("mac-arm64", "mac-x64"):
            self.assertIn("name: nightly-" + target + "-${{ github.run_id }}-${{ github.run_attempt }}", content)
        self.assertNotIn("merge-multiple", content)
        self.assertNotIn("--clobber", content)


if __name__ == "__main__":
    unittest.main()
