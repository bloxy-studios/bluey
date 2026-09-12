#!/usr/bin/env python3
"""Native Apple verification. A JSON marker alone is never a substitute for these checks."""

import argparse
import json
import os
from pathlib import Path
import plistlib
import re
import shutil
import subprocess
import sys
import tarfile
import tempfile

from release_metadata import (ARCHES, RECORD, UPDATER_FEED, ReleaseError, checked_tag, discover_bundle,
                              installer_entry, payload_updaters, provenance, read_json, read_signature,
                              regular_file, require, safe_name, source_metadata, updater_entry,
                              updater_name_for, validate_installer, validate_payload, validate_updater,
                              write_json)

# An updater archive is the signed app; a bigger one is not something Tauri built here.
MAX_ARCHIVE_MEMBERS = 20000
MAX_ARCHIVE_BYTES = 2 * 1024 * 1024 * 1024

CODESIGN = "/usr/bin/codesign"
SPCTL = "/usr/sbin/spctl"
XCRUN = "/usr/bin/xcrun"
HDIUTIL = "/usr/bin/hdiutil"
LIPO = "/usr/bin/lipo"


def apple(*args):
    # No shell, no interpolated command, no output of credentials or notary responses.
    # Apple tools do not need the publish token or build-time secrets in their environment.
    environment = {key: value for key, value in os.environ.items()
                   if key not in {"GH_TOKEN", "GITHUB_TOKEN", "APPLE_PASSWORD", "APPLE_CERTIFICATE_P12",
                                  "APPLE_CERTIFICATE_PASSWORD", "BLUEY_ANTIGRAVITY_CLIENT_SECRET"}}
    result = subprocess.run(list(args), stdout=subprocess.PIPE, stderr=subprocess.PIPE, env=environment)
    require(result.returncode == 0, "Apple verification failed: " + Path(args[0]).name + " " + args[1])
    return result.stdout, result.stderr


def signed(path, team, executable=False):
    apple(CODESIGN, "--verify", "--strict", "--deep", str(path))
    stdout, stderr = apple(CODESIGN, "--display", "--verbose=4", str(path))
    details = (stdout + stderr).decode("utf-8", errors="replace")
    require(re.search(r"^TeamIdentifier=" + re.escape(team) + r"$", details, re.MULTILINE) is not None,
            "Signature team does not match configured release team")
    require(re.search(r"^Authority=Developer ID Application:", details, re.MULTILINE) is not None,
            "A Developer ID Application signature is required")
    require("Signature=adhoc" not in details, "Ad-hoc signature is not distributable")
    require(re.search(r"^Timestamp=.+$", details, re.MULTILINE) is not None, "Secure signing timestamp required")
    if executable:
        require(re.search(r"^CodeDirectory .*flags=.*\bruntime\b", details, re.MULTILINE) is not None,
                "Executable is missing hardened runtime")


def entitlements(path, expected):
    stdout, _ = apple(CODESIGN, "--display", "--entitlements", ":-", str(path))
    actual = plistlib.loads(stdout)
    require(isinstance(actual, dict), "Missing signed entitlements")
    require(all(key in actual and actual[key] == value and type(actual[key]) is type(value)
                for key, value in expected.items()), "Signed entitlements do not preserve source capabilities")
    require(actual.get("com.apple.security.get-task-allow", False) is False, "Debug entitlement is forbidden")


def load_plist(path):
    with regular_file(path).open("rb") as stream:
        result = plistlib.load(stream)
    require(isinstance(result, dict), "Expected plist dictionary")
    return result


def check_app(root, app, target, team, source):
    app = Path(app)
    require(app.is_dir() and not app.is_symlink(), "App must be a non-symlink directory")
    require(app.name == source["productName"] + ".app", "Unexpected application bundle name")
    info = load_plist(app / "Contents/Info.plist")
    require(info.get("CFBundleIdentifier") == source["identifier"], "Wrong bundled app identifier")
    require(info.get("CFBundleShortVersionString") == source["version"], "Wrong bundled app version")
    require(info.get("LSMinimumSystemVersion") == source["minimumOsVersion"], "Bundled minimum OS does not match source")
    main_name = safe_name(info.get("CFBundleExecutable"))
    main_binary = regular_file(app / "Contents/MacOS" / main_name)
    require(os.access(main_binary, os.X_OK), "Main app binary is not executable")
    signed(app, team, executable=True)
    app_entitlements = load_plist(Path(root) / "src-tauri/entitlements.plist")
    entitlements(app, app_entitlements)
    # Tauri 2.11.4 signs embedded externalBin files with the app's entitlements.
    # Validate the helper's required microphone entitlement too; never --deep re-sign.
    helper_entitlements = load_plist(Path(root) / "src-tauri/swift/BlueyHelper/bluey-helper.entitlements")
    for name in (main_name, "bluey-helper", "bluey-agent"):
        binary = regular_file(app / "Contents/MacOS" / safe_name(name))
        require(os.access(binary, os.X_OK), "Bundled sidecar is not executable")
        signed(binary, team, executable=True)
        stdout, _ = apple(LIPO, "-archs", str(binary))
        require(stdout.decode("ascii").strip() == ARCHES[target], "Bundled executable has wrong/multiple architectures")
        if name == "bluey-helper":
            entitlements(binary, helper_entitlements)
        if name == "bluey-agent":
            entitlements(binary, {"com.apple.security.cs.allow-jit": True})
    apple(XCRUN, "stapler", "validate", str(app))
    apple(SPCTL, "--assess", "--type", "execute", "--verbose=2", str(app))


def check_dmg(root, dmg, target, team, source):
    dmg = regular_file(dmg).resolve()
    signed(dmg, team)
    apple(HDIUTIL, "verify", str(dmg))
    apple(XCRUN, "stapler", "validate", str(dmg))
    apple(SPCTL, "--assess", "--type", "open", "--context", "context:primary-signature", "--verbose=2", str(dmg))
    # Inspect the app actually delivered inside the DMG, not merely the build-tree app.
    with tempfile.TemporaryDirectory(prefix="bluey-dmg-verify-") as temporary:
        mount = Path(temporary).resolve() / "mount"
        mount.mkdir()
        attached = False
        try:
            apple(HDIUTIL, "attach", "-readonly", "-nobrowse", "-noautoopen", "-mountpoint", str(mount), str(dmg))
            attached = True
            apps = list(mount.glob("*.app"))
            require(len(apps) == 1, "DMG must contain exactly one application")
            check_app(root, apps[0], target, team, source)
        finally:
            if attached:
                # Detach failure is a verification failure too; do not force unmount.
                apple(HDIUTIL, "detach", str(mount))


def notarize_dmg(dmg):
    # Tauri signs the DMG but its current bundler only notarizes/staples the app.
    # Capture the whole notary response privately and require explicit Accepted.
    for key in ("APPLE_ID", "APPLE_PASSWORD", "APPLE_TEAM_ID"):
        require(bool(os.environ.get(key, "").strip()), "Missing notarization credential: " + key)
    stdout, _ = apple(XCRUN, "notarytool", "submit", str(dmg),
                      "--apple-id", os.environ["APPLE_ID"], "--password", os.environ["APPLE_PASSWORD"],
                      "--team-id", os.environ["APPLE_TEAM_ID"], "--wait", "--timeout", "30m", "--output-format", "json")
    try:
        response = json.loads(stdout)
    except (ValueError, UnicodeDecodeError):
        raise ReleaseError("Unparseable notarization result") from None
    require(response.get("status") == "Accepted", "DMG notarization was not Accepted")
    apple(XCRUN, "stapler", "staple", str(dmg))


def safe_members(archive):
    """Members of the updater tarball, refused unless every one stays inside the extraction root."""
    members = []
    total = 0
    for member in archive:
        name = member.name
        parts = Path(name).parts
        require(name and not name.startswith("/") and ".." not in parts and not Path(name).is_absolute(),
                "Updater archive member escapes the extraction root")
        require(member.isfile() or member.isdir() or member.issym(), "Updater archive contains an unsupported member type")
        if member.issym():
            link = Path(member.linkname)
            require(not link.is_absolute() and ".." not in link.parts, "Updater archive symlink escapes the bundle")
        total += max(member.size, 0)
        members.append(member)
        require(len(members) <= MAX_ARCHIVE_MEMBERS and total <= MAX_ARCHIVE_BYTES, "Updater archive is implausibly large")
    return members


def check_archive(root, archive, signature, target, team, source):
    """The bundle users actually install through the updater: extract it and run the app checks on it."""
    read_signature(signature)
    archive = regular_file(archive)
    with tempfile.TemporaryDirectory(prefix="bluey-updater-verify-") as temporary:
        destination = Path(temporary).resolve() / "extract"
        destination.mkdir()
        try:
            with tarfile.open(archive, "r:gz") as tar:
                members = safe_members(tar)
                if hasattr(tarfile, "data_filter"):
                    tar.extractall(destination, members=members, filter="data")
                else:
                    tar.extractall(destination, members=members)
        except (tarfile.TarError, OSError, ValueError):
            raise ReleaseError("Updater archive is not a readable gzip tarball") from None
        apps = [p for p in destination.iterdir() if p.suffix == ".app"]
        require(len(apps) == 1 and {p.name for p in destination.iterdir()} == {apps[0].name},
                "Updater archive must contain exactly the application bundle")
        check_app(root, apps[0], target, team, source)


def verification(target):
    return {"appCodesign": True, "appGatekeeper": True, "appStaple": True,
            "dmgCodesign": True, "dmgGatekeeper": True, "dmgStaple": True,
            "mountedApp": True, "updaterApp": True, "architecture": ARCHES[target]}


def stage(root, target, expected, destination, team):
    require(sys.platform == "darwin", "Native verification requires macOS; Linux cannot verify Apple trust")
    require(re.fullmatch(r"[A-Z0-9]{10}", team) is not None, "Invalid release team ID")
    checked_tag(root, expected["tag"], expected["commit"])
    source = source_metadata(root, expected["tag"])
    app, dmg, archive, signature = discover_bundle(root, target)
    # No staging/eligible record exists until every expensive native check passes.
    check_app(root, app, target, team, source)
    notarize_dmg(dmg)
    check_dmg(root, dmg, target, team, source)
    check_archive(root, archive, signature, target, team, source)
    destination = Path(destination)
    require(not destination.exists(), "Refusing to reuse an existing verified artifact directory")
    destination.parent.mkdir(parents=True, exist_ok=True)
    with tempfile.TemporaryDirectory(prefix=".bluey-verified-", dir=destination.parent) as temporary:
        temporary = Path(temporary).resolve()
        copy = temporary / dmg.name
        shutil.copyfile(dmg, copy)
        entry = installer_entry(target, copy, source["minimumOsVersion"])
        require(entry == installer_entry(target, dmg, source["minimumOsVersion"]), "DMG changed during staging")
        validate_installer(entry)
        # Tauri names the archive `Bluey.app.tar.gz` for every target; the release needs one name per target.
        archive_copy = temporary / updater_name_for(dmg.name)
        signature_copy = temporary / (archive_copy.name + ".sig")
        shutil.copyfile(archive, archive_copy)
        shutil.copyfile(signature, signature_copy)
        updater = updater_entry(target, archive_copy, signature_copy)
        original = updater_entry(target, archive, signature)
        require({k: v for k, v in updater.items() if k != "asset"} == {k: v for k, v in original.items() if k != "asset"},
                "Updater bundle changed during staging")
        validate_updater(updater)
        write_json(temporary / RECORD, {"schemaVersion": 1, "provenance": expected,
                                      "version": source["version"], "installer": entry,
                                      "updater": updater, "verification": verification(target)})
        # Rename the complete directory in one step; failed builds never leave a marker.
        temporary.rename(destination)


def check_native_payload(root, directory, tag, team):
    require(sys.platform == "darwin", "Native verification requires macOS; Linux cannot verify Apple trust")
    require(re.fullmatch(r"[A-Z0-9]{10}", team) is not None, "Invalid release team ID")
    source = source_metadata(root, tag)
    data = validate_payload(root, directory, tag)
    for entry in data["installers"]:
        check_dmg(root, Path(directory) / entry["asset"], entry["target"], team, source)
    for entry in payload_updaters(directory, read_json(Path(directory) / UPDATER_FEED)):
        check_archive(root, Path(directory) / entry["asset"], Path(directory) / (entry["asset"] + ".sig"),
                      entry["target"], team, source)


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("command", choices=["stage", "payload"])
    parser.add_argument("--root", type=Path, required=True)
    parser.add_argument("--target", choices=list(ARCHES))
    parser.add_argument("--tag", required=True)
    parser.add_argument("--commit")
    parser.add_argument("--run-id")
    parser.add_argument("--run-attempt")
    parser.add_argument("--output", type=Path)
    parser.add_argument("--directory", type=Path)
    args = parser.parse_args()
    team = os.environ.get("APPLE_TEAM_ID", "")
    require(sys.platform == "darwin", "Native verification requires macOS; Linux cannot verify Apple trust")
    require(re.fullmatch(r"[A-Z0-9]{10}", team) is not None, "Invalid release team ID")
    if args.command == "stage":
        require(args.target and args.output and args.commit and args.run_id and args.run_attempt, "Missing staging context")
        expected = provenance(args.tag, args.commit, args.run_id, args.run_attempt)
        stage(args.root, args.target, expected, args.output, team)
        print("Native app, DMG and updater-bundle checks passed; current-attempt verified installer staged.")
    else:
        require(args.directory is not None, "--directory is required")
        check_native_payload(args.root, args.directory, args.tag, team)
        print("Downloaded DMGs, updater bundles and their embedded apps passed independent native re-verification.")


if __name__ == "__main__":
    try:
        main()
    except (ReleaseError, OSError, ValueError, KeyError, TypeError) as error:
        print("Native release verification failed: " + str(error), file=sys.stderr)
        sys.exit(1)
