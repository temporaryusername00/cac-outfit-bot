"""Roblox Account Manager (v4) integration: launch accounts into CAC and find their windows.

Uses RAM's Local Web API (Settings -> Developer / WebServer). Needed there:
  - Enable the web server
  - Allow "LaunchAccount"
  - Multi Roblox turned on (Settings -> General)
"""
import os
import time

import requests

CAC_PLACE_ID = 7041939546


class RamError(Exception):
    pass


def _base(ram_cfg: dict) -> str:
    return f"http://127.0.0.1:{ram_cfg.get('port', 7963)}"


def _password(ram_cfg: dict) -> str:
    return os.environ.get("RAM_PASSWORD") or ram_cfg.get("password", "")


def check_running(ram_cfg: dict) -> None:
    try:
        requests.get(f"{_base(ram_cfg)}/v2/Running", timeout=5)
    except requests.RequestException:
        raise RamError(
            f"Can't reach Roblox Account Manager on port {ram_cfg.get('port', 7963)}. "
            "Open RAM and enable its web server (Settings -> Developer / WebServer)."
        )


def launch_account(ram_cfg: dict, account: str) -> None:
    params = {"Account": account, "PlaceId": ram_cfg.get("place_id", CAC_PLACE_ID)}
    password = _password(ram_cfg)
    if password:
        params["Password"] = password
    try:
        r = requests.get(f"{_base(ram_cfg)}/v2/LaunchAccount", params=params, timeout=60)
    except requests.RequestException as e:
        raise RamError(f"RAM request failed for {account}: {e}")
    if r.status_code != 200:
        raise RamError(f"RAM refused to launch {account}: {r.status_code} {r.text.strip()}")


def roblox_windows(title: str) -> list:
    import pygetwindow as gw

    return [w for w in gw.getWindowsWithTitle(title) if w.title.strip() == title]


def wait_for_new_window(title: str, known: set, timeout: float):
    """Wait for a Roblox window whose handle isn't in `known`."""
    end = time.monotonic() + timeout
    while time.monotonic() < end:
        for w in roblox_windows(title):
            if w._hWnd not in known:
                return w
        time.sleep(0.5)
    return None


def place_window(win, index: int, size: list | None) -> None:
    """Resize to the calibrated size and tile windows across the screen."""
    import pyautogui

    try:
        if win.isMinimized:
            win.restore()
        if size:
            win.resizeTo(*size)
        screen_w, screen_h = pyautogui.size()
        cols = max(1, screen_w // win.width)
        rows = max(1, screen_h // win.height)
        cell = index % (cols * rows)
        win.moveTo((cell % cols) * win.width, (cell // cols) * win.height)
    except Exception:
        pass  # placement is a nicety; the bot still works on overlapping windows
