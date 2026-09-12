"""Nightly channel planning and rolling-prerelease protocol tests; mock API only, no network."""

import copy
import datetime
from pathlib import Path
import shutil
import sys
import unittest

sys.path.insert(0, str(Path(__file__).resolve().parents[1]))
import nightly
import publish_release as publisher
import release_metadata as metadata
from test_metadata import COMMIT, ReleaseFixture, signature_text

TODAY = datetime.datetime(2026, 9, 13, 3, 0, tzinfo=datetime.timezone.utc)


class NightlyFakeAPI:
    def __init__(self, release=None):
        self.release = release
        self.events = []  # ("call", method, path) | ("upload", name) in the order they happened
        self.calls = []
        self.uploads = []
        self.facts = {}
        self.next_id = 500

    def find_release(self, tag):
        assert tag == nightly.NIGHTLY_TAG
        return copy.deepcopy(self.release)

    def call(self, method, path, data=None):
        self.calls.append((method, path, copy.deepcopy(data)))
        self.events.append(("call", method, path))
        if method == "POST" and path.endswith("/releases"):
            self.release = {"id": 77, "assets": [], "tag_name": data["tag_name"], **copy.deepcopy(data)}
            return copy.deepcopy(self.release)
        if method == "PATCH" and path == publisher.PREFIX + "/git/refs/tags/nightly":
            return {"ref": "refs/tags/nightly", "object": {"type": "commit", "sha": data["sha"]}}
        if method == "DELETE" and "/releases/assets/" in path:
            asset_id = int(path.rsplit("/", 1)[1])
            self.release["assets"] = [asset for asset in self.release["assets"] if asset["id"] != asset_id]
            return None
        if method == "PATCH" and path == publisher.PREFIX + f"/releases/{self.release['id']}":
            self.release.update(data)
            return copy.deepcopy(self.release)
        raise AssertionError("Unexpected API request " + method + " " + path)

    def upload(self, release_id, path):
        assert release_id == self.release["id"]
        self.uploads.append(path.name)
        self.events.append(("upload", path.name))
        facts = metadata.file_facts(path)
        self.next_id += 1
        asset = {"id": self.next_id, "name": path.name, "size": facts["bytes"], "state": "uploaded"}
        self.facts[asset["id"]] = facts
        self.release["assets"].append(asset)
        return copy.deepcopy(asset)

    def asset_facts(self, asset_id):
        return dict(self.facts[asset_id])


class NightlyVersionTests(unittest.TestCase):
    def test_nightly_versions_sort_between_the_sources_and_the_next_stable(self):
        self.assertEqual(nightly.nightly_version("0.1.2", "20260913"), "0.1.3-nightly.20260913")
        self.assertEqual(nightly.nightly_version("v1.9.9-rc.1", "20260913"), "1.9.10-nightly.20260913")
        for version in ("0.1.3-nightly.20260913", "12.0.0-nightly.20991231"):
            self.assertEqual(nightly.check_version(version), version)
            self.assertTrue(metadata.version_tag(version)[1])
        for bad in ("0.1.3", "0.1.3-rc.1", "0.1.3-nightly.2026091", "0.1.3-nightly.20260913+build", "0.1.3-nightly.20260913\n",
                    "v0.1.3-nightly.20260913", "0.1.3-nightly.20260913; rm -rf /", 42, None):
            with self.subTest(bad=bad), self.assertRaises(metadata.ReleaseError):
                nightly.check_version(bad)
        with self.assertRaises(metadata.ReleaseError):
            nightly.nightly_version("0.1.2", "2026-09-13")
        with self.assertRaises(metadata.ReleaseError):
            nightly.nightly_version("not-a-version", "20260913")


class NightlyPlanTests(ReleaseFixture):
    def expected_version(self):
        return nightly.nightly_version(self.version, "20260913")

    def test_plan_skips_unchanged_main_unless_forced(self):
        body = nightly.body_for(self.expected_version(), COMMIT, "1")
        api = NightlyFakeAPI({"id": 77, "body": body, "prerelease": True, "draft": False, "assets": []})
        result = nightly.plan(self.root, COMMIT, api, False, TODAY)
        self.assertEqual(result, {"build": False, "version": self.expected_version(), "commit": COMMIT})
        self.assertTrue(nightly.plan(self.root, COMMIT, api, True, TODAY)["build"])
        self.assertTrue(nightly.plan(self.root, "b" * 40, api, False, TODAY)["build"])
        self.assertTrue(nightly.plan(self.root, COMMIT, NightlyFakeAPI(None), False, TODAY)["build"])
        # A body without the marker (edited by hand) means "unknown", so build.
        api.release["body"] = "someone edited this"
        self.assertTrue(nightly.plan(self.root, COMMIT, api, False, TODAY)["build"])
        for commit in ("--evil", "", "b" * 39, COMMIT + "\n"):
            with self.subTest(commit=commit), self.assertRaises(metadata.ReleaseError):
                nightly.plan(self.root, commit, api, False, TODAY)
        self.assertEqual(api.calls, [])


class NightlyPublishTests(ReleaseFixture):
    def setUp(self):
        super().setUp()
        self.nightly_version = nightly.nightly_version(self.version, "20260913")
        self.incoming = self.base / "nightly-incoming"
        self.assets = self.base / "nightly-assets"

    def artifacts(self, version=None):
        version = version or self.nightly_version
        for target, arch in (("mac-arm64", "aarch64"), ("mac-x64", "x64")):
            # upload-artifact keeps Tauri's dmg/ and macos/ directories.
            dmg_dir, macos_dir = self.incoming / target / "dmg", self.incoming / target / "macos"
            dmg_dir.mkdir(parents=True)
            macos_dir.mkdir(parents=True)
            (dmg_dir / f"Bluey_{version}_{arch}.dmg").write_bytes(b"nightly dmg " + arch.encode())
            (macos_dir / "Bluey.app.tar.gz").write_bytes(b"nightly archive " + arch.encode())
            (macos_dir / "Bluey.app.tar.gz.sig").write_text(signature_text("nightly " + arch) + "\n")

    def publish(self, api, version=None):
        shutil.rmtree(self.assets, ignore_errors=True)  # each attempt collects into a fresh staging directory
        return nightly.publish(api, self.root, self.incoming, version or self.nightly_version, COMMIT, "1234", "1",
                               output=self.assets)

    def expected_names(self):
        names = set()
        for arch in ("aarch64", "x64"):
            stem = f"Bluey_{self.nightly_version}_{arch}"
            names.update({stem + ".dmg", stem + metadata.UPDATER_SUFFIX, stem + metadata.UPDATER_SUFFIX + ".sig"})
        return names | {metadata.CHECKSUMS, metadata.UPDATER_FEED}

    def test_first_nightly_creates_the_prerelease_and_uploads_the_feed_last(self):
        self.artifacts()
        api = NightlyFakeAPI(None)
        self.assertEqual(self.publish(api), 77)
        method, path, data = api.calls[0]
        self.assertEqual((method, path), ("POST", publisher.PREFIX + "/releases"))
        self.assertEqual((data["tag_name"], data["target_commitish"], data["prerelease"], data["draft"], data["make_latest"]),
                         ("nightly", COMMIT, True, False, "false"))
        self.assertIn(f"<!-- bluey-nightly commit={COMMIT} version={self.nightly_version} -->", data["body"])
        self.assertEqual(len(api.uploads), 8)
        self.assertTrue(all(name.endswith(".dmg") for name in api.uploads[:2]))
        self.assertEqual(api.uploads[-2:], [metadata.CHECKSUMS, metadata.UPDATER_FEED])
        self.assertEqual(set(api.uploads), self.expected_names())
        self.assertEqual({asset["name"] for asset in api.release["assets"]}, self.expected_names())
        feed = metadata.read_json(self.assets / metadata.UPDATER_FEED)
        self.assertEqual(feed["version"], self.nightly_version)
        self.assertEqual(feed["platforms"]["darwin-aarch64"]["url"],
                         f"https://github.com/bloxy-studios/bluey/releases/download/nightly/Bluey_{self.nightly_version}_aarch64.app.tar.gz")
        self.assertEqual(feed["platforms"]["darwin-x86_64"]["signature"], signature_text("nightly x64"))
        self.assertEqual(len((self.assets / metadata.CHECKSUMS).read_text().splitlines()), 4)
        final = api.calls[-1]
        self.assertEqual((final[0], final[1]), ("PATCH", publisher.PREFIX + "/releases/77"))
        self.assertEqual((final[2]["prerelease"], final[2]["draft"], final[2]["make_latest"]), (True, False, "false"))
        self.assertNotIn("DELETE", [call[0] for call in api.calls])
        # Only the rolling tag is ever touched; version tags and their releases never are.
        self.assertFalse(any("/git/refs/tags/v" in call[1] for call in api.calls))
        self.assertFalse(any(call[1].endswith("/releases/latest") for call in api.calls))

    def test_rerun_moves_the_tag_and_replaces_rolling_files_before_removing_stale_assets_last(self):
        self.artifacts()
        previous = nightly.nightly_version(self.version, "20260912")
        stale = [{"id": 1, "name": f"Bluey_{previous}_aarch64.dmg"}, {"id": 2, "name": metadata.UPDATER_FEED},
                 {"id": 3, "name": metadata.CHECKSUMS}, {"id": 4, "name": f"Bluey_{previous}_x64.app.tar.gz.sig"}]
        api = NightlyFakeAPI({"id": 77, "tag_name": "nightly", "prerelease": True, "draft": False, "assets": stale,
                              "body": nightly.body_for(previous, "b" * 40, "1")})
        self.publish(api)
        self.assertEqual(api.events[0], ("call", "PATCH", publisher.PREFIX + "/git/refs/tags/nightly"))
        self.assertEqual(api.calls[0][2], {"sha": COMMIT, "force": True})
        first_upload = next(i for i, event in enumerate(api.events) if event[0] == "upload")
        last_upload = max(i for i, event in enumerate(api.events) if event[0] == "upload")
        deletes = {int(event[2].rsplit("/", 1)[1]): i for i, event in enumerate(api.events) if event[1] == "DELETE"}
        self.assertEqual(set(deletes), {1, 2, 3, 4})
        # Rolling files are replaced (deleted, then re-uploaded); yesterday's bundles go only after the new set is complete.
        self.assertLess(max(deletes[2], deletes[3]), first_upload)
        self.assertGreater(min(deletes[1], deletes[4]), last_upload)
        self.assertLess(last_upload, api.events.index(("call", "PATCH", publisher.PREFIX + "/releases/77")))
        self.assertEqual({asset["name"] for asset in api.release["assets"]}, self.expected_names())
        self.assertIn(f"commit={COMMIT} version={self.nightly_version}", api.release["body"])
        self.assertEqual(api.uploads[-1], metadata.UPDATER_FEED)

    def test_refuses_non_nightly_versions_drafts_and_mismatched_artifacts(self):
        self.artifacts()
        api = NightlyFakeAPI(None)
        for version in ("1.2.3", self.version, "0.1.3-rc.1"):
            with self.subTest(version=version), self.assertRaises(metadata.ReleaseError):
                self.publish(api, version)
        self.assertEqual(api.calls, [])
        self.assertEqual(api.uploads, [])
        for release in ({"id": 77, "prerelease": True, "draft": True, "assets": []},
                        {"id": 77, "prerelease": False, "draft": False, "assets": []}):
            api = NightlyFakeAPI(release)
            with self.subTest(release=release), self.assertRaises(metadata.ReleaseError):
                self.publish(api)
            self.assertEqual(api.calls, [])
        # A DMG built with another version cannot be published as tonight's nightly.
        other = nightly.nightly_version(self.version, "20260901")
        dmg = next((self.incoming / "mac-arm64").rglob("*.dmg"))
        dmg.rename(dmg.with_name(dmg.name.replace(self.nightly_version, other)))
        api = NightlyFakeAPI(None)
        with self.assertRaises(metadata.ReleaseError):
            self.publish(api)
        self.assertEqual(api.calls, [])
        self.assertFalse(self.assets.exists() and any(self.assets.iterdir()) and api.uploads)


if __name__ == "__main__":
    unittest.main()
