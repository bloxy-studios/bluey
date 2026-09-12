#!/usr/bin/env python3
"""Draft-first GitHub Release publication. Never create/move tags or replace releases/assets."""

import argparse
import hashlib
import http.client
import json
import os
from pathlib import Path
import sys
import urllib.error
import urllib.parse
import urllib.request

from release_metadata import (CHECKSUMS, MANIFEST, REPOSITORY, UPDATER_FEED, ReleaseError, checked_tag,
                              file_facts, payload_upload_order, provenance, require, source_metadata,
                              validate_commit, validate_payload, version_tag)
from verify_macos import check_native_payload

API_HOST = "api.github.com"
UPLOAD_HOST = "uploads.github.com"
PREFIX = "/repos/" + REPOSITORY


class APIError(ReleaseError):
    def __init__(self, status):
        super().__init__("GitHub API request failed (HTTP " + str(status) + "); release was not finalized")
        self.status = status


class NoTokenRedirect(urllib.request.HTTPRedirectHandler):
    def redirect_request(self, req, fp, code, msg, headers, newurl):
        parsed = urllib.parse.urlparse(newurl)
        require(parsed.scheme == "https" and parsed.hostname in {"release-assets.githubusercontent.com", "objects.githubusercontent.com"},
                "Unexpected release asset redirect")
        redirected = super().redirect_request(req, fp, code, msg, headers, newurl)
        if redirected is not None:
            # Signed asset-storage URLs do not need (and must not receive) the GitHub token.
            redirected.remove_header("Authorization")
        return redirected


class GitHub:
    def __init__(self):
        token = os.environ.get("GH_TOKEN", "")
        require(bool(token), "GH_TOKEN is required for the release API")
        self.headers = {"Authorization": "Bearer " + token, "Accept": "application/vnd.github+json",
                        "X-GitHub-Api-Version": "2026-03-10", "User-Agent": "bluey-release"}

    def call(self, method, path, data=None):
        require(path.startswith(PREFIX + "/"), "API endpoint outside the canonical repository")
        body = None if data is None else json.dumps(data).encode("utf-8")
        request = urllib.request.Request("https://" + API_HOST + path, data=body,
                                         headers={**self.headers, "Content-Type": "application/json"}, method=method)
        try:
            # API requests must never silently redirect across repositories/hosts.
            opener = urllib.request.build_opener(NoTokenRedirect())
            with opener.open(request, timeout=60) as response:
                content = response.read(8 * 1024 * 1024 + 1)
            require(len(content) <= 8 * 1024 * 1024, "Oversized GitHub API response")
            if not content.strip():
                return None  # 204 No Content (asset deletion)
            return json.loads(content)
        except urllib.error.HTTPError as error:
            raise APIError(error.code) from None
        except (urllib.error.URLError, TimeoutError, ValueError):
            raise ReleaseError("GitHub API transport/JSON error; no automatic destructive retry") from None

    def remote_commit(self, tag):
        version_tag(tag)
        encoded = urllib.parse.quote(tag, safe="")
        try:
            reference = self.call("GET", PREFIX + "/git/ref/tags/" + encoded)
        except APIError as error:
            if error.status == 404:
                raise ReleaseError("The exact release tag must already exist remotely") from None
            raise
        require(reference.get("ref") == "refs/tags/" + tag, "Unexpected remote tag reference")
        obj = reference["object"]
        # Peel annotated tags (bounded); lightweight tags already point to a commit.
        for _ in range(8):
            if obj.get("type") == "commit":
                return validate_commit(obj.get("sha"))
            require(obj.get("type") == "tag", "Tag does not resolve to a commit")
            sha = validate_commit(obj.get("sha"))
            obj = self.call("GET", PREFIX + "/git/tags/" + sha)["object"]
        raise ReleaseError("Tag nesting exceeds release safety limit")

    def find_release(self, tag):
        matches = []
        # Drafts are included only with push access. The publisher has contents:write;
        # read-only preparation can reject visible published releases, not reliably drafts.
        for page in range(1, 101):
            rows = self.call("GET", PREFIX + f"/releases?per_page=100&page={page}")
            require(isinstance(rows, list), "Unexpected release listing response")
            matches.extend(row for row in rows if row.get("tag_name") == tag)
            if len(rows) < 100:
                require(len(matches) <= 1, "Ambiguous releases for tag")
                return matches[0] if matches else None
        raise ReleaseError("Release listing exceeds bounded pagination limit")

    def upload(self, release_id, path):
        path = Path(path)
        facts = file_facts(path)
        connection = http.client.HTTPSConnection(UPLOAD_HOST, timeout=120)
        endpoint = PREFIX + f"/releases/{release_id}/assets?" + urllib.parse.urlencode({"name": path.name})
        try:
            connection.putrequest("POST", endpoint)
            for key, value in {**self.headers, "Content-Type": "application/octet-stream", "Content-Length": str(facts["bytes"])}.items():
                connection.putheader(key, value)
            connection.endheaders()
            with path.open("rb") as stream:
                for block in iter(lambda: stream.read(1024 * 1024), b""):
                    connection.send(block)
            response = connection.getresponse()
            content = response.read(1024 * 1024 + 1)
            require(response.status == 201, "Release asset upload failed; draft left for owner review")
            require(len(content) <= 1024 * 1024, "Oversized upload response")
            return json.loads(content)
        except (OSError, http.client.HTTPException, ValueError):
            raise ReleaseError("Release upload transport/JSON error; draft left for owner review") from None
        finally:
            connection.close()

    def asset_facts(self, asset_id):
        # Verify the bytes GitHub stored, not only the size in an upload response.
        require(type(asset_id) is int and asset_id > 0, "Invalid GitHub asset ID")
        request = urllib.request.Request("https://" + API_HOST + PREFIX + f"/releases/assets/{asset_id}",
                                         headers={**self.headers, "Accept": "application/octet-stream"})
        digest, size = hashlib.sha256(), 0
        try:
            with urllib.request.build_opener(NoTokenRedirect()).open(request, timeout=120) as response:
                for block in iter(lambda: response.read(1024 * 1024), b""):
                    size += len(block)
                    digest.update(block)
        except (OSError, urllib.error.URLError):
            raise ReleaseError("Cannot read back uploaded asset; draft left for owner review") from None
        return {"bytes": size, "sha256": digest.hexdigest()}


def no_existing_release(api, tag, commit):
    require(api.remote_commit(tag) == commit, "Remote release tag moved or differs from checked-out source")
    existing = api.find_release(tag)
    require(existing is None, "A release already exists for this tag (draft or published). Never overwrite; inspect it manually.")


def context(root, tag, expected_commit=None):
    source = source_metadata(root, tag)
    commit = checked_tag(root, tag, expected_commit)
    return source, commit


def preflight(root, tag, output=None, check_api=True):
    source, commit = context(root, tag)
    if check_api:
        no_existing_release(GitHub(), tag, commit)
    if output:
        # All values have strict single-line grammars before entering Actions outputs.
        with Path(output).open("a", encoding="utf-8") as stream:
            stream.write(f"version={source['version']}\ntag={tag}\ncommit={commit}\n")
    return commit


def publish(api, root, directory, tag, commit, run_id, run_attempt):
    context(root, tag, commit)
    payload = validate_payload(root, directory, tag)
    # DMGs, updater archives + signatures, SHA256SUMS, the updater feed and the site manifest (last).
    files = payload_upload_order(directory, payload)
    verified_facts = {path.name: file_facts(path) for path in files}
    for entry in payload["installers"]:
        require(verified_facts[entry["asset"]] == {"bytes": entry["bytes"], "sha256": entry["sha256"]},
                "Payload bytes do not match manifest")
    expected = provenance(tag, commit, run_id, run_attempt)
    no_existing_release(api, tag, commit)
    # An invocation of this publisher cannot bypass native gates by providing a
    # hand-written manifest/marker. The publisher itself rechecks downloaded DMGs.
    check_native_payload(root, directory, tag, os.environ.get("APPLE_TEAM_ID", ""))
    is_prerelease = version_tag(tag)[1]
    marker = f"bluey-release run={expected['runId']} attempt={expected['runAttempt']} commit={commit}"
    body = (f"Bluey {payload['version']} for macOS. Both DMGs are Developer ID signed and notarized.\n\n"
            f"Installer metadata: `{MANIFEST}`. SHA-256 checksums: `{CHECKSUMS}`. "
            f"In-app updater feed: `{UPDATER_FEED}` with the signed `.app.tar.gz` bundles (docs/UPDATES.md).\n\n"
            f"<!-- {marker} -->")
    draft = api.call("POST", PREFIX + "/releases", {"tag_name": tag, "target_commitish": commit,
                       "name": "Bluey " + payload["version"], "body": body,
                       "draft": True, "prerelease": is_prerelease, "make_latest": "false"})
    release_id = draft.get("id")
    require(type(release_id) is int and release_id > 0, "Invalid draft release ID")
    require(draft.get("draft") is True and draft.get("tag_name") == tag and draft.get("body") == body,
            "Unexpected created draft; refusing to upload")
    uploaded = {}
    # Deliberately no --clobber, DELETE, resume, automatic upload retry or tag creation.
    # Failure/timeout anywhere below leaves this draft unpublished for human inspection.
    for path in files:
        facts = file_facts(path)
        require(facts == verified_facts[path.name], "Local payload changed after native verification; draft left unpublished")
        asset = api.upload(release_id, path)
        require(asset.get("name") == path.name and asset.get("state") == "uploaded" and asset.get("size") == facts["bytes"],
                "Incomplete release asset upload")
        require(api.asset_facts(asset.get("id")) == facts, "Stored GitHub asset differs from verified payload")
        uploaded[path.name] = (asset.get("id"), facts["bytes"])
    current = api.call("GET", PREFIX + f"/releases/{release_id}")
    require(current.get("draft") is True and current.get("tag_name") == tag and current.get("body") == body
            and current.get("prerelease") is is_prerelease, "Draft changed during upload; refusing publication")
    assets = current.get("assets", [])
    require(len(assets) == len(uploaded), "Unexpected or missing draft assets")
    seen = set()
    for asset in assets:
        name = asset.get("name")
        require(name in uploaded and name not in seen and asset.get("state") == "uploaded"
                and (asset.get("id"), asset.get("size")) == uploaded[name], "Draft assets changed before publication")
        seen.add(name)
    require(api.remote_commit(tag) == commit, "Remote tag moved during upload; refusing publication")
    validate_payload(root, directory, tag)
    # Stable releases participate in GitHub's version/date latest selection; a
    # prerelease never displaces stable. Do not make an older backfill latest by fiat.
    final = api.call("PATCH", PREFIX + f"/releases/{release_id}",
                     {"draft": False, "prerelease": is_prerelease, "make_latest": "false" if is_prerelease else "legacy"})
    require(final.get("draft") is False and final.get("tag_name") == tag and final.get("prerelease") is is_prerelease,
            "Finalization response uncertain; inspect the release before rerunning")
    return release_id


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("command", choices=["preflight", "publish"])
    parser.add_argument("--root", type=Path, required=True)
    parser.add_argument("--tag", required=True)
    parser.add_argument("--commit")
    parser.add_argument("--output")
    parser.add_argument("--directory", type=Path)
    parser.add_argument("--run-id")
    parser.add_argument("--run-attempt")
    args = parser.parse_args()
    require(os.environ.get("GITHUB_REPOSITORY") == REPOSITORY, "Publishing API is restricted to the canonical repository")
    if args.command == "preflight":
        preflight(args.root, args.tag, args.output)
        print("Existing tag and source versions checked; no existing release for this tag.")
    else:
        require(os.environ.get("GITHUB_ACTIONS") == "true", "Publication CLI is restricted to the gated Actions workflow")
        require(args.directory is not None and args.commit and args.run_id and args.run_attempt, "Missing publishing context")
        release_id = publish(GitHub(), args.root, args.directory, args.tag, args.commit, args.run_id, args.run_attempt)
        print("Published complete release ID " + str(release_id) + ".")


if __name__ == "__main__":
    try:
        main()
    except (ReleaseError, OSError, ValueError, KeyError, TypeError) as error:
        print("Release publication error: " + str(error), file=sys.stderr)
        sys.exit(1)
