#!/usr/bin/env python3
"""The Nightly channel: version planning and the rolling `nightly` prerelease (docs/UPDATES.md).

`plan` decides whether tonight's build is needed (main moved since the last nightly, or forced)
and computes the nightly version; `publish` turns the two developer build artifacts into the
rolling prerelease's assets and updater feed. Unlike version releases, the `nightly` tag moves
and its assets are replaced — that is the point of the channel — but a versioned tag or release
is never touched here, and the feed is uploaded last so it only ever points at assets that exist.
"""

import argparse
import datetime
import os
from pathlib import Path
import re
import shutil
import sys
import tempfile

from publish_release import PREFIX, APIError, GitHub
from release_metadata import (CHECKSUMS, SEMVER, TARGETS, UPDATER_FEED, UPDATER_SUFFIX,
                              ReleaseError, file_facts, installer_entry, now_rfc3339, regular_file,
                              require, safe_name, source_metadata, updater_entry, updater_feed,
                              updater_name_for, validate_commit, validate_feed, version_tag, write_json)

NIGHTLY_TAG = "nightly"
NIGHTLY_VERSION = re.compile(r"(0|[1-9][0-9]*)\.(0|[1-9][0-9]*)\.(0|[1-9][0-9]*)-nightly\.(20[0-9]{6})\Z", re.ASCII)
MARKER = re.compile(r"<!-- bluey-nightly commit=([0-9a-f]{40}) version=([0-9A-Za-z.+-]{1,64}) -->")


def nightly_version(source_version, date):
    """`X.Y.(Z+1)-nightly.YYYYMMDD`: above the sources' version, below the next stable release."""
    version, _ = version_tag(source_version)
    match = SEMVER.fullmatch(version)
    require(re.fullmatch(r"20[0-9]{6}", date) is not None, "Nightly date must be YYYYMMDD")
    return f"{match.group(1)}.{match.group(2)}.{int(match.group(3)) + 1}-nightly.{date}"


def check_version(version):
    require(isinstance(version, str) and NIGHTLY_VERSION.fullmatch(version) is not None,
            "Not a nightly version (X.Y.Z-nightly.YYYYMMDD)")
    assert version_tag(version)[1], "nightly versions are prereleases"
    return version


def last_nightly(release):
    """`(commit, version)` recorded in the rolling release's body, or `(None, None)`."""
    if not release:
        return None, None
    match = MARKER.search(release.get("body") or "")
    return (match.group(1), match.group(2)) if match else (None, None)


def plan(root, commit, api, force, today=None):
    commit = validate_commit(commit)
    date = (today or datetime.datetime.now(datetime.timezone.utc)).strftime("%Y%m%d")
    version = check_version(nightly_version(source_metadata(root)["version"], date))
    previous_commit, _ = last_nightly(api.find_release(NIGHTLY_TAG))
    build = bool(force) or previous_commit != commit
    return {"build": build, "version": version, "commit": commit}


def collect(incoming, output, version, source):
    """Rename Tauri's per-target outputs into release asset names and write SHA256SUMS + the feed."""
    incoming, output = Path(incoming), Path(output)
    require(incoming.is_dir() and {p.name for p in incoming.iterdir()} == set(TARGETS), "Expected exactly both nightly artifacts")
    output.mkdir(parents=True, exist_ok=False)
    installers, updaters = [], []
    for target in TARGETS:
        directory = incoming / target
        require(directory.is_dir() and not directory.is_symlink(), f"{target}: invalid artifact directory")
        # upload-artifact keeps Tauri's `dmg/` and `macos/` layout below the artifact root.
        dmgs = [p for p in directory.rglob("*.dmg") if not p.is_symlink()]
        archives = [p for p in directory.rglob("*" + UPDATER_SUFFIX) if not p.is_symlink()]
        signatures = [p for p in directory.rglob("*" + UPDATER_SUFFIX + ".sig") if not p.is_symlink()]
        require(len(dmgs) == 1 and len(archives) == 1 and len(signatures) == 1, f"{target}: expected one DMG, one archive and one signature")
        require(signatures[0].name == archives[0].name + ".sig", f"{target}: signature does not belong to the archive")
        dmg = output / safe_name(dmgs[0].name, ".dmg")
        require(version in dmg.name, f"{target}: DMG name does not carry the nightly version")
        archive = output / updater_name_for(dmg.name)
        signature = output / (archive.name + ".sig")
        for src, dst in ((dmgs[0], dmg), (archives[0], archive), (signatures[0], signature)):
            shutil.copyfile(regular_file(src), dst)
        installers.append(installer_entry(target, dmg, source["minimumOsVersion"]))
        updaters.append(updater_entry(target, archive, signature))
    with (output / CHECKSUMS).open("x", encoding="utf-8") as stream:
        stream.write("".join(f"{e['sha256']}  {e['asset']}\n" for e in installers + updaters))
    notes = f"Bluey nightly {version} — built from main; unsigned developer build (see the release notes)."
    feed = updater_feed(version, NIGHTLY_TAG, updaters, now_rfc3339(), notes)
    write_json(output / UPDATER_FEED, feed)
    validate_feed(feed, version, NIGHTLY_TAG, updaters)
    ordered = [output / e["asset"] for e in installers] + [output / e["asset"] for e in updaters]
    ordered += [output / (e["asset"] + ".sig") for e in updaters] + [output / CHECKSUMS, output / UPDATER_FEED]
    return ordered


def body_for(version, commit, run_id):
    return (f"## Bluey nightly {version}\n\n"
            f"Built from `main` @ `{commit[:12]}` by run {run_id}. **Unsigned developer build** — not notarized; "
            "macOS asks you to confirm the first launch (right-click → Open). This release is rebuilt every night "
            "`main` changed and is what the **Nightly** update channel installs; it can break. Stable builds are "
            "the versioned releases.\n\n"
            f"Updater feed: `{UPDATER_FEED}`. Checksums: `{CHECKSUMS}`.\n\n"
            f"<!-- bluey-nightly commit={commit} version={version} -->")


def publish(api, root, incoming, version, commit, run_id, run_attempt, output=None):
    check_version(version)
    commit = validate_commit(commit)
    require(re.fullmatch(r"[1-9][0-9]*", str(run_id)) and re.fullmatch(r"[1-9][0-9]*", str(run_attempt)), "Invalid run context")
    source = source_metadata(root)
    with tempfile.TemporaryDirectory(prefix="bluey-nightly-") as temporary:
        staging = Path(output) if output else Path(temporary) / "assets"
        files = collect(incoming, staging, version, source)
        names = {p.name for p in files}
        release = api.find_release(NIGHTLY_TAG)
        body = body_for(version, commit, run_id)
        if release is None:
            # Creating the release creates the lightweight `nightly` tag at `commit`.
            release = api.call("POST", PREFIX + "/releases", {"tag_name": NIGHTLY_TAG, "target_commitish": commit,
                               "name": f"Bluey nightly {version}", "body": body, "draft": False,
                               "prerelease": True, "make_latest": "false"})
        else:
            require(release.get("prerelease") is True and release.get("draft") is False, "The nightly release must stay a published prerelease")
            api.call("PATCH", PREFIX + "/git/refs/tags/" + NIGHTLY_TAG, {"sha": commit, "force": True})
        release_id = release.get("id")
        require(type(release_id) is int and release_id > 0, "Invalid nightly release ID")
        existing = {asset.get("name"): asset for asset in release.get("assets", []) if isinstance(asset, dict)}
        # Same-day reruns and the two rolling files are replaced by deleting first; the feed is uploaded last.
        for name in list(names & set(existing)):
            api.call("DELETE", PREFIX + f"/releases/assets/{existing[name]['id']}")
            del existing[name]
        for path in files:
            facts = file_facts(path)
            asset = api.upload(release_id, path)
            require(asset.get("name") == path.name and asset.get("state") == "uploaded" and asset.get("size") == facts["bytes"],
                    "Incomplete nightly asset upload")
            require(api.asset_facts(asset.get("id")) == facts, "Stored nightly asset differs from the built file")
        # Previous nightlies' assets go once the new set is complete and the feed points at it.
        for name, asset in existing.items():
            api.call("DELETE", PREFIX + f"/releases/assets/{asset['id']}")
        final = api.call("PATCH", PREFIX + f"/releases/{release_id}",
                         {"name": f"Bluey nightly {version}", "body": body, "prerelease": True, "draft": False,
                          "make_latest": "false"})
        require(final.get("tag_name") == NIGHTLY_TAG and final.get("prerelease") is True, "Nightly release finalization uncertain")
        return release_id


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    sub = parser.add_subparsers(dest="command", required=True)
    check = sub.add_parser("check-version")
    check.add_argument("version")
    planner = sub.add_parser("plan")
    planner.add_argument("--root", type=Path, required=True)
    publisher = sub.add_parser("publish")
    publisher.add_argument("--root", type=Path, required=True)
    publisher.add_argument("--incoming", type=Path, required=True)
    publisher.add_argument("--version", required=True)
    publisher.add_argument("--commit", required=True)
    publisher.add_argument("--run-id", required=True)
    publisher.add_argument("--run-attempt", required=True)
    args = parser.parse_args()
    if args.command == "check-version":
        check_version(args.version)
        print(args.version)
        return
    require(os.environ.get("GITHUB_REPOSITORY") == "bloxy-studios/bluey", "Nightly publishing is restricted to the canonical repository")
    if args.command == "plan":
        require(os.environ.get("GITHUB_REF") == "refs/heads/main", "Nightly builds come only from main")
        force = os.environ.get("INPUT_FORCE", "false") == "true"
        result = plan(args.root, os.environ.get("GITHUB_SHA", ""), GitHub(), force)
        with Path(os.environ["GITHUB_OUTPUT"]).open("a", encoding="utf-8") as stream:
            stream.write(f"build={str(result['build']).lower()}\nversion={result['version']}\ncommit={result['commit']}\n")
        print(("Nightly build needed: " if result["build"] else "main unchanged since the last nightly; skipping: ") + result["version"])
    else:
        require(os.environ.get("GITHUB_ACTIONS") == "true", "Nightly publication runs only inside the Actions workflow")
        release_id = publish(GitHub(), args.root, args.incoming, args.version, args.commit, args.run_id, args.run_attempt)
        print("Nightly release " + str(release_id) + " updated to " + args.version + ".")


if __name__ == "__main__":
    try:
        main()
    except (ReleaseError, APIError, OSError, ValueError, KeyError, TypeError) as error:
        print("Nightly release error: " + str(error), file=sys.stderr)
        sys.exit(1)
