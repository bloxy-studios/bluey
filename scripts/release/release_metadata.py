#!/usr/bin/env python3
"""Fail-closed, stdlib-only metadata boundary for Bluey's macOS releases (Python 3.9+)."""

import argparse
import datetime
import hashlib
import json
import os
from pathlib import Path
import re
import shutil
import stat
import subprocess
import sys
import urllib.parse

REPOSITORY = "bloxy-studios/bluey"
MANIFEST = "bluey-downloads.json"
CHECKSUMS = "SHA256SUMS"
RECORD = "verified.json"
# Tauri's static updater manifest (docs/UPDATES.md): the app reads it from the release.
UPDATER_FEED = "latest.json"
UPDATER_SUFFIX = ".app.tar.gz"
SIGNATURE_SUFFIX = ".app.tar.gz.sig"
MAX_JSON = 64 * 1024
MAX_SIGNATURE = 8 * 1024
TARGETS = {"mac-arm64": "aarch64-apple-darwin", "mac-x64": "x86_64-apple-darwin"}
ARCHES = {"mac-arm64": "arm64", "mac-x64": "x86_64"}
# `platforms` keys of the updater feed (`<os>-<arch>` as tauri-plugin-updater reports them).
PLATFORMS = {"mac-arm64": "darwin-aarch64", "mac-x64": "darwin-x86_64"}
SEMVER = re.compile(
    r"(0|[1-9][0-9]*)\.(0|[1-9][0-9]*)\.(0|[1-9][0-9]*)"
    r"(?:-([0-9A-Za-z-]+(?:\.[0-9A-Za-z-]+)*))?"
    r"(?:\+([0-9A-Za-z-]+(?:\.[0-9A-Za-z-]+)*))?\Z", re.ASCII
)
OS_VERSION = re.compile(r"[0-9]+(?:\.[0-9]+){0,2}\Z", re.ASCII)
SAFE_NAME = re.compile(r"[A-Za-z0-9][A-Za-z0-9._+ -]{0,199}\Z", re.ASCII)
SHA256 = re.compile(r"[0-9a-f]{64}\Z", re.ASCII)
COMMIT = re.compile(r"[0-9a-f]{40}\Z", re.ASCII)
# The `.sig` Tauri writes next to the updater archive: base64 text (the minisign signature).
SIGNATURE = re.compile(r"[A-Za-z0-9+/=\s]{64,8192}\Z", re.ASCII)
RFC3339 = re.compile(r"[0-9]{4}-[0-9]{2}-[0-9]{2}T[0-9]{2}:[0-9]{2}:[0-9]{2}(?:\.[0-9]+)?(?:Z|[+-][0-9]{2}:[0-9]{2})\Z", re.ASCII)
# A release the feed may point at: a version tag (`v1.2.3`, `v1.2.3-rc.1+build.2`) or the rolling `nightly` prerelease.
RELEASE_REF = re.compile(r"[A-Za-z0-9][A-Za-z0-9._+-]{0,127}\Z", re.ASCII)


class ReleaseError(ValueError):
    pass


def require(condition, message):
    if not condition:
        raise ReleaseError(message)


def version_tag(tag):
    require(isinstance(tag, str) and len(tag) <= 128, "Invalid version tag")
    version = tag[1:] if tag.startswith("v") else tag
    match = SEMVER.fullmatch(version)
    require(match is not None, "Tag must be a SemVer version, with optional leading v")
    prerelease = match.group(4)
    if prerelease:
        require(all(not (part.isdigit() and len(part) > 1 and part[0] == "0")
                    for part in prerelease.split(".")), "Numeric prerelease identifiers cannot have leading zeroes")
    return version, bool(prerelease)


def safe_name(value, suffix=None):
    require(isinstance(value, str) and SAFE_NAME.fullmatch(value) is not None,
            "Unsafe asset or bundle basename")
    require(value == value.strip() and ".." not in value and not value.endswith("."),
            "Unsafe asset or bundle basename")
    if suffix:
        require(value.endswith(suffix), "Unexpected asset or bundle extension")
    return value


def regular_file(path):
    path = Path(path)
    require(stat.S_ISREG(path.lstat().st_mode), "Expected a regular, non-symlink file")
    # Reject symlinked parent directories too, not just the final component.
    require(all(not parent.is_symlink() for parent in path.parents), "Symlinked artifact parent")
    return path


def file_facts(path):
    path = regular_file(path)
    digest = hashlib.sha256()
    size = 0
    with path.open("rb") as stream:
        for block in iter(lambda: stream.read(1024 * 1024), b""):
            digest.update(block)
            size += len(block)
    require(0 < size <= 2**53 - 1, "Installer must have a positive, safely representable byte size")
    return {"bytes": size, "sha256": digest.hexdigest()}


def no_duplicate_keys(pairs):
    result = {}
    for key, value in pairs:
        require(key not in result, "Duplicate JSON object key")
        result[key] = value
    return result


def read_json(path):
    path = regular_file(path)
    require(path.stat().st_size <= MAX_JSON, "Metadata exceeds 64 KiB")
    return json.loads(path.read_text(encoding="utf-8"), object_pairs_hook=no_duplicate_keys)


def write_json(path, data):
    content = json.dumps(data, indent=2, ensure_ascii=True) + "\n"
    require(len(content.encode("utf-8")) <= MAX_JSON, "Metadata exceeds 64 KiB")
    # Exclusive creation avoids silently reusing or overwriting earlier verification.
    with Path(path).open("x", encoding="utf-8") as stream:
        stream.write(content)


def cargo_versions(path):
    """Read only literal package/workspace versions; reject noncanonical/ambiguous TOML.

    This intentionally is NOT a TOML implementation. The source uses single-line
    sections and literal versions. New multiline TOML or alternate version table
    syntax needs explicit review rather than weakening release validation on Python 3.9.
    """
    text = regular_file(path).read_text(encoding="utf-8")
    require('"""' not in text and "'''" not in text, "Multiline Cargo TOML needs release-reader review")
    values, seen_sections = {}, set()
    section = ""
    for line in text.splitlines():
        stripped = line.strip()
        if not stripped or stripped.startswith("#"):
            continue
        if stripped.startswith("["):
            match = re.fullmatch(r"\[([^\[\]]+)\]\s*(?:#.*)?", stripped)
            require(match is not None, "Noncanonical Cargo section needs release-reader review")
            section = match.group(1)
            require(section not in seen_sections, "Duplicate Cargo section")
            seen_sections.add(section)
            require(section not in {"package.version", "workspace.package.version"},
                    "Use a literal Cargo version or version.workspace = true")
        elif section in {"package", "workspace.package"}:
            key = stripped.split("=", 1)[0].strip().strip('"\'')
            if key not in {"version", "version.workspace"}:
                continue
            full_key = section + "." + key
            require(full_key not in values, "Duplicate Cargo version")
            match = re.fullmatch(r"version\s*=\s*([\"'])([^\"']+)\1\s*(?:#.*)?", stripped)
            if match:
                values[full_key] = match.group(2)
            elif section == "package" and re.fullmatch(r"version\.workspace\s*=\s*true\s*(?:#.*)?", stripped):
                values[full_key] = True
            else:
                raise ReleaseError("Noncanonical Cargo version needs release-reader review")
    require(not ("package.version" in values and "package.version.workspace" in values), "Conflicting Cargo versions")
    if values.get("package.version.workspace") is True:
        require("workspace.package.version" in values, "Inherited Cargo workspace version missing")
        values["package.version"] = values["workspace.package.version"]
    require("package.version" in values, "Cargo package version missing")
    return {key: value for key, value in values.items() if key != "package.version.workspace"}


def source_metadata(root, tag=None):
    root = Path(root)
    package = read_json(root / "package.json")
    config = read_json(root / "src-tauri/tauri.conf.json")
    values = {"package.json": package.get("version"), "tauri.conf.json": config.get("version")}
    values.update({"Cargo.toml:" + key: value for key, value in cargo_versions(root / "src-tauri/Cargo.toml").items()})
    require(all(isinstance(value, str) for value in values.values()), "Missing literal source version")
    version = package["version"]
    require(version_tag(version)[0] == version, "Source version cannot include leading v")
    require(all(value == version for value in values.values()), "Source manifest versions do not match")
    if tag is not None:
        require(version_tag(tag)[0] == version, "Release tag does not match all source versions")
    macos = config.get("bundle", {}).get("macOS", {})
    minimum = macos.get("minimumSystemVersion")
    require(isinstance(minimum, str) and OS_VERSION.fullmatch(minimum) is not None, "Missing/invalid macOS minimumSystemVersion")
    require(macos.get("hardenedRuntime") is True, "Hardened runtime must stay enabled")
    require(macos.get("entitlements") == "entitlements.plist", "Review changed app entitlements path before publishing")
    require(macos.get("skipStapling", False) is False, "App stapling must not be disabled")
    require(config.get("bundle", {}).get("targets") == ["app", "dmg"], "Expected app + dmg bundle targets")
    require(config.get("bundle", {}).get("externalBin") == ["binaries/bluey-helper", "binaries/bluey-agent"],
            "Review changed bundled sidecars before publishing")
    # Automatic config overlays can change version, signing, output paths or deployment floor.
    overlays = list((root / "src-tauri").glob("tauri.macos.conf.*"))
    require(not overlays, "macOS config overlays require release audit")
    require(not os.environ.get("TAURI_CONFIG"), "TAURI_CONFIG override is not allowed for releases")
    product = safe_name(config.get("productName"))
    identifier = config.get("identifier")
    require(isinstance(identifier, str) and re.fullmatch(r"[A-Za-z0-9.-]+", identifier) is not None,
            "Invalid application identifier")
    return {"version": version, "minimumOsVersion": minimum, "productName": product,
            "identifier": identifier, "sourceVersions": values}


def git(root, *args):
    process = subprocess.run(["git", "-C", str(root), *args], capture_output=True, text=True)
    require(process.returncode == 0, "Git release reference validation failed")
    return process.stdout.strip()


def validate_commit(value):
    require(isinstance(value, str) and COMMIT.fullmatch(value) is not None, "Expected full commit SHA")
    return value


def checked_tag(root, tag, expected_commit=None):
    version_tag(tag)  # Restrict input before it reaches git/URLs/Actions outputs.
    head = validate_commit(git(root, "rev-parse", "HEAD"))
    commit = validate_commit(git(root, "rev-parse", "--verify", "refs/tags/" + tag + "^{commit}"))
    require(commit == head, "Existing tag must resolve to checked-out HEAD")
    if expected_commit:
        require(commit == validate_commit(expected_commit), "Tag moved since workflow preflight")
    return commit


def provenance(tag, commit, run_id, run_attempt):
    version_tag(tag)
    validate_commit(commit)
    require(re.fullmatch(r"[1-9][0-9]*", str(run_id)) is not None, "Invalid workflow run ID")
    require(re.fullmatch(r"[1-9][0-9]*", str(run_attempt)) is not None, "Invalid workflow run attempt")
    return {"repository": REPOSITORY, "tag": tag, "commit": commit,
            "runId": str(run_id), "runAttempt": str(run_attempt)}


def installer_entry(target, path, minimum):
    require(target in TARGETS, "Unsupported macOS target")
    require(isinstance(minimum, str) and OS_VERSION.fullmatch(minimum) is not None, "Invalid minimum OS version")
    return {"target": target, "format": "dmg", "asset": safe_name(Path(path).name, ".dmg"),
            **file_facts(path), "minimumOsVersion": minimum}


def now_rfc3339():
    return datetime.datetime.now(datetime.timezone.utc).replace(microsecond=0).isoformat().replace("+00:00", "Z")


def read_signature(path):
    """The updater archive's detached signature as Tauri wrote it: base64 text, never binary."""
    path = regular_file(path)
    require(path.stat().st_size <= MAX_SIGNATURE, "Oversized updater signature")
    try:
        text = path.read_text(encoding="ascii").strip()
    except (UnicodeDecodeError, ValueError):
        raise ReleaseError("Updater signature is not ASCII text") from None
    require(SIGNATURE.fullmatch(text) is not None, "Updater signature is not base64 text")
    return text


def updater_entry(target, archive, signature):
    """The updater bundle Tauri built for `target` (`<name>.app.tar.gz`) and its `<name>.app.tar.gz.sig`."""
    require(target in TARGETS, "Unsupported macOS target")
    archive, signature = Path(archive), Path(signature)
    name = safe_name(archive.name, UPDATER_SUFFIX)
    require(signature.name == name + ".sig", "Signature file must sit next to its archive as <archive>.sig")
    return {"target": target, "platform": PLATFORMS[target], "asset": name, **file_facts(archive),
            "signature": read_signature(signature)}


def validate_updater(entry):
    require(isinstance(entry, dict) and set(entry) == {"target", "platform", "asset", "bytes", "sha256", "signature"},
            "Unexpected updater fields")
    require(entry["target"] in TARGETS and entry["platform"] == PLATFORMS[entry["target"]], "Unsupported updater target/platform")
    safe_name(entry["asset"], UPDATER_SUFFIX)
    require(type(entry["bytes"]) is int and 0 < entry["bytes"] <= 2**53 - 1, "Invalid updater byte size")
    require(isinstance(entry["sha256"], str) and SHA256.fullmatch(entry["sha256"]) is not None, "Invalid SHA256")
    require(isinstance(entry["signature"], str) and SIGNATURE.fullmatch(entry["signature"]) is not None
            and entry["signature"] == entry["signature"].strip(), "Invalid updater signature")


def asset_url(release_ref, name):
    require(isinstance(release_ref, str) and RELEASE_REF.fullmatch(release_ref) is not None, "Unsafe release reference")
    return (f"https://github.com/{REPOSITORY}/releases/download/"
            f"{urllib.parse.quote(release_ref, safe='')}/{urllib.parse.quote(safe_name(name), safe='')}")


def updater_feed(version, release_ref, entries, published_at, notes=None):
    """Tauri's static updater manifest (`latest.json`): one `platforms` entry per macOS target."""
    version_tag(version)
    require(isinstance(published_at, str) and RFC3339.fullmatch(published_at) is not None, "Feed pub_date must be RFC 3339")
    platforms = {}
    for entry in entries:
        validate_updater(entry)
        require(entry["platform"] not in platforms, "Duplicated updater platform")
        platforms[entry["platform"]] = {"signature": entry["signature"], "url": asset_url(release_ref, entry["asset"])}
    require(set(platforms) == set(PLATFORMS.values()), "Both macOS updater bundles are required")
    if notes is None:
        notes = f"Bluey {version} — https://github.com/{REPOSITORY}/releases/tag/{urllib.parse.quote(release_ref, safe='')}"
    require(isinstance(notes, str) and 0 < len(notes) <= 4000 and "\x00" not in notes, "Invalid feed notes")
    return {"version": version, "notes": notes, "pub_date": published_at, "platforms": platforms}


def validate_feed(data, version, release_ref, entries):
    require(isinstance(data, dict) and set(data) == {"version", "notes", "pub_date", "platforms"}, "Unexpected feed fields")
    expected = updater_feed(version, release_ref, entries, data.get("pub_date") if isinstance(data.get("pub_date"), str) else "",
                            data.get("notes") if isinstance(data.get("notes"), str) else None)
    require(data == expected, "Updater feed does not match the verified bundles")
    return data


def feed_assets(data):
    """Asset basenames the feed points at, in `PLATFORMS` order."""
    require(isinstance(data, dict) and isinstance(data.get("platforms"), dict), "Invalid updater feed")
    names = []
    for platform in PLATFORMS.values():
        entry = data["platforms"].get(platform)
        require(isinstance(entry, dict) and isinstance(entry.get("url"), str), "Feed platform missing")
        prefix = f"https://github.com/{REPOSITORY}/releases/download/"
        require(entry["url"].startswith(prefix), "Feed URL outside the canonical repository")
        parts = entry["url"][len(prefix):].split("/")
        require(len(parts) == 2, "Unexpected feed URL shape")
        names.append(safe_name(urllib.parse.unquote(parts[1]), UPDATER_SUFFIX))
    return names


def validate_installer(entry):
    require(isinstance(entry, dict) and set(entry) == {"target", "format", "asset", "bytes", "sha256", "minimumOsVersion"},
            "Unexpected installer fields")
    require(entry["target"] in TARGETS and entry["format"] == "dmg", "Unsupported target/format")
    safe_name(entry["asset"], ".dmg")
    require(type(entry["bytes"]) is int and 0 < entry["bytes"] <= 2**53 - 1, "Invalid installer byte size")
    require(isinstance(entry["sha256"], str) and SHA256.fullmatch(entry["sha256"]) is not None, "Invalid SHA256")
    minimum = entry["minimumOsVersion"]
    require(isinstance(minimum, str) and OS_VERSION.fullmatch(minimum) is not None, "Invalid minimum OS version")


def validate_manifest(data):
    require(isinstance(data, dict) and set(data) == {"schemaVersion", "version", "tag", "repository", "installers"},
            "Unexpected manifest fields")
    require(type(data["schemaVersion"]) is int and data["schemaVersion"] == 1, "Unsupported manifest schema")
    require(data["repository"] == REPOSITORY, "Wrong manifest repository")
    require(version_tag(data["tag"])[0] == data["version"], "Manifest tag/version mismatch")
    entries = data["installers"]
    require(isinstance(entries, list) and len(entries) == len(TARGETS), "Both macOS installers are required")
    for entry in entries:
        validate_installer(entry)
    require({entry["target"] for entry in entries} == set(TARGETS), "Missing or duplicated macOS target")
    require(len({entry["asset"].casefold() for entry in entries}) == len(entries), "Duplicated asset basename")
    require(len({entry["minimumOsVersion"] for entry in entries}) == 1, "Minimum OS versions differ")
    return data


def discover_bundle(root, target):
    """The one app, DMG, updater archive and signature Tauri built for `target`."""
    require(target in TARGETS, "Unsupported macOS target")
    base = Path(root) / "src-tauri/target" / TARGETS[target] / "release/bundle"
    apps, dmgs = list((base / "macos").glob("*.app")), list((base / "dmg").glob("*.dmg"))
    require(len(apps) == 1 and len(dmgs) == 1, "Expected exactly one built app and one DMG for target")
    require(apps[0].is_dir() and not apps[0].is_symlink(), "Invalid app bundle")
    safe_name(apps[0].name, ".app")
    regular_file(dmgs[0])
    safe_name(dmgs[0].name, ".dmg")
    archives = [p for p in (base / "macos").glob("*" + UPDATER_SUFFIX) if p.name.endswith(UPDATER_SUFFIX)]
    signatures = list((base / "macos").glob("*" + SIGNATURE_SUFFIX))
    require(len(archives) == 1 and len(signatures) == 1,
            "Expected exactly one updater archive and one signature for target "
            "(bundle.createUpdaterArtifacts with TAURI_SIGNING_PRIVATE_KEY set)")
    regular_file(archives[0])
    safe_name(archives[0].name, UPDATER_SUFFIX)
    require(signatures[0].name == archives[0].name + ".sig", "Updater signature does not belong to the archive")
    read_signature(signatures[0])
    return apps[0].resolve(), dmgs[0].resolve(), archives[0].resolve(), signatures[0].resolve()


def updater_name_for(dmg_name):
    """The release name of a target's updater bundle: the DMG's stem (`Bluey_1.2.3_aarch64`) + `.app.tar.gz`."""
    stem = safe_name(dmg_name, ".dmg")[: -len(".dmg")]
    return safe_name(stem + UPDATER_SUFFIX, UPDATER_SUFFIX)


def validate_record(data, expected, target, source, dmg, archive=None, signature=None):
    require(isinstance(data, dict) and set(data) == {"schemaVersion", "provenance", "version", "installer", "updater", "verification"},
            "Unexpected verification record fields")
    require(type(data["schemaVersion"]) is int and data["schemaVersion"] == 1, "Wrong verification schema")
    require(data["provenance"] == expected, "Artifact provenance/run attempt mismatch")
    require(data["version"] == source["version"], "Artifact version mismatch")
    required_checks = {"appCodesign": True, "appGatekeeper": True, "appStaple": True,
                       "dmgCodesign": True, "dmgGatekeeper": True, "dmgStaple": True,
                       "mountedApp": True, "updaterApp": True, "architecture": ARCHES[target]}
    checks = data["verification"]
    require(isinstance(checks, dict) and checks == required_checks
            and all(type(checks[key]) is type(value) for key, value in required_checks.items()),
            "Artifact has not passed every native check")
    validate_installer(data["installer"])
    actual = installer_entry(target, dmg, source["minimumOsVersion"])
    require(data["installer"] == actual, "Installer differs from verified bytes/metadata")
    validate_updater(data["updater"])
    require(data["updater"]["target"] == target, "Updater bundle belongs to another target")
    if archive is not None:
        require(data["updater"] == updater_entry(target, archive, signature), "Updater bundle differs from verified bytes/metadata")
    return actual, data["updater"]


def assemble(root, incoming, output, expected, published_at=None):
    source = source_metadata(root, expected["tag"])
    incoming, output = Path(incoming), Path(output)
    require(incoming.is_dir() and not incoming.is_symlink(), "Missing artifact directory")
    require({p.name for p in incoming.iterdir()} == set(TARGETS), "Expected exactly both current-attempt matrix artifacts")
    entries, updaters, files = [], [], []
    for target in TARGETS:
        directory = incoming / target
        require(directory.is_dir() and not directory.is_symlink(), "Invalid target artifact directory")
        data = read_json(directory / RECORD)
        validate_installer(data.get("installer"))
        validate_updater(data.get("updater"))
        dmg = directory / data["installer"]["asset"]
        archive = directory / data["updater"]["asset"]
        signature = directory / (archive.name + ".sig")
        require({p.name for p in directory.iterdir()} == {RECORD, dmg.name, archive.name, signature.name},
                "Unexpected artifact contents")
        installer, updater = validate_record(data, expected, target, source, dmg, archive, signature)
        entries.append(installer)
        updaters.append(updater)
        files.extend([dmg, archive, signature])
    manifest = validate_manifest({"schemaVersion": 1, "version": source["version"],
                                  "tag": expected["tag"], "repository": REPOSITORY, "installers": entries})
    feed = updater_feed(source["version"], expected["tag"], updaters, published_at or now_rfc3339())
    # All inputs pass before an output directory or manifest can become eligible.
    output.mkdir(parents=True, exist_ok=False)
    try:
        for path in files:
            shutil.copyfile(path, output / path.name)
        write_json(output / MANIFEST, manifest)
        write_json(output / UPDATER_FEED, feed)
        content = "".join(f"{entry['sha256']}  {entry['asset']}\n" for entry in entries)
        content += "".join(f"{entry['sha256']}  {entry['asset']}\n" for entry in updaters)
        with (output / CHECKSUMS).open("x", encoding="utf-8") as stream:
            stream.write(content)
        validate_payload(root, output, expected["tag"])
    except BaseException:
        shutil.rmtree(output)
        raise
    return manifest


def payload_updaters(directory, feed):
    """Re-derive the updater entries from the payload files the feed points at."""
    directory = Path(directory)
    entries = []
    for target, name in zip(PLATFORMS, feed_assets(feed)):
        entries.append(updater_entry(target, directory / name, directory / (name + ".sig")))
    return entries


def validate_payload(root, directory, tag):
    source = source_metadata(root, tag)
    directory = Path(directory)
    data = validate_manifest(read_json(directory / MANIFEST))
    require(data["tag"] == tag and data["version"] == source["version"], "Payload version/tag mismatch")
    expected_names = {MANIFEST, CHECKSUMS, UPDATER_FEED}
    for entry in data["installers"]:
        actual = installer_entry(entry["target"], directory / entry["asset"], source["minimumOsVersion"])
        require(actual == entry, "Payload bytes do not match manifest")
        expected_names.add(entry["asset"])
    feed = read_json(directory / UPDATER_FEED)
    updaters = payload_updaters(directory, feed)
    validate_feed(feed, source["version"], tag, updaters)
    for entry in updaters:
        expected_names.update({entry["asset"], entry["asset"] + ".sig"})
    require({p.name for p in directory.iterdir()} == expected_names, "Unexpected release payload files")
    checksum_file = regular_file(directory / CHECKSUMS)
    require(checksum_file.stat().st_size <= MAX_JSON, "Oversized checksums file")
    expected_sums = "".join(f"{entry['sha256']}  {entry['asset']}\n" for entry in data["installers"])
    expected_sums += "".join(f"{entry['sha256']}  {entry['asset']}\n" for entry in updaters)
    require(checksum_file.read_text(encoding="utf-8") == expected_sums, "SHA256SUMS does not match manifest")
    return data


def payload_upload_order(directory, data):
    """Release upload order: DMGs, updater archives, signatures, SHA256SUMS, the updater feed, then the site manifest last."""
    directory = Path(directory)
    feed = read_json(directory / UPDATER_FEED)
    archives = feed_assets(feed)
    files = [directory / entry["asset"] for entry in data["installers"]]
    files += [directory / name for name in archives]
    files += [directory / (name + ".sig") for name in archives]
    files += [directory / CHECKSUMS, directory / UPDATER_FEED, directory / MANIFEST]
    return files


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    sub = parser.add_subparsers(dest="command", required=True)
    version = sub.add_parser("version")
    version.add_argument("--root", type=Path, required=True)
    version.add_argument("--tag")
    version.add_argument("--check-tag", action="store_true")
    version.add_argument("--commit")
    build = sub.add_parser("assemble")
    for p in (build,):
        p.add_argument("--root", type=Path, required=True)
        p.add_argument("--tag", required=True)
        p.add_argument("--commit", required=True)
        p.add_argument("--run-id", required=True)
        p.add_argument("--run-attempt", required=True)
    build.add_argument("--incoming", type=Path, required=True)
    build.add_argument("--output", type=Path, required=True)
    args = parser.parse_args()
    if args.command == "version":
        data = source_metadata(args.root, args.tag)
        if args.check_tag:
            require(args.tag is not None, "--check-tag requires --tag")
            data["commit"] = checked_tag(args.root, args.tag, args.commit)
        print(json.dumps(data, indent=2))
    else:
        expected = provenance(args.tag, args.commit, args.run_id, args.run_attempt)
        assemble(args.root, args.incoming, args.output, expected)
        print("Validated complete macOS installer pair; manifest, updater feed and SHA256SUMS written.")


if __name__ == "__main__":
    try:
        main()
    except (ReleaseError, OSError, ValueError, KeyError, TypeError) as error:
        print("Release metadata error: " + str(error), file=sys.stderr)
        sys.exit(1)
