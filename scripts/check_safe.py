"""Safe-mode build guard.

Kinetix must not ship or spawn PowerShell helpers. This check runs before the
production build so a future widget cannot quietly reintroduce one.
"""

from __future__ import annotations

from pathlib import Path
import re
import sys

SOURCE_EXTENSIONS = {
    ".ts",
    ".tsx",
    ".js",
    ".mjs",
    ".cjs",
    ".cs",
    ".xaml",
    ".py",
    ".pyw",
    ".rs",
    ".toml",
}
RE_POWERSHELL = re.compile(r"powershell\.exe", re.IGNORECASE)
RE_EXEC_POLICY = re.compile(r"-ExecutionPolicy", re.IGNORECASE)
RE_CAPTURE_HOOK = re.compile(
    r"CopyFromScreen|PrintWindow|SetWindowsHookEx|GetAsyncKeyState", re.IGNORECASE
)
RE_SYNTHETIC_INPUT = re.compile(r"keybd_event|SendInput|mouse_event", re.IGNORECASE)
RE_USER_ENUM = re.compile(
    r"NetUserEnum|NetLocalGroupEnum|NetGroupEnum|Win32_UserAccount", re.IGNORECASE
)


def check_safe(root: Path | str | None = None) -> list[str]:
    root_path = Path(root) if root is not None else Path.cwd()
    problems: list[str] = []

    def inspect(file_path: Path) -> None:
        if not file_path.is_file():
            return
        rel = file_path.relative_to(root_path)
        rel_str = str(rel)
        if file_path.suffix.lower() == ".ps1":
            problems.append(f"{rel_str}: PowerShell script is not allowed in safe mode")
            return
        if file_path.suffix.lower() not in SOURCE_EXTENSIONS:
            return
        normalized = rel.as_posix()
        python_helper = normalized in {
            "tools/icon_extract.py",
            "tools/system_controls.py",
        }
        widget_package = normalized.startswith("widgets/")
        native_dock = (
            normalized.startswith("native-dock/")
            or (widget_package and "/native/" in normalized)
            or normalized
            in {
                "startup_windows.py",
                "run_native_dock.py",
                "run_native_clock.py",
                "run_kinetix.py",
                "run_kinetix.pyw",
                "Launch Kinetix.pyw",
            }
            or normalized.startswith("scripts/")
        )
        if (
            not normalized.startswith("src/")
            and not normalized.startswith("wpf/Kinetix/")
            and not python_helper
            and not native_dock
            and not widget_package
        ):
            return
        if "/bin/" in normalized or "/obj/" in normalized:
            return
        text = file_path.read_text(encoding="utf-8")
        if RE_POWERSHELL.search(text):
            problems.append(f"{rel_str}: launches powershell.exe")
        if RE_EXEC_POLICY.search(text):
            problems.append(f"{rel_str}: uses -ExecutionPolicy")
        if RE_CAPTURE_HOOK.search(text):
            problems.append(f"{rel_str}: uses a capture/input-hook API")
        if (
            normalized.startswith("src/")
            or normalized.startswith("resources/")
            or python_helper
            or native_dock
            or widget_package
        ) and RE_SYNTHETIC_INPUT.search(text):
            problems.append(f"{rel_str}: injects synthetic input")
        if native_dock and RE_USER_ENUM.search(text):
            problems.append(f"{rel_str}: enumerates local accounts or groups")

    def walk(directory: Path) -> None:
        if not directory.is_dir():
            return
        for entry in sorted(directory.iterdir()):
            if entry.is_dir():
                walk(entry)
            elif entry.is_file():
                inspect(entry)

    walk(root_path / "src" / "main")
    walk(root_path / "resources")
    walk(root_path / "wpf" / "Kinetix")
    inspect(root_path / "tools" / "icon_extract.py")
    inspect(root_path / "tools" / "system_controls.py")
    walk(root_path / "native-dock" / "src")
    walk(root_path / "widgets")
    inspect(root_path / "native-dock" / "build.rs")
    inspect(root_path / "native-dock" / "Cargo.toml")
    inspect(root_path / "run_native_dock.py")
    inspect(root_path / "run_native_clock.py")
    for name in ["run_kinetix.py", "run_kinetix.pyw", "Launch Kinetix.pyw", "startup_windows.py"]:
        inspect(root_path / name)
    for name in [
        "build-portable.py",
        "export-source.py",
        "verify-portable.py",
        "test-portable.py",
        "third_party.py",
    ]:
        inspect(root_path / "scripts" / name)
    walk(root_path / "scripts" / "portable_python")
    return problems


def main() -> int:
    problems = check_safe()
    if problems:
        print("Safe-mode check failed:", file=sys.stderr)
        for problem in problems:
            print(f"  - {problem}", file=sys.stderr)
        return 1
    print("Safe-mode check: no PowerShell helpers found.")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
