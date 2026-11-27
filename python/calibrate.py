"""Record where each CAC button sits, relative to the Roblox window.

CAC's layout changes with the window size, so points are saved per size
(config.json -> "points" -> "1920x1080", "960x600", ...). Calibrate once for
every window size you run the bot at.

Usage:
    python calibrate.py              # record every point the flow clicks
    python calibrate.py first_result # re-record only the named point(s)

For each point: open the screen in CAC where that button shows, hover the
mouse over it and press F8. Press Esc to stop.
"""
import sys

import keyboard
import pyautogui

from common import CONFIG_PATH, click_points_in_flow, find_window, focus_window, load_config, save_json, size_key


def main() -> None:
    config = load_config()
    names = sys.argv[1:] or click_points_in_flow(config["flow"])
    win = find_window(config["window_title"])
    focus_window(win)
    key = size_key(win.width, win.height)
    points = config.setdefault("points", {}).setdefault(key, {})

    print(f"Calibrating {len(names)} point(s) for window size {key}. Keep the Roblox window where it is.\n")
    for name in names:
        hint = config.get("point_hints", {}).get(name, "")
        print(f"-> {name}{'  (' + hint + ')' if hint else ''}")
        print("   Hover over it and press F8 (Esc to stop).")
        event = keyboard.read_hotkey(suppress=False)
        while event not in ("f8", "esc"):
            event = keyboard.read_hotkey(suppress=False)
        if event == "esc":
            print("Stopped.")
            break
        x, y = pyautogui.position()
        rel = [x - win.left, y - win.top]
        points[name] = rel
        print(f"   saved {name} = {rel}\n")
        save_json(CONFIG_PATH, config)

    print(f"Done. Points are in {CONFIG_PATH.name} under \"{key}\".")


if __name__ == "__main__":
    main()
