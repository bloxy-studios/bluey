#!/usr/bin/env python3
"""Check release credentials without logging values; manage an ephemeral CI keychain."""

import argparse
import base64
import json
import os
from pathlib import Path
import re
import secrets
import shlex
import shutil
import subprocess
import sys

from release_metadata import ReleaseError, read_json, require, write_json

REQUIRED = ("APPLE_CERTIFICATE_P12", "APPLE_CERTIFICATE_PASSWORD", "APPLE_SIGNING_IDENTITY",
            "APPLE_ID", "APPLE_PASSWORD", "APPLE_TEAM_ID")
SECURITY = "/usr/bin/security"


def check_credentials(environ=None):
    environ = os.environ if environ is None else environ
    missing = [key for key in REQUIRED if not environ.get(key, "").strip()]
    require(not missing, "Publication requires all signing/notarization credentials; missing: " + ", ".join(missing))
    team = environ["APPLE_TEAM_ID"]
    require(re.fullmatch(r"[A-Z0-9]{10}", team) is not None, "Invalid APPLE_TEAM_ID")
    identity = environ["APPLE_SIGNING_IDENTITY"]
    require(identity.startswith("Developer ID Application: ") and identity.endswith("(" + team + ")")
            and "\n" not in identity and "\r" not in identity, "Expected matching Developer ID Application identity")
    try:
        certificate = base64.b64decode("".join(environ["APPLE_CERTIFICATE_P12"].split()), validate=True)
    except ValueError:
        raise ReleaseError("APPLE_CERTIFICATE_P12 is not valid base64") from None
    require(0 < len(certificate) <= 1024 * 1024, "Invalid signing certificate size")
    return certificate


def security(*args, must_succeed=True):
    result = subprocess.run([SECURITY, *args], stdout=subprocess.PIPE, stderr=subprocess.PIPE)
    # Never relay raw output/errors: import failures can echo supplied credentials.
    if must_succeed:
        require(result.returncode == 0, "Keychain operation failed: " + args[0])
    return result


def import_certificate(state):
    require(sys.platform == "darwin", "Signing keychain setup requires macOS")
    certificate = check_credentials()
    state = Path(state).resolve()
    state.mkdir(mode=0o700, parents=False, exist_ok=False)
    keychain = state / "signing.keychain-db"
    original = shlex.split(security("list-keychains", "-d", "user").stdout.decode("utf-8"))
    write_json(state / "state.json", {"originalKeychains": original})
    password = secrets.token_urlsafe(32)
    cert = state / "certificate.p12"
    try:
        security("create-keychain", "-p", password, str(keychain))
        security("set-keychain-settings", "-lut", "21600", str(keychain))
        security("unlock-keychain", "-p", password, str(keychain))
        cert.write_bytes(certificate)
        cert.chmod(0o600)
        security("import", str(cert), "-P", os.environ["APPLE_CERTIFICATE_PASSWORD"],
                 "-t", "cert", "-f", "pkcs12", "-k", str(keychain), "-T", "/usr/bin/codesign")
        security("set-key-partition-list", "-S", "apple-tool:,apple:,codesign:", "-s", "-k", password, str(keychain))
        security("list-keychains", "-d", "user", "-s", str(keychain), *original)
        identities = security("find-identity", "-v", "-p", "codesigning", str(keychain)).stdout.decode("utf-8")
        require('"' + os.environ["APPLE_SIGNING_IDENTITY"] + '"' in identities,
                "Imported keychain lacks the configured valid signing identity")
    except BaseException:
        cleanup(state)
        raise
    finally:
        cert.unlink(missing_ok=True)


def cleanup(state):
    state = Path(state).resolve()
    if not state.exists():
        return
    failed = False
    record = state / "state.json"
    if record.exists():
        original = read_json(record).get("originalKeychains")
        require(isinstance(original, list) and all(isinstance(item, str) for item in original), "Invalid keychain cleanup state")
        failed |= security("list-keychains", "-d", "user", "-s", *original, must_succeed=False).returncode != 0
    keychain = state / "signing.keychain-db"
    if keychain.exists():
        failed |= security("delete-keychain", str(keychain), must_succeed=False).returncode != 0
    shutil.rmtree(state)
    require(not failed, "Signing keychain cleanup failed; runner must not be reused")


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("command", choices=["check", "import", "cleanup"])
    parser.add_argument("--state", type=Path)
    args = parser.parse_args()
    if args.command == "check":
        check_credentials()
        print("All required signing/notarization inputs are present (values not logged).")
    else:
        require(args.state is not None, "--state is required")
        if args.command == "import":
            import_certificate(args.state)
            print("Temporary signing identity imported and checked.")
        else:
            cleanup(args.state)
            print("Temporary signing keychain cleanup complete.")


if __name__ == "__main__":
    try:
        main()
    except (ReleaseError, OSError, ValueError, KeyError, TypeError) as error:
        print("Release credentials error: " + str(error), file=sys.stderr)
        sys.exit(1)
