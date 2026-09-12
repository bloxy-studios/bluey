"""Portable fixtures only: these tests never claim native signing/notarization."""

import base64
import copy
import hashlib
import json
import os
from pathlib import Path
import shutil
import sys
import tempfile
import unittest
from unittest.mock import patch

sys.path.insert(0, str(Path(__file__).resolve().parents[1]))
import release_metadata as metadata
from release_credentials import REQUIRED, check_credentials
from verify_macos import verification

SOURCE_ROOT = Path(__file__).resolve().parents[3]
COMMIT = "a" * 40
TAG = "v0.1.0"
PUBLISHED_AT = "2026-09-13T03:00:00Z"


def signature_text(name):
    """A base64 stand-in for the minisign `.sig` Tauri writes next to an updater archive."""
    return base64.b64encode(("portable fixture signature, not minisign: " + name).encode() * 2).decode()


def fixture_root(root):
    (root / "src-tauri").mkdir(parents=True)
    for relative in ("package.json", "src-tauri/tauri.conf.json", "src-tauri/Cargo.toml"):
        shutil.copyfile(SOURCE_ROOT / relative, root / relative)
    return root


def replace_json(path, change):
    data = json.loads(path.read_text())
    change(data)
    path.write_text(json.dumps(data))


class ReleaseFixture(unittest.TestCase):
    def setUp(self):
        self.temp = tempfile.TemporaryDirectory()
        self.addCleanup(self.temp.cleanup)
        self.base = Path(self.temp.name).resolve()
        self.root = fixture_root(self.base / "repo")
        self.version = metadata.source_metadata(self.root)["version"]
        self.tag = "v" + self.version
        self.expected = metadata.provenance(self.tag, COMMIT, "1234", "1")
        self.incoming, self.output = self.base / "incoming", self.base / "payload"

    def set_version(self, version):
        """Rewrite the fixture's three root sources to `version` (stable or prerelease).

        The fixture starts from the live repository's sources, so tests that assert a
        stable or a prerelease outcome must pin the version they test instead of
        assuming what the checked-out tree currently says."""
        for relative in ("package.json", "src-tauri/tauri.conf.json", "src-tauri/Cargo.toml"):
            path = self.root / relative
            path.write_text(path.read_text().replace('"' + self.version + '"', '"' + version + '"', 1))
        self.version, self.tag = version, "v" + version
        self.expected = metadata.provenance(self.tag, COMMIT, "1234", "1")

    def pair(self):
        source = metadata.source_metadata(self.root, self.tag)
        # Deliberately not Tauri example names: basename must come from actual files.
        for target, name, content in [("mac-arm64", "Bluey actual ARM + signed", b"arm installer fixture"),
                                       ("mac-x64", "Bluey actual Intel", b"intel installer fixture")]:
            directory = self.incoming / target
            directory.mkdir(parents=True)
            dmg = directory / (name + ".dmg")
            dmg.write_bytes(content)
            archive = directory / (name + metadata.UPDATER_SUFFIX)
            archive.write_bytes(b"updater archive fixture for " + content)
            signature = directory / (archive.name + ".sig")
            signature.write_text(signature_text(name) + "\n")
            metadata.write_json(directory / metadata.RECORD, {
                "schemaVersion": 1, "provenance": self.expected, "version": self.version,
                "installer": metadata.installer_entry(target, dmg, source["minimumOsVersion"]),
                "updater": metadata.updater_entry(target, archive, signature),
                "verification": verification(target)})
        return self.incoming

    def assemble(self):
        return metadata.assemble(self.root, self.incoming, self.output, self.expected, PUBLISHED_AT)


class SourceAndTagTests(ReleaseFixture):
    def test_actual_project_versions_are_read_not_assumed(self):
        actual = metadata.source_metadata(SOURCE_ROOT)
        self.assertEqual(actual["version"], json.loads((SOURCE_ROOT / "package.json").read_text())["version"])
        self.assertEqual(set(actual["sourceVersions"].values()), {actual["version"]})
        self.assertIn("Cargo.toml:package.version", actual["sourceVersions"])
        self.assertEqual(actual["minimumOsVersion"], "14.0")

    def test_valid_tag_semver_classification(self):
        for tag, prerelease in [("v1.2.3", False), ("1.2.3", False), ("v1.2.3+build.01", False),
                                ("v1.2.3-rc.1", True), ("1.0.0-beta+sha.123", True)]:
            with self.subTest(tag=tag):
                self.assertEqual(metadata.version_tag(tag), (tag.removeprefix("v"), prerelease))

    def test_rejects_tag_argument_and_output_injection(self):
        for tag in ("--help", "v1.2.3\ncommit=bad", "refs/tags/v1.2.3", "$(touch hacked)", "v01.2.3", "v1.2", "v1.2.3-01",
                    "v1.2.3;echo", "v1.2.3 ", "v1.2.3/evil", "v1.2.3..", "v1.2.3-", "v１.2.3"):
            with self.subTest(tag=tag), self.assertRaises(metadata.ReleaseError):
                metadata.version_tag(tag)

    def test_tag_and_each_manifest_mismatch(self):
        with self.assertRaises(metadata.ReleaseError):
            metadata.source_metadata(self.root, "v99.99.99")
        for relative in ("package.json", "src-tauri/tauri.conf.json", "src-tauri/Cargo.toml"):
            original = (self.root / relative).read_text()
            with self.subTest(manifest=relative):
                (self.root / relative).write_text(original.replace('"' + self.version + '"', '"99.99.99"', 1))
                with self.assertRaises(metadata.ReleaseError):
                    metadata.source_metadata(self.root, self.tag)
                (self.root / relative).write_text(original)

    def test_explicit_and_inherited_workspace_version(self):
        cargo = self.root / "src-tauri/Cargo.toml"
        cargo.write_text('[package]\nversion.workspace = true\n[workspace.package]\nversion = "' + self.version + '"\n')
        result = metadata.source_metadata(self.root, self.tag)
        self.assertEqual(result["sourceVersions"]["Cargo.toml:workspace.package.version"], self.version)
        cargo.write_text('[package]\nversion = "' + self.version + '"\n[workspace.package]\nversion = "99.99.99"\n')
        with self.assertRaises(metadata.ReleaseError):
            metadata.source_metadata(self.root, self.tag)

    def test_restricted_cargo_parser_fails_closed(self):
        cargo = self.root / "src-tauri/Cargo.toml"
        for content in ('[package]\nversion.workspace=true\n', '[package]\nversion="1.2.3"\nversion="1.2.3"\n',
                        '[package.version]\nworkspace=true\n', '[package]\n"version"="1.2.3"\n',
                        '[package]\ndescription="""\nversion="1.2.3"\n"""\n', '[package]\n[package]\nversion="1.2.3"\n'):
            with self.subTest(content=content):
                cargo.write_text(content)
                with self.assertRaises(metadata.ReleaseError):
                    metadata.cargo_versions(cargo)

    def test_config_signing_or_minimum_changes_fail(self):
        config = self.root / "src-tauri/tauri.conf.json"
        original = config.read_text()
        for key, value in [("minimumSystemVersion", None), ("minimumSystemVersion", "latest"),
                           ("hardenedRuntime", False), ("skipStapling", True), ("entitlements", "other.plist")]:
            with self.subTest(key=key):
                replace_json(config, lambda data: data["bundle"]["macOS"].update({key: value}))
                with self.assertRaises(metadata.ReleaseError):
                    metadata.source_metadata(self.root, self.tag)
                config.write_text(original)
        (self.root / "src-tauri/tauri.macos.conf.json").write_text("{}")
        with self.assertRaises(metadata.ReleaseError):
            metadata.source_metadata(self.root, self.tag)

    def test_head_must_equal_existing_tag_and_preflight_commit(self):
        with patch.object(metadata, "git", side_effect=[COMMIT, "b" * 40]):
            with self.assertRaises(metadata.ReleaseError):
                metadata.checked_tag(self.root, self.tag)
        with patch.object(metadata, "git", return_value=COMMIT):
            with self.assertRaises(metadata.ReleaseError):
                metadata.checked_tag(self.root, self.tag, "b" * 40)
            self.assertEqual(metadata.checked_tag(self.root, self.tag, COMMIT), COMMIT)


class ManifestTests(ReleaseFixture):
    def test_actual_names_sizes_hashes_and_contract(self):
        self.pair()
        data = self.assemble()
        self.assertEqual(set(data), {"schemaVersion", "version", "tag", "repository", "installers"})
        self.assertEqual(data["repository"], "bloxy-studios/bluey")
        self.assertLess((self.output / metadata.MANIFEST).stat().st_size, 65536)
        self.assertEqual(data["installers"][0]["asset"], "Bluey actual ARM + signed.dmg")
        for entry in data["installers"]:
            raw = (self.output / entry["asset"]).read_bytes()
            self.assertEqual(entry["bytes"], len(raw))
            self.assertEqual(entry["sha256"], hashlib.sha256(raw).hexdigest())
        self.assertEqual(metadata.validate_payload(self.root, self.output, self.tag), data)
        # The updater feed is Tauri's format, points only at this release's own assets by their real names.
        feed = metadata.read_json(self.output / metadata.UPDATER_FEED)
        self.assertEqual(set(feed), {"version", "notes", "pub_date", "platforms"})
        self.assertEqual((feed["version"], feed["pub_date"]), (self.version, PUBLISHED_AT))
        self.assertEqual(set(feed["platforms"]), {"darwin-aarch64", "darwin-x86_64"})
        arm = feed["platforms"]["darwin-aarch64"]
        self.assertEqual(arm["url"], "https://github.com/bloxy-studios/bluey/releases/download/"
                         + self.tag + "/Bluey%20actual%20ARM%20%2B%20signed.app.tar.gz")
        self.assertEqual(arm["signature"], signature_text("Bluey actual ARM + signed"))
        self.assertEqual(len((self.output / metadata.CHECKSUMS).read_text().splitlines()), 4)
        self.assertEqual(len(list(self.output.iterdir())), 9)
        self.assertEqual([p.name for p in metadata.payload_upload_order(self.output, data)][-3:],
                         [metadata.CHECKSUMS, metadata.UPDATER_FEED, metadata.MANIFEST])

    def test_missing_empty_extra_duplicate_and_wrong_provenance_fail_before_output(self):
        cases = ["missing", "empty", "extra-file", "extra-target", "duplicate-name", "duplicate-target", "wrong-attempt",
                 "wrong-run", "wrong-commit", "wrong-repo", "unsigned", "bad-check-type", "bad-bytes", "tampered", "wrong-minimum",
                 "missing-signature", "tampered-archive", "swapped-signature", "binary-signature", "wrong-updater-target",
                 "unverified-updater"]
        for case in cases:
            with self.subTest(case=case):
                self.pair()
                arm = self.incoming / "mac-arm64"
                intel = self.incoming / "mac-x64"
                record = arm / metadata.RECORD
                if case == "missing":
                    shutil.rmtree(intel)
                elif case == "empty":
                    next(arm.glob("*.dmg")).write_bytes(b"")
                elif case == "extra-file":
                    (arm / "source.zip").write_bytes(b"extra")
                elif case == "extra-target":
                    (self.incoming / "linux-x64").mkdir()
                elif case == "duplicate-name":
                    data = metadata.read_json(intel / metadata.RECORD)
                    old = intel / data["installer"]["asset"]
                    name = next(arm.glob("*.dmg")).name
                    old.rename(intel / name)
                    replace_json(intel / metadata.RECORD, lambda value: value["installer"].update(asset=name))
                elif case == "duplicate-target":
                    replace_json(record, lambda value: value["installer"].update(target="mac-x64"))
                elif case in ("wrong-attempt", "wrong-run", "wrong-commit", "wrong-repo"):
                    field, value = {"wrong-attempt": ("runAttempt", "2"), "wrong-run": ("runId", "5678"),
                                    "wrong-commit": ("commit", "b" * 40), "wrong-repo": ("repository", "attacker/repo")}[case]
                    replace_json(record, lambda data: data["provenance"].update({field: value}))
                elif case == "unsigned":
                    replace_json(record, lambda data: data["verification"].update(appStaple=False))
                elif case == "bad-check-type":
                    replace_json(record, lambda data: data["verification"].update(appStaple=1))
                elif case == "bad-bytes":
                    replace_json(record, lambda data: data["installer"].update(bytes=True))
                elif case == "tampered":
                    next(arm.glob("*.dmg")).write_bytes(b"changed after verification")
                elif case == "wrong-minimum":
                    replace_json(record, lambda data: data["installer"].update(minimumOsVersion="11.0"))
                elif case == "missing-signature":
                    next(arm.glob("*.sig")).unlink()
                elif case == "tampered-archive":
                    next(arm.glob("*" + metadata.UPDATER_SUFFIX)).write_bytes(b"replaced after verification")
                elif case == "swapped-signature":
                    next(arm.glob("*.sig")).write_text(signature_text("someone else"))
                elif case == "binary-signature":
                    next(arm.glob("*.sig")).write_bytes(b"\x00\xff" * 64)
                elif case == "wrong-updater-target":
                    replace_json(record, lambda data: data["updater"].update(target="mac-x64", platform="darwin-x86_64"))
                elif case == "unverified-updater":
                    replace_json(record, lambda data: data["verification"].update(updaterApp=False))
                with self.assertRaises(metadata.ReleaseError):
                    self.assemble()
                self.assertFalse(self.output.exists())
                shutil.rmtree(self.incoming)

    def test_updater_feed_contract(self):
        self.pair()
        self.assemble()
        feed = metadata.read_json(self.output / metadata.UPDATER_FEED)
        updaters = metadata.payload_updaters(self.output, feed)
        self.assertEqual([entry["target"] for entry in updaters], ["mac-arm64", "mac-x64"])
        self.assertEqual(metadata.validate_feed(feed, self.version, self.tag, updaters), feed)
        for mutation in (lambda x: x.update(version="9.9.9"), lambda x: x.update(pub_date="yesterday"),
                         lambda x: x.update(extra=True), lambda x: x.pop("notes"),
                         lambda x: x["platforms"].pop("darwin-x86_64"),
                         lambda x: x["platforms"].update({"linux-x86_64": x["platforms"]["darwin-x86_64"]}),
                         lambda x: x["platforms"]["darwin-aarch64"].update(url="https://evil.test/Bluey.app.tar.gz"),
                         lambda x: x["platforms"]["darwin-aarch64"].update(
                             url=x["platforms"]["darwin-x86_64"]["url"]),
                         lambda x: x["platforms"]["darwin-aarch64"].update(signature=signature_text("other"))):
            modified = copy.deepcopy(feed)
            mutation(modified)
            with self.assertRaises(metadata.ReleaseError):
                metadata.validate_feed(modified, self.version, self.tag, updaters)
        with self.assertRaises(metadata.ReleaseError):
            metadata.updater_feed(self.version, self.tag, updaters[:1], PUBLISHED_AT)
        with self.assertRaises(metadata.ReleaseError):
            metadata.updater_feed(self.version, self.tag, updaters + updaters[:1], PUBLISHED_AT)
        with self.assertRaises(metadata.ReleaseError):
            metadata.updater_feed(self.version, self.tag, updaters, "2026-09-13 03:00")
        for ref in ("../x", "v1.2.3/evil", "-flag", "nightly x", "", "v1.2.3\n"):
            with self.subTest(ref=ref), self.assertRaises(metadata.ReleaseError):
                metadata.asset_url(ref, "Bluey.app.tar.gz")
        self.assertEqual(metadata.asset_url("nightly", "Bluey_0.1.3-nightly.20260913_aarch64.app.tar.gz"),
                         "https://github.com/bloxy-studios/bluey/releases/download/nightly/"
                         "Bluey_0.1.3-nightly.20260913_aarch64.app.tar.gz")
        self.assertEqual(metadata.updater_name_for("Bluey_0.1.2_aarch64.dmg"), "Bluey_0.1.2_aarch64.app.tar.gz")

    def test_signature_files_must_be_bounded_base64_text(self):
        path = self.base / "x.sig"
        for content in (b"\x00\xff" * 64, b"short", b"A" * (8 * 1024 + 1), "ünïcode".encode() * 20,
                        b"has spaces inside but also $ shell characters, so it is not base64 text at all"):
            path.write_bytes(content)
            with self.subTest(content=content[:12]), self.assertRaises(metadata.ReleaseError):
                metadata.read_signature(path)
        path.write_text(signature_text("ok") + "\n")
        self.assertEqual(metadata.read_signature(path), signature_text("ok"))
        with self.assertRaises(metadata.ReleaseError):
            metadata.updater_entry("mac-arm64", self.base / "Bluey.app.tar.gz", self.base / "other.sig")

    def test_unsafe_names_rejected(self):
        for name in ("../a.dmg", "-option.dmg", "a#label.dmg", "a\nb.dmg", "a%20b.dmg", "a\\b.dmg", "a..b.dmg", "a.dmg ", "é.dmg", "file.zip"):
            with self.subTest(name=name), self.assertRaises(metadata.ReleaseError):
                metadata.safe_name(name, ".dmg")

    def test_manifest_strict_types_duplicates_and_unknown_platform(self):
        self.pair()
        data = self.assemble()
        for mutation in (lambda x: x.update(schemaVersion=True), lambda x: x.update(repository="other/repo"),
                         lambda x: x["installers"][0].update(target="windows-x64"),
                         lambda x: x["installers"][0].update(sha256="A" * 64),
                         lambda x: x["installers"][0].update(bytes=0),
                         lambda x: x["installers"][1].update(asset=x["installers"][0]["asset"].upper()),
                         lambda x: x.update(extra=True), lambda x: x["installers"][0].update(url="https://evil.test/a")):
            modified = copy.deepcopy(data)
            mutation(modified)
            with self.assertRaises(metadata.ReleaseError):
                metadata.validate_manifest(modified)

    def test_symlink_metadata_installer_and_directory_rejected(self):
        self.pair()
        arm = self.incoming / "mac-arm64"
        dmg = next(arm.glob("*.dmg"))
        contents = dmg.read_bytes()
        other = self.base / "external.dmg"
        other.write_bytes(contents)
        dmg.unlink()
        dmg.symlink_to(other)
        with self.assertRaises(metadata.ReleaseError):
            self.assemble()
        dmg.unlink()
        dmg.write_bytes(contents)
        arm.rename(self.base / "arm")
        arm.symlink_to(self.base / "arm", target_is_directory=True)
        with self.assertRaises(metadata.ReleaseError):
            self.assemble()

    def test_json_duplicate_keys_and_size_limit(self):
        path = self.base / "bad.json"
        path.write_text('{"schemaVersion":1,"schemaVersion":1}')
        with self.assertRaises(metadata.ReleaseError):
            metadata.read_json(path)
        path.write_text(" " * (65536 + 1))
        with self.assertRaises(metadata.ReleaseError):
            metadata.read_json(path)

    def test_payload_checksum_drift_and_extra_asset_fail(self):
        self.pair()
        self.assemble()
        path = self.output / metadata.CHECKSUMS
        path.write_text(path.read_text() + "unexpected\n")
        with self.assertRaises(metadata.ReleaseError):
            metadata.validate_payload(self.root, self.output, self.tag)


class CredentialTests(unittest.TestCase):
    def valid(self):
        return {"APPLE_CERTIFICATE_P12": "ZmFrZS1jZXJ0", "APPLE_CERTIFICATE_PASSWORD": "test-password",
                "APPLE_SIGNING_IDENTITY": "Developer ID Application: Fixture (ABCDEFGHIJ)",
                "APPLE_ID": "fixture@example.test", "APPLE_PASSWORD": "test-app-password", "APPLE_TEAM_ID": "ABCDEFGHIJ"}

    def test_all_six_inputs_required_and_only_names_in_errors(self):
        self.assertEqual(check_credentials(self.valid()), b"fake-cert")
        for key in REQUIRED:
            values = self.valid()
            del values[key]
            with self.subTest(key=key), self.assertRaises(metadata.ReleaseError) as raised:
                check_credentials(values)
            self.assertIn(key, str(raised.exception))
            self.assertNotIn("test-password", str(raised.exception))
            self.assertNotIn("test-app-password", str(raised.exception))

    def test_certificate_and_real_identity_shape(self):
        for key, value in [("APPLE_SIGNING_IDENTITY", "-"), ("APPLE_TEAM_ID", "wrong"),
                           ("APPLE_CERTIFICATE_P12", "not base64!!!!")]:
            values = self.valid()
            values[key] = value
            with self.assertRaises(metadata.ReleaseError):
                check_credentials(values)

    def test_optional_oauth_not_required(self):
        self.assertNotIn("BLUEY_ANTIGRAVITY_CLIENT_SECRET", REQUIRED)
        check_credentials(self.valid())


if __name__ == "__main__":
    unittest.main()
