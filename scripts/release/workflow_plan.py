#!/usr/bin/env python3
"""Resolve safe workflow inputs before any signing secrets or expensive builds are used."""

import os
from pathlib import Path
import sys

from publish_release import GitHub, no_existing_release
from release_metadata import (REPOSITORY, ReleaseError, checked_tag, git, require,
                              source_metadata, validate_commit, version_tag)


def main():
    root = Path(__file__).resolve().parents[2]
    event = os.environ.get("GITHUB_EVENT_NAME", "")
    require(event in {"push", "workflow_dispatch"}, "Unsupported release workflow event (no PR secret execution)")
    requested = os.environ.get("INPUT_PUBLISH", "false")
    require(requested in {"true", "false", ""}, "Invalid publish input")
    publish = event == "push" or requested == "true"
    tag = os.environ.get("INPUT_RELEASE_TAG", "")
    if event == "push":
        ref = os.environ.get("GITHUB_REF", "")
        require(ref.startswith("refs/tags/"), "Only tag pushes can publish")
        tag = ref[len("refs/tags/"):]
    if publish:
        require(os.environ.get("GITHUB_REPOSITORY") == REPOSITORY, "Releases may only publish from the canonical repository")
        version_tag(tag)
        # Environment protection evaluates the ORIGINAL workflow ref, not a later checkout.
        # Never allow dispatching protected tag A or a branch to publish different tag B.
        require(os.environ.get("GITHUB_REF") == "refs/tags/" + tag,
                "Publication must run from the exact requested version tag, never a branch or different tag")
        event_sha = validate_commit(os.environ.get("GITHUB_SHA", ""))
        event_commit = validate_commit(git(root, "rev-parse", event_sha + "^{commit}"))
        commit = checked_tag(root, tag, event_commit)
        require(commit == event_commit, "Release tag and triggering event commit differ")
        source_metadata(root, tag)
        # With contents:read, this rejects existing published releases. Draft visibility
        # requires push access; the write-authorized publisher repeats the authoritative check.
        no_existing_release(GitHub(), tag, commit)
    else:
        require(not tag, "Leave release_tag empty for a build-only dispatch")
        source_metadata(root)
        commit = validate_commit(git(root, "rev-parse", "HEAD"))
    with Path(os.environ["GITHUB_OUTPUT"]).open("a", encoding="utf-8") as stream:
        stream.write(f"publish={str(publish).lower()}\ntag={tag}\ncommit={commit}\n")
    print("Plan validated: " + ("publish existing version tag" if publish else "developer build only; not publishable"))


if __name__ == "__main__":
    try:
        main()
    except (ReleaseError, OSError, ValueError, KeyError, TypeError) as error:
        print("Release planning error: " + str(error), file=sys.stderr)
        sys.exit(1)
