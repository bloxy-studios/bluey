"""Bounded Apple-command mocks exercise failure propagation, NOT native trust acceptance."""

import io
import json
import os
from pathlib import Path
import plistlib
import shutil
import sys
import tarfile
import unittest
from unittest.mock import patch

sys.path.insert(0, str(Path(__file__).resolve().parents[1]))
import release_metadata as metadata
import verify_macos as native
from test_metadata import COMMIT, ReleaseFixture, SOURCE_ROOT, signature_text

TEAM = "ABCDEFGHIJ"


class NativeGateTests(ReleaseFixture):
    def setUp(self):
        super().setUp()
        for relative in ("src-tauri/entitlements.plist", "src-tauri/swift/BlueyHelper/bluey-helper.entitlements"):
            path = self.root / relative
            path.parent.mkdir(parents=True, exist_ok=True)
            shutil.copyfile(SOURCE_ROOT / relative, path)
        self.source = metadata.source_metadata(self.root)
        self.bundle = self.root / "src-tauri/target/aarch64-apple-darwin/release/bundle"
        self.app = self.bundle / "macos" / (self.source["productName"] + ".app")
        self.app.mkdir(parents=True)
        self.app_contents(self.app)
        self.dmg = self.bundle / "dmg/Actually discovered signed ARM.dmg"
        self.dmg.parent.mkdir(parents=True)
        self.dmg.write_bytes(b"MOCK ONLY NOT A REAL DMG")
        # Tauri writes the updater bundle next to the app under the product name, whatever the target.
        self.archive = self.bundle / "macos" / (self.source["productName"] + metadata.UPDATER_SUFFIX)
        self.write_archive(self.archive, self.app)
        self.archive_signature = self.archive.with_name(self.archive.name + ".sig")
        self.archive_signature.write_text(signature_text("native fixture") + "\n")
        self.destination = self.base / "verified/mac-arm64"
        self.calls = []
        self.fail = None
        self.arch = "arm64"
        self.signature = ("Authority=Developer ID Application: Fixture (ABCDEFGHIJ)\n"
                          "TeamIdentifier=ABCDEFGHIJ\nTimestamp=Jan 1, 2026 at 00:00:00\n"
                          "CodeDirectory v=20500 size=999 flags=0x10000(runtime) hashes=1+1\n")
        self.notary_status = "Accepted"

    def app_contents(self, app):
        binaries = app / "Contents/MacOS"
        binaries.mkdir(parents=True, exist_ok=True)
        for name in ("bluey", "bluey-agent", "bluey-helper"):
            binary = binaries / name
            binary.write_bytes(b"MOCK EXECUTABLE")
            binary.chmod(0o755)
        info = {"CFBundleIdentifier": self.source["identifier"], "CFBundleShortVersionString": self.source["version"],
                "LSMinimumSystemVersion": self.source["minimumOsVersion"], "CFBundleExecutable": "bluey"}
        (app / "Contents/Info.plist").write_bytes(plistlib.dumps(info))

    def write_archive(self, path, app, arcname=None, extra=()):
        with tarfile.open(path, "w:gz") as tar:
            tar.add(app, arcname=arcname or app.name)
            for name, content in extra:
                info = tarfile.TarInfo(name)
                info.size = len(content)
                tar.addfile(info, io.BytesIO(content))

    def apple(self, *args):
        self.calls.append(args)
        if self.fail and self.fail(args):
            raise metadata.ReleaseError("mock native command rejected")
        tool = Path(args[0]).name
        if tool == "codesign" and "--entitlements" in args:
            return (SOURCE_ROOT / "src-tauri/entitlements.plist").read_bytes(), b""
        if tool == "codesign" and "--display" in args:
            return b"", self.signature.encode()
        if tool == "lipo":
            return self.arch.encode(), b""
        if tool == "xcrun" and "notarytool" in args:
            return json.dumps({"status": self.notary_status}).encode(), b""
        if tool == "hdiutil" and "attach" in args:
            mount = Path(args[args.index("-mountpoint") + 1])
            shutil.copytree(self.app, mount / self.app.name)
        return b"", b""

    def stage(self):
        with patch.object(native.sys, "platform", "darwin"), patch.object(native, "apple", side_effect=self.apple), \
                patch.object(native, "checked_tag", return_value=COMMIT), \
                patch.dict(os.environ, {"APPLE_ID": "fixture@example.test", "APPLE_PASSWORD": "not-a-real-secret", "APPLE_TEAM_ID": TEAM}):
            native.stage(self.root, "mac-arm64", self.expected, self.destination, TEAM)

    def test_complete_mock_path_stages_last_and_checks_embedded_app(self):
        self.stage()
        self.assertTrue((self.destination / metadata.RECORD).is_file())
        record = metadata.read_json(self.destination / metadata.RECORD)
        self.assertEqual(record["installer"]["asset"], self.dmg.name)
        self.assertEqual(record["provenance"], self.expected)
        self.assertEqual(record["verification"], native.verification("mac-arm64"))
        self.assertTrue(any("attach" in args for args in self.calls))
        self.assertTrue(any("detach" in args for args in self.calls))
        # Built app, DMG, mounted app and the app inside the updater archive.
        self.assertEqual(sum("stapler" in args and "validate" in args for args in self.calls), 4)
        self.assertTrue(any("notarytool" in args and "submit" in args for args in self.calls))
        self.assertFalse(any("--force" in args for args in self.calls))
        self.assertTrue(any("bluey-updater-verify-" in str(arg) for args in self.calls for arg in args))
        # The updater bundle is staged under the DMG's name so each target's archive has its own asset name.
        self.assertEqual(record["updater"]["asset"], "Actually discovered signed ARM.app.tar.gz")
        self.assertEqual(record["updater"]["signature"], signature_text("native fixture"))
        self.assertEqual((self.destination / record["updater"]["asset"]).read_bytes(), self.archive.read_bytes())
        self.assertEqual({p.name for p in self.destination.iterdir()},
                         {metadata.RECORD, self.dmg.name, record["updater"]["asset"], record["updater"]["asset"] + ".sig"})

    def test_every_native_failure_prevents_eligible_record(self):
        failures = [lambda a: Path(a[0]).name == "codesign" and "--verify" in a,
                    lambda a: "stapler" in a and "validate" in a,
                    lambda a: Path(a[0]).name == "spctl",
                    lambda a: "notarytool" in a,
                    lambda a: "stapler" in a and "staple" in a,
                    lambda a: Path(a[0]).name == "hdiutil" and "verify" in a,
                    lambda a: "attach" in a,
                    lambda a: "detach" in a,
                    lambda a: Path(a[0]).name == "codesign" and "/mount/" in a[-1],
                    lambda a: any("bluey-updater-verify-" in str(x) for x in a)]
        for index, failure in enumerate(failures):
            self.fail = failure
            with self.subTest(failure=index), self.assertRaises(metadata.ReleaseError):
                self.stage()
            self.assertFalse(self.destination.exists())

    def test_rejected_notary_json_does_not_staple_or_stage(self):
        self.notary_status = "Rejected"
        with self.assertRaises(metadata.ReleaseError):
            self.stage()
        self.assertFalse(any("staple" in args for args in self.calls))
        self.assertFalse(self.destination.exists())

    def test_wrong_signature_identity_timestamp_runtime_and_arch_rejected(self):
        original = self.signature
        for signature in (original.replace("ABCDEFGHIJ", "ZZZZZZZZZZ"), "Signature=adhoc\n",
                          original.replace("Timestamp=Jan 1, 2026 at 00:00:00\n", ""), original.replace("(runtime)", "")):
            self.signature = signature
            with self.subTest(signature=signature), self.assertRaises(metadata.ReleaseError):
                self.stage()
            self.assertFalse(self.destination.exists())
        self.signature = original
        for arch in ("x86_64", "arm64 x86_64"):
            self.arch = arch
            with self.assertRaises(metadata.ReleaseError):
                self.stage()
            self.assertFalse(self.destination.exists())

    def test_bundle_metadata_and_missing_sidecar_fail_closed(self):
        info = self.app / "Contents/Info.plist"
        original = info.read_bytes()
        for key, value in [("CFBundleShortVersionString", "99.0.0"), ("LSMinimumSystemVersion", "13.0"),
                           ("CFBundleIdentifier", "wrong.app"), ("CFBundleExecutable", "../outside")]:
            changed = plistlib.loads(original)
            changed[key] = value
            info.write_bytes(plistlib.dumps(changed))
            with self.assertRaises(metadata.ReleaseError):
                self.stage()
            self.assertFalse(self.destination.exists())
            info.write_bytes(original)
        (self.app / "Contents/MacOS/bluey-helper").unlink()
        with self.assertRaises((metadata.ReleaseError, FileNotFoundError)):
            self.stage()
        self.assertFalse(self.destination.exists())

    def test_entitlements_must_survive_signing(self):
        with patch.object(native, "apple", return_value=(plistlib.dumps({"com.apple.security.get-task-allow": True}), b"")):
            with self.assertRaises(metadata.ReleaseError):
                native.entitlements(self.app, {"com.apple.security.device.audio-input": True})

    def test_native_validation_is_refused_on_linux(self):
        with patch.object(native.sys, "platform", "linux"), patch.object(native, "apple") as apple:
            with self.assertRaises(metadata.ReleaseError):
                native.stage(self.root, "mac-arm64", self.expected, self.destination, TEAM)
            apple.assert_not_called()
        self.assertFalse(self.destination.exists())

    def test_discovery_rejects_missing_or_multiple_dmgs(self):
        (self.dmg.parent / "second.dmg").write_bytes(b"another")
        with self.assertRaises(metadata.ReleaseError):
            metadata.discover_bundle(self.root, "mac-arm64")

    def test_updater_archive_gates(self):
        self.archive_signature.unlink()
        with self.assertRaises(metadata.ReleaseError):
            metadata.discover_bundle(self.root, "mac-arm64")
        self.archive_signature.write_text(signature_text("native fixture"))
        cases = {"escaping-member": lambda: self.write_archive(self.archive, self.app, extra=[("../evil", b"x")]),
                 "absolute-member": lambda: self.write_archive(self.archive, self.app, extra=[("/tmp/evil", b"x")]),
                 "two-top-level": lambda: self.write_archive(self.archive, self.app, extra=[("README", b"x")]),
                 "renamed-not-app": lambda: self.write_archive(self.archive, self.app, arcname="Bluey"),
                 "not-gzip": lambda: self.archive.write_bytes(b"not a tarball"),
                 "binary-signature": lambda: self.archive_signature.write_bytes(b"\x00\xff" * 64),
                 "second-archive": lambda: (self.archive.parent / "Other.app.tar.gz").write_bytes(b"x"),
                 "foreign-signature-name": lambda: self.archive_signature.rename(self.archive.parent / "Other.app.tar.gz.sig")}
        for case, mutate in cases.items():
            with self.subTest(case=case):
                mutate()
                self.calls = []
                with self.assertRaises(metadata.ReleaseError):
                    self.stage()
                self.assertFalse(self.destination.exists())
                for stray in self.archive.parent.glob("Other.*"):
                    stray.unlink()
                self.write_archive(self.archive, self.app)
                self.archive_signature.write_text(signature_text("native fixture"))


if __name__ == "__main__":
    unittest.main()
