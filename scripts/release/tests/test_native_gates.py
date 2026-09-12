"""Bounded Apple-command mocks exercise failure propagation, NOT native trust acceptance."""

import json
import os
from pathlib import Path
import plistlib
import shutil
import sys
import unittest
from unittest.mock import patch

sys.path.insert(0, str(Path(__file__).resolve().parents[1]))
import release_metadata as metadata
import verify_macos as native
from test_metadata import COMMIT, ReleaseFixture, SOURCE_ROOT

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
        self.assertEqual(sum("stapler" in args and "validate" in args for args in self.calls), 3)
        self.assertTrue(any("notarytool" in args and "submit" in args for args in self.calls))
        self.assertFalse(any("--force" in args for args in self.calls))

    def test_every_native_failure_prevents_eligible_record(self):
        failures = [lambda a: Path(a[0]).name == "codesign" and "--verify" in a,
                    lambda a: "stapler" in a and "validate" in a,
                    lambda a: Path(a[0]).name == "spctl",
                    lambda a: "notarytool" in a,
                    lambda a: "stapler" in a and "staple" in a,
                    lambda a: Path(a[0]).name == "hdiutil" and "verify" in a,
                    lambda a: "attach" in a,
                    lambda a: "detach" in a,
                    lambda a: Path(a[0]).name == "codesign" and "/mount/" in a[-1]]
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


if __name__ == "__main__":
    unittest.main()
