"""ensure-sidecars.sh rebuilds a sidecar that is missing or older than its sources (TEST-009)."""

import os
from pathlib import Path
import shutil
import subprocess
import tempfile
import unittest

SOURCE_ROOT = Path(__file__).resolve().parents[3]

# Stand-in for build-helper.sh / build-agent.sh: log, then install the binary and its stamp.
FAKE_BUILD = '''#!/usr/bin/env bash
set -euo pipefail
ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
printf '%s\\n' "KIND" >> "$MOCK_LOG"
out="$ROOT/src-tauri/binaries/bluey-KIND-aarch64-apple-darwin"
mkdir -p "$(dirname "$out")"
printf 'bin' > "$out"
chmod +x "$out"
bash "$ROOT/scripts/sidecar-stamp.sh" KIND > "$out.stamp"
'''


class EnsureSidecarsTests(unittest.TestCase):
    def setUp(self):
        self.base = Path(tempfile.mkdtemp())
        self.addCleanup(shutil.rmtree, self.base)
        self.root = self.base / "repo"
        scripts = self.root / "scripts"
        scripts.mkdir(parents=True)
        for name in ("ensure-sidecars.sh", "sidecar-stamp.sh"):
            shutil.copy(SOURCE_ROOT / "scripts" / name, scripts / name)
        (scripts / "build-helper.sh").write_text(FAKE_BUILD.replace("KIND", "helper"))
        (scripts / "build-agent.sh").write_text(FAKE_BUILD.replace("KIND", "agent"))
        helper = self.root / "src-tauri/swift/BlueyHelper"
        (helper / "Sources/BlueyHelper").mkdir(parents=True)
        (helper / "Package.swift").write_text("// package\n")
        (helper / "bluey-helper.entitlements").write_text("<plist/>\n")
        (helper / "Sources/BlueyHelper/main.swift").write_text("print(1)\n")
        agent = self.root / "sidecars/agent"
        (agent / "src").mkdir(parents=True)
        (agent / "package.json").write_text("{}\n")
        (agent / "bun.lock").write_text("{}\n")
        (agent / "src/index.ts").write_text("export {};\n")
        bin_dir = self.base / "bin"
        bin_dir.mkdir()
        (bin_dir / "uname").write_text('#!/usr/bin/env bash\nprintf "arm64\\n"\n')
        (bin_dir / "uname").chmod(0o755)
        self.log = self.base / "builds.log"
        self.env = {**os.environ, "PATH": str(bin_dir) + os.pathsep + os.environ["PATH"],
                    "MOCK_LOG": str(self.log)}

    def ensure(self):
        """Run ensure-sidecars.sh; return which sidecars it built."""
        self.log.write_text("")
        result = subprocess.run(["bash", str(self.root / "scripts/ensure-sidecars.sh")], cwd=self.root,
                                env=self.env, capture_output=True, text=True, timeout=30)
        self.assertEqual(result.returncode, 0, result.stderr)
        return self.log.read_text().split()

    def test_builds_missing_then_skips_up_to_date_sidecars(self):
        self.assertEqual(self.ensure(), ["helper", "agent"])
        self.assertEqual(self.ensure(), [])

    def test_rebuilds_only_the_sidecar_whose_sources_changed(self):
        self.ensure()
        (self.root / "src-tauri/swift/BlueyHelper/Sources/BlueyHelper/main.swift").write_text("print(2)\n")
        self.assertEqual(self.ensure(), ["helper"])
        (self.root / "sidecars/agent/src/new.ts").write_text("export const x = 1;\n")
        self.assertEqual(self.ensure(), ["agent"])

    def test_rebuilds_a_binary_built_before_stamps_existed(self):
        self.ensure()
        (self.root / "src-tauri/binaries/bluey-agent-aarch64-apple-darwin.stamp").unlink()
        self.assertEqual(self.ensure(), ["agent"])


if __name__ == "__main__":
    unittest.main()
