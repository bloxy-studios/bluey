"""Mock API protocol tests; no token, network, real tag or real Release is used."""

import copy
from pathlib import Path
import sys
import unittest
from unittest.mock import patch
import urllib.request

sys.path.insert(0, str(Path(__file__).resolve().parents[1]))
import publish_release as publisher
import release_metadata as metadata
from test_metadata import COMMIT, ReleaseFixture, replace_json


class FakeAPI:
    def __init__(self, existing=None, fail_upload=None, tamper_readback=False, mutate_before_final=False, move_tag=False):
        self.existing = existing
        self.fail_upload = fail_upload
        self.tamper_readback = tamper_readback
        self.mutate_before_final = mutate_before_final
        self.move_tag = move_tag
        self.remote_reads = 0
        self.draft = None
        self.uploads = []
        self.calls = []
        self.facts = {}

    def remote_commit(self, tag):
        self.remote_reads += 1
        return "b" * 40 if self.move_tag and self.remote_reads > 1 else COMMIT

    def find_release(self, tag):
        return self.existing

    def call(self, method, path, data=None):
        self.calls.append((method, path, copy.deepcopy(data)))
        if method == "POST" and path.endswith("/releases"):
            self.draft = {"id": 10, "assets": [], **copy.deepcopy(data)}
            self.existing = self.draft
            return copy.deepcopy(self.draft)
        if method == "GET":
            result = copy.deepcopy(self.draft)
            if self.mutate_before_final:
                result["assets"].append({"id": 999, "name": "extra.zip", "size": 10, "state": "uploaded"})
            return result
        if method == "PATCH":
            self.draft.update(data)
            return copy.deepcopy(self.draft)
        raise AssertionError("Unexpected API mutation")

    def upload(self, release_id, path):
        if self.fail_upload == len(self.uploads) + 1:
            raise metadata.ReleaseError("mock upload failure")
        self.uploads.append(path.name)
        facts = metadata.file_facts(path)
        asset = {"id": 100 + len(self.uploads), "name": path.name, "size": facts["bytes"], "state": "uploaded"}
        self.facts[asset["id"]] = facts
        self.draft["assets"].append(asset)
        return copy.deepcopy(asset)

    def asset_facts(self, asset_id):
        result = self.facts[asset_id].copy()
        if self.tamper_readback:
            result["sha256"] = "0" * 64
        return result


class PublicationTests(ReleaseFixture):
    def setup_payload(self):
        self.pair()
        self.assemble()

    def run_publish(self, api, native_failure=None):
        with patch.object(publisher, "context", return_value=(metadata.source_metadata(self.root), COMMIT)), \
                patch.object(publisher, "check_native_payload", side_effect=native_failure):
            return publisher.publish(api, self.root, self.output, self.tag, COMMIT, "1234", "1")

    def test_publisher_itself_requires_native_acceptance_before_draft(self):
        self.setup_payload()
        api = FakeAPI()
        with self.assertRaises(metadata.ReleaseError):
            self.run_publish(api, metadata.ReleaseError("mock native gate rejected"))
        self.assertEqual(api.calls, [])
        self.assertEqual(api.uploads, [])

    def test_draft_upload_readback_then_stable_finalization(self):
        self.setup_payload()
        api = FakeAPI()
        self.assertEqual(self.run_publish(api), 10)
        self.assertEqual(api.calls[0][0], "POST")
        self.assertIs(api.calls[0][2]["draft"], True)
        self.assertIs(api.calls[-1][2]["draft"], False)
        self.assertIs(api.calls[-1][2]["prerelease"], False)
        self.assertEqual(api.calls[-1][2]["make_latest"], "legacy")
        self.assertEqual(len(api.uploads), 4)
        self.assertEqual(api.uploads[-1], metadata.MANIFEST)
        self.assertEqual(set(api.uploads), {p.name for p in self.output.iterdir()})
        self.assertNotIn("DELETE", [call[0] for call in api.calls])
        self.assertFalse(any("/git/" in call[1] for call in api.calls))

    def test_prerelease_never_becomes_stable_latest(self):
        version = "1.2.3-rc.1+build.2"
        for relative in ("package.json", "src-tauri/tauri.conf.json", "src-tauri/Cargo.toml"):
            path = self.root / relative
            path.write_text(path.read_text().replace('"' + self.version + '"', '"' + version + '"', 1))
        self.version, self.tag = version, "v" + version
        self.expected = metadata.provenance(self.tag, COMMIT, "1234", "1")
        self.setup_payload()
        api = FakeAPI()
        self.run_publish(api)
        self.assertIs(api.calls[-1][2]["prerelease"], True)
        self.assertEqual(api.calls[-1][2]["make_latest"], "false")

    def test_existing_draft_or_published_release_refused_without_mutation(self):
        self.setup_payload()
        for draft in (True, False):
            api = FakeAPI(existing={"id": 99, "draft": draft})
            with self.subTest(draft=draft), self.assertRaises(metadata.ReleaseError):
                self.run_publish(api)
            self.assertEqual(api.calls, [])
            self.assertEqual(api.uploads, [])

    def test_partial_upload_never_finalizes_or_clobbers(self):
        self.setup_payload()
        api = FakeAPI(fail_upload=2)
        with self.assertRaises(metadata.ReleaseError):
            self.run_publish(api)
        self.assertEqual(len(api.uploads), 1)
        self.assertIs(api.draft["draft"], True)
        self.assertNotIn("PATCH", [call[0] for call in api.calls])
        # Replay refuses this partial draft instead of deleting/replacing its assets.
        with self.assertRaises(metadata.ReleaseError):
            self.run_publish(api)
        self.assertEqual(len(api.uploads), 1)

    def test_stored_bytes_asset_set_or_tag_drift_prevent_finalization(self):
        self.setup_payload()
        for kwargs in ({"tamper_readback": True}, {"mutate_before_final": True}, {"move_tag": True}):
            api = FakeAPI(**kwargs)
            with self.subTest(kwargs=kwargs), self.assertRaises(metadata.ReleaseError):
                self.run_publish(api)
            self.assertIs(api.draft["draft"], True)
            self.assertNotIn("PATCH", [call[0] for call in api.calls])

    def test_bad_payload_rejected_before_draft_creation(self):
        self.setup_payload()
        next(self.output.glob("*.dmg")).write_bytes(b"tampered")
        api = FakeAPI()
        with self.assertRaises(metadata.ReleaseError):
            self.run_publish(api)
        self.assertEqual(api.calls, [])

    def test_finalize_timeout_is_not_retried_or_rolled_back(self):
        self.setup_payload()
        api = FakeAPI()
        real_call = api.call

        def timeout_after_patch(method, path, data=None):
            response = real_call(method, path, data)
            if method == "PATCH":
                raise metadata.ReleaseError("response lost after server finalized")
            return response

        api.call = timeout_after_patch
        with self.assertRaises(metadata.ReleaseError):
            self.run_publish(api)
        self.assertIs(api.draft["draft"], False)
        with self.assertRaises(metadata.ReleaseError):
            self.run_publish(api)
        self.assertEqual(sum(call[0] == "PATCH" for call in api.calls), 1)


class TransportTests(unittest.TestCase):
    def test_redirect_drops_github_token(self):
        request = urllib.request.Request("https://api.github.com/repos/bloxy-studios/bluey/releases/assets/123",
                                         headers={"Authorization": "Bearer fake-token"})
        redirected = publisher.NoTokenRedirect().redirect_request(
            request, None, 302, "Found", {}, "https://release-assets.githubusercontent.com/asset?signature=fake")
        self.assertFalse(redirected.has_header("Authorization"))
        with self.assertRaises(metadata.ReleaseError):
            publisher.NoTokenRedirect().redirect_request(request, None, 302, "Found", {}, "https://evil.test/asset")

    def test_annotated_tag_resolution_is_bounded_and_exact(self):
        with patch.dict("os.environ", {"GH_TOKEN": "fake-token"}):
            api = publisher.GitHub()
        with patch.object(api, "call", side_effect=[
            {"ref": "refs/tags/v1.2.3", "object": {"type": "tag", "sha": "b" * 40}},
            {"object": {"type": "commit", "sha": COMMIT}}]):
            self.assertEqual(api.remote_commit("v1.2.3"), COMMIT)
        with patch.object(api, "call", return_value={"ref": "refs/tags/other", "object": {"type": "commit", "sha": COMMIT}}):
            with self.assertRaises(metadata.ReleaseError):
                api.remote_commit("v1.2.3")


if __name__ == "__main__":
    unittest.main()
