"""Shared helpers: config loading and Roblox window lookup."""
import json
import sys
from pathlib import Path

# The scripts live in python/; the data is shared with the app in data/ and logs/.
ROOT = Path(__file__).resolve().parent.parent
DATA = ROOT / "data"
LOGS = ROOT / "logs"
CONFIG_PATH = DATA / "config.json"


def load_json(path: Path, default=None):
    if not path.exists():
        if default is not None:
            return default
        sys.exit(f"Missing file: {path.name}")
    # utf-8-sig: Notepad and PowerShell often save JSON with a BOM.
    with path.open(encoding="utf-8-sig") as f:
        return json.load(f)


def save_json(path: Path, data) -> None:
    path.parent.mkdir(parents=True, exist_ok=True)
    with path.open("w", encoding="utf-8") as f:
        json.dump(data, f, indent=2)


def load_config() -> dict:
    return load_json(CONFIG_PATH)


def size_key(width: int, height: int) -> str:
    return f"{width}x{height}"


def click_points_in_flow(flow: dict) -> list[str]:
    """Every point name used by a 'click' step, in first-use order."""
    names: list[str] = []
    for section in ("start", "per_item", "finish"):
        for step in flow.get(section, []):
            name = step.get("click")
            if name and name not in names:
                names.append(name)
    return names


def find_window(title: str):
    """Return the Roblox window, or exit with a helpful message."""
    import pygetwindow as gw

    matches = [w for w in gw.getWindowsWithTitle(title) if w.title.strip() == title]
    if not matches:
        matches = gw.getWindowsWithTitle(title)
    if not matches:
        sys.exit(f'No window titled "{title}" found. Open Catalog Avatar Creator first.')
    return matches[0]


def focus_window(win) -> None:
    try:
        if win.isMinimized:
            win.restore()
        win.activate()
    except Exception:
        # activate() can raise even when it worked; clicking inside fixes focus anyway.
        pass
