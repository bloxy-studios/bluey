"""Bounded PATH mocks for shell ordering/frozen deps; no native build or Apple validation."""

import os
from pathlib import Path
import shutil
import subprocess
import sys
import unittest

sys.path.insert(0, str(Path(__file__).resolve().parents[1]))
from test_metadata import COMMIT, ReleaseFixture, SOURCE_ROOT


class ShellGateTests(ReleaseFixture):
    def setUp(self):
        super().setUp()
        scripts = self.root / "scripts"
        scripts.mkdir()
        shutil.copyfile(SOURCE_ROOT / "scripts/release.sh", scripts / "release.sh")
        shutil.copytree(SOURCE_ROOT / "scripts/release", scripts / "release", ignore=shutil.ignore_patterns("tests", "__pycache__"))
        self.bin = self.base / "bin"
        self.bin.mkdir()
        self.log = self.base / "commands.log"
        self.runner = self.base / "runner"
        self.runner.mkdir()
        self.home = self.base / "home"
        self.home.mkdir()
        self.executable(self.bin / "bun", '''#!/usr/bin/env bash
set -euo pipefail
if [[ "${1:-}" == --version ]]; then printf '%s\\n' "${MOCK_BUN_VERSION:-1.4.2}"; exit 0; fi
printf 'bun' >> "$MOCK_LOG"; printf ' [%s]' "$@" >> "$MOCK_LOG"; printf '\\n' >> "$MOCK_LOG"
if [[ "${1:-}" == install && "${MOCK_FAIL:-}" == install ]]; then exit 19; fi
if [[ "${1:-} ${2:-}" == 'run typecheck' && "${MOCK_FAIL:-}" == typecheck ]]; then exit 20; fi
if [[ "${1:-} ${2:-} ${3:-}" == 'run tauri build' && "${MOCK_FAIL:-}" == build ]]; then exit 21; fi
''')
        self.executable(self.bin / "uname", '#!/usr/bin/env bash\nprintf "Darwin\\n"\n')
        self.executable(self.bin / "git", '#!/usr/bin/env bash\ncase " $* " in *" rev-parse "*) printf "' + COMMIT + '\\n";; esac\n')
        self.executable(scripts / "check-rust.sh", '#!/usr/bin/env bash\nprintf "rust-check\\n" >> "$MOCK_LOG"\n')
        self.executable(scripts / "build-helper.sh", '''#!/usr/bin/env bash
set -euo pipefail
printf 'helper\\n' >> "$MOCK_LOG"
mkdir -p src-tauri/binaries
for binary in bluey-helper bluey-agent; do
  printf 'mock' > "src-tauri/binaries/$binary-$TARGET"
  chmod +x "src-tauri/binaries/$binary-$TARGET"
done
''')
        self.executable(scripts / "build-agent.sh", '''#!/usr/bin/env bash
set -euo pipefail
printf 'agent-%s\\n' "$BLUEY_AGENT_VARIANT" >> "$MOCK_LOG"
# Simulate the actual nested helper install (which lacks its own frozen flag).
bun install --os darwin --cpu '*'
''')
        self.env = {key: value for key, value in os.environ.items()
                    if not key.startswith(("APPLE_", "BLUEY_", "RELEASE_", "PUBLISH_", "TAURI_", "GITHUB_"))}
        self.env.update(PATH=str(self.bin) + os.pathsep + os.environ["PATH"], HOME=str(self.home),
                        TARGET="aarch64-apple-darwin", MOCK_LOG=str(self.log), RUNNER_TEMP=str(self.runner),
                        GITHUB_ACTIONS="true", GITHUB_REPOSITORY="bloxy-studios/bluey", GITHUB_RUN_ID="1234",
                        GITHUB_RUN_ATTEMPT="1", RELEASE_TAG=self.tag, RELEASE_COMMIT=COMMIT,
                        TAURI_SIGNING_PRIVATE_KEY="fixture-updater-private-key",
                        TAURI_SIGNING_PRIVATE_KEY_PASSWORD="fixture-updater-key-password")

    def executable(self, path, content):
        path.write_text(content)
        path.chmod(0o755)

    def run_shell(self, **environment):
        return subprocess.run(["bash", str(self.root / "scripts/release.sh")], cwd=self.root,
                              env={**self.env, **environment}, capture_output=True, text=True, timeout=20)

    def commands(self):
        return self.log.read_text() if self.log.exists() else ""

    def credentials(self):
        return {"APPLE_CERTIFICATE_P12": "ZmFrZS1jZXJ0", "APPLE_CERTIFICATE_PASSWORD": "fixture-cert-password",
                "APPLE_SIGNING_IDENTITY": "Developer ID Application: Fixture (ABCDEFGHIJ)",
                "APPLE_ID": "fixture@example.test", "APPLE_PASSWORD": "fixture-notary-password", "APPLE_TEAM_ID": "ABCDEFGHIJ"}

    def test_no_credentials_cannot_publish_or_begin_install(self):
        result = self.run_shell(PUBLISH_RELEASE="true")
        self.assertNotEqual(result.returncode, 0)
        self.assertIn("missing:", result.stderr)
        self.assertEqual(self.commands(), "")
        self.assertFalse((self.runner / "bluey-verified").exists())

    def test_build_only_keeps_claude_option_and_forces_nested_frozen_install(self):
        result = self.run_shell(RESEARCH_BACKEND="claude")
        self.assertEqual(result.returncode, 0, result.stderr)
        commands = self.commands().splitlines()
        self.assertIn("agent-full", commands)
        installs = [line for line in commands if line.startswith("bun [install]")]
        self.assertEqual(len(installs), 2)
        self.assertTrue(all("[--frozen-lockfile]" in line for line in installs))
        self.assertIn("[--os] [darwin] [--cpu] [*]", installs[-1])
        self.assertTrue(any("[--target] [aarch64-apple-darwin]" in line and "[--locked]" in line for line in commands))
        self.assertFalse(any("[--config]" in line for line in commands))
        self.assertFalse((self.runner / "bluey-verified").exists())
        self.assertIn("not a published release", result.stdout)

    def test_missing_updater_signing_key_fails_before_install(self):
        for environment in ({"TAURI_SIGNING_PRIVATE_KEY": ""},
                            {"TAURI_SIGNING_PRIVATE_KEY": "", "PUBLISH_RELEASE": "true", **self.credentials()}):
            with self.subTest(environment=environment):
                result = self.run_shell(**environment)
                self.assertNotEqual(result.returncode, 0)
                self.assertIn("TAURI_SIGNING_PRIVATE_KEY", result.stderr)
                self.assertEqual(self.commands(), "")
        result = self.run_shell(TAURI_SIGNING_PRIVATE_KEY="", TAURI_SIGNING_PRIVATE_KEY_PATH=str(self.base / "updater.key"))
        self.assertEqual(result.returncode, 0, result.stderr)

    def test_nightly_version_override_is_developer_only_and_reaches_tauri(self):
        result = self.run_shell(BLUEY_BUILD_VERSION="0.1.3-nightly.20260913")
        self.assertEqual(result.returncode, 0, result.stderr)
        builds = [line for line in self.commands().splitlines() if "[tauri] [build]" in line]
        self.assertEqual(len(builds), 1)
        self.assertIn('[--config] [{"version":"0.1.3-nightly.20260913"}]', builds[0])
        for environment in ({"BLUEY_BUILD_VERSION": "1.2.3"}, {"BLUEY_BUILD_VERSION": "0.1.3-nightly.20260913; touch pwned"},
                            {"BLUEY_BUILD_VERSION": '0.1.3-nightly.20260913","bundle":{"active":false'},
                            {"BLUEY_BUILD_VERSION": "0.1.3-nightly.20260913", "PUBLISH_RELEASE": "true", **self.credentials()}):
            with self.subTest(environment=environment):
                self.log.unlink(missing_ok=True)
                result = self.run_shell(**environment)
                self.assertNotEqual(result.returncode, 0)
                self.assertEqual(self.commands(), "")

    def test_failed_frozen_install_has_no_fallback(self):
        result = self.run_shell(MOCK_FAIL="install")
        self.assertNotEqual(result.returncode, 0)
        commands = self.commands()
        self.assertEqual(commands.count("bun [install]"), 1)
        self.assertNotIn("helper", commands)
        self.assertNotIn("[tauri]", commands)

    def test_failed_checks_never_build(self):
        result = self.run_shell(MOCK_FAIL="typecheck")
        self.assertNotEqual(result.returncode, 0)
        self.assertNotIn("[tauri]", self.commands())
        self.assertFalse((self.runner / "bluey-verified").exists())

    def test_failed_tauri_build_removes_stale_bundle_and_never_stages(self):
        stale = self.root / "src-tauri/target/aarch64-apple-darwin/release/bundle/dmg/old.dmg"
        stale.parent.mkdir(parents=True)
        stale.write_bytes(b"not a new build")
        result = self.run_shell(PUBLISH_RELEASE="true", MOCK_FAIL="build", **self.credentials())
        self.assertNotEqual(result.returncode, 0)
        self.assertIn("[tauri] [build]", self.commands())
        self.assertFalse(stale.exists())
        self.assertFalse((self.runner / "bluey-verified").exists())
        self.assertNotIn("fixture-cert-password", result.stdout + result.stderr)
        self.assertNotIn("fixture-notary-password", result.stdout + result.stderr)
        self.assertNotIn("fixture-updater-private-key", result.stdout + result.stderr)
        self.assertNotIn("fixture-updater-key-password", result.stdout + result.stderr)

    def test_existing_verified_directory_refused_before_build(self):
        stale = self.runner / "bluey-verified/mac-arm64"
        stale.mkdir(parents=True)
        result = self.run_shell(PUBLISH_RELEASE="true", **self.credentials())
        self.assertNotEqual(result.returncode, 0)
        self.assertIn("stale verified", result.stderr)
        self.assertEqual(self.commands(), "")

    def test_wrong_bun_or_unsupported_target_rejected(self):
        for environment in ({"MOCK_BUN_VERSION": "1.3.0"}, {"TARGET": "x86_64-unknown-linux-gnu"}, {"TARGET": "--help"}):
            with self.subTest(environment=environment):
                result = self.run_shell(**environment)
                self.assertNotEqual(result.returncode, 0)
                self.assertEqual(self.commands(), "")

    def test_nested_install_cannot_disable_frozen_flag(self):
        shim = SOURCE_ROOT / "scripts/release/bun-frozen.sh"
        for flag in ("--no-frozen-lockfile", "--frozen-lockfile=false"):
            result = subprocess.run(["bash", str(shim), "install", flag], capture_output=True, text=True,
                                    env={**self.env, "BLUEY_RELEASE_BUN": str(self.bin / "bun")})
            self.assertNotEqual(result.returncode, 0)
            self.assertEqual(self.commands(), "")

    def test_bash_syntax(self):
        for script in (SOURCE_ROOT / "scripts/release.sh", SOURCE_ROOT / "scripts/release/bun-frozen.sh"):
            result = subprocess.run(["bash", "-n", str(script)], capture_output=True, text=True)
            self.assertEqual(result.returncode, 0, result.stderr)


if __name__ == "__main__":
    unittest.main()
