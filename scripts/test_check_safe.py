"""Unit tests for scripts/check_safe.py."""

from pathlib import Path
import re
import sys
import tempfile
import unittest

sys.path.insert(0, str(Path(__file__).resolve().parent))
from check_safe import check_safe


def check_fixture(source: str, relative: str = "native-dock/src/main.rs") -> list[str]:
    with tempfile.TemporaryDirectory(prefix="kinetix-safety-test-") as tmp:
        root = Path(tmp)
        (root / "native-dock/src").mkdir(parents=True, exist_ok=True)
        (root / "tools").mkdir(parents=True, exist_ok=True)
        for rel in [
            "tools/icon_extract.py",
            "native-dock/Cargo.toml",
            "run_native_dock.py",
            "run_native_clock.py",
        ]:
            (root / rel).write_text("", encoding="utf-8")
        target = root / relative
        target.parent.mkdir(parents=True, exist_ok=True)
        target.write_text(source, encoding="utf-8")
        return check_safe(root)


class CheckSafeTests(unittest.TestCase):
    def test_discovered_source_packages_receive_the_same_safety_checks(self) -> None:
        self.assertEqual(len(check_fixture("SendInput()", "widgets/sample/native/mod.rs")), 1)
        self.assertEqual(len(check_fixture("powershell.exe", "widgets/sample/native/mod.rs")), 1)
        self.assertEqual(len(check_fixture("NetUserEnum()", "widgets/sample/native/mod.rs")), 1)

    def test_native_source_is_accepted_when_clean(self) -> None:
        self.assertEqual(check_fixture('fn main() { println!("Local dock"); }'), [])

    def test_optional_portable_python_entry_points_receive_safety_checks(self) -> None:
        for rel in [
            "scripts/portable_python/launch_kinetix.py",
            "scripts/portable_python/Launch Kinetix.pyw",
        ]:
            self.assertEqual(len(check_fixture("SendInput()", rel)), 1)
            self.assertEqual(len(check_fixture("powershell.exe", rel)), 1)

    def test_native_source_guard_rejects_prohibited_apis(self) -> None:
        for api in [
            "SendInput",
            "GetAsyncKeyState",
            "PrintWindow",
            "NetUserEnum",
            "powershell.exe",
        ]:
            result = check_fixture(f'extern "system" {{ fn {api}(); }}')
            self.assertEqual(len(result), 1, api)
            self.assertRegex(result[0], re.compile(r"native-dock[\\/]src[\\/]main\.rs"))


if __name__ == "__main__":
    unittest.main()
