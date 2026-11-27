"""CAC Outfit Bot: builds each outfit in Catalog Avatar Creator and publishes it.

It drives real Roblox windows with mouse and keyboard, following the steps in
config.json ("flow"). Nothing is injected into Roblox.

With several Roblox windows (one per account), the bot interleaves them: while
one window waits for a search, an equip or a publish to load, it works in
another. Only one window can have the mouse at a time, so they take turns.

Usage:
    python bot.py                 # single window (whatever Roblox window is open)
    python bot.py --attach        # use every open Roblox window
    python bot.py --launch        # launch config "accounts" via Roblox Account Manager, then run
    python bot.py --dry-run       # print the steps without touching the mouse
    python bot.py --check-items   # look up item IDs first and skip bad ones
    python bot.py --limit 3       # at most 3 outfits per window this run

Stop at any time: press End, or slam the mouse into a screen corner.
"""
import argparse
import heapq
import sys
import threading
import time
from datetime import datetime

from common import DATA, LOGS, ROOT, find_window, focus_window, load_config, load_json, save_json, size_key

OUTFITS_PATH = DATA / "outfits.json"
PUBLISHED_PATH = DATA / "published.json"
LOG_PATH = LOGS / "bot.log"

stop_event = threading.Event()
log_lock = threading.Lock()


class Stopped(Exception):
    pass


def log(msg: str) -> None:
    line = f"[{datetime.now():%H:%M:%S}] {msg}"
    with log_lock:
        print(line)
        LOG_PATH.parent.mkdir(parents=True, exist_ok=True)
        with LOG_PATH.open("a", encoding="utf-8") as f:
            f.write(line + "\n")


def sleep(seconds: float) -> None:
    """Sleep in small slices so End stops the bot quickly."""
    end = time.monotonic() + seconds
    while time.monotonic() < end:
        if stop_event.is_set():
            raise Stopped
        time.sleep(min(0.05, max(0.0, end - time.monotonic())))


def roblox_click(x: int, y: int) -> None:
    """Click that Roblox actually registers.

    Roblox ignores a click when the cursor was only teleported there (SetCursorPos,
    which is what pyautogui does): its buttons need real mouse-move events first.
    So: put the cursor there, wiggle it with relative moves, then press and release
    with a short hold.
    """
    import ctypes

    user32 = ctypes.windll.user32
    MOVE, DOWN, UP = 0x0001, 0x0002, 0x0004
    user32.SetCursorPos(int(x), int(y))
    time.sleep(0.04)
    for _ in range(2):
        user32.mouse_event(MOVE, 1, 0, 0, 0)
        time.sleep(0.02)
        user32.mouse_event(MOVE, -1, 0, 0, 0)
        time.sleep(0.02)
    user32.mouse_event(DOWN, 0, 0, 0, 0)
    time.sleep(0.06)
    user32.mouse_event(UP, 0, 0, 0, 0)


VK = {
    "enter": 0x0D, "backspace": 0x08, "tab": 0x09, "esc": 0x1B, "space": 0x20,
    "ctrl": 0x11, "shift": 0x10, "alt": 0x12, "delete": 0x2E,
}


def roblox_key(combo: str) -> None:
    """Press a key combo the way Roblox registers it: with hardware scan codes and a short hold.

    pyautogui's key presses carry no scan code and are released instantly; Roblox
    often drops those (Enter in particular).
    """
    import ctypes

    user32 = ctypes.windll.user32
    codes = []
    for k in combo.lower().split("+"):
        vk = VK.get(k) or (ord(k.upper()) if len(k) == 1 else None)
        if vk is None:
            raise ValueError(f"Unknown key: {k}")
        codes.append((vk, user32.MapVirtualKeyW(vk, 0)))
    for vk, sc in codes:
        user32.keybd_event(vk, sc, 0, 0)
        time.sleep(0.03)
    time.sleep(0.06)
    for vk, sc in reversed(codes):
        user32.keybd_event(vk, sc, 2, 0)
        time.sleep(0.03)


class StepFailed(Exception):
    pass


class Slot:
    """One Roblox window (one account)."""

    def __init__(self, label: str, win, points: dict, states: dict, ready_at: float = 0.0):
        self.label = label
        self.win = win
        self.points = points
        self.states = states
        self.ready_at = ready_at
        self.count = 0
        self.failed = 0


class Bot:
    def __init__(self, config: dict, dry_run: bool, test: bool, queue: list, limit: int, published: dict):
        self.config = config
        self.dry_run = dry_run
        self.test = test
        self.delays = config.get("delays", {})
        self.queue = queue
        self.limit = limit
        self.published = published
        self.gap = float(config.get("seconds_between_outfits", 30))
        self.focused = None
        if not dry_run:
            import pyautogui

            pyautogui.FAILSAFE = True
            pyautogui.PAUSE = 0
            self.gui = pyautogui

    # ---------- reading the screen ----------

    def screen_is(self, slot: Slot, state: str) -> bool:
        """True if every pixel listed for this state matches (within tolerance)."""
        from PIL import ImageGrab

        spec = slot.states.get(state)
        if not spec:
            raise SystemExit(f'No screen check "{state}" in config.json -> "states" for this window size.')
        tol = spec.get("tol", 30)
        w = slot.win
        img = ImageGrab.grab(bbox=(w.left, w.top, w.left + w.width, w.top + w.height), all_screens=True).convert("RGB")
        for x, y, rgb in spec["pixels"]:
            px = img.getpixel((x, y))
            if any(abs(a - b) > tol for a, b in zip(px, rgb)):
                return False
        return True

    def wait_for(self, slot: Slot, state: str, timeout: float, negate: bool = False) -> bool:
        end = time.monotonic() + timeout
        while True:
            if self.screen_is(slot, state) != negate:
                return True
            if time.monotonic() >= end:
                return False
            sleep(0.25)

    def detect(self, slot: Slot) -> str | None:
        for state in self.config.get("recovery", {}).get("order", []):
            if state in slot.states and self.screen_is(slot, state):
                return state
        return None

    def recover(self, slot: Slot) -> bool:
        """Close whatever is open until CAC is back on its main screen."""
        rec = self.config.get("recovery", {})
        closers = rec.get("close", {})
        for _ in range(8):
            state = self.detect(slot)
            if state == "main":
                return True
            if state in closers:
                log(f"[{slot.label}]   recovering: closing {state}")
                x, y = slot.points[closers[state]]
                roblox_click(slot.win.left + x, slot.win.top + y)
            sleep(1.2)
        return False

    def snapshot(self, slot: Slot, why: str) -> None:
        try:
            from PIL import ImageGrab

            folder = LOGS / "failures"
            folder.mkdir(exist_ok=True)
            w = slot.win
            path = folder / f"{datetime.now():%Y%m%d-%H%M%S}-{slot.label}-{why}.png"
            ImageGrab.grab(bbox=(w.left, w.top, w.left + w.width, w.top + w.height), all_screens=True).save(path)
            log(f"[{slot.label}]   screenshot saved: logs/failures/{path.name}")
        except Exception:
            pass

    # ---------- low-level actions (run while the slot's window has focus) ----------

    def focus(self, slot: Slot) -> None:
        if self.dry_run or self.focused is slot:
            return
        focus_window(slot.win)
        try:
            import pygetwindow as gw

            active = gw.getActiveWindow()
            if active is None or active._hWnd != slot.win._hWnd:
                # Windows blocks focus steals unless a key was just pressed.
                roblox_key("alt")
                focus_window(slot.win)
        except Exception:
            pass
        self.focused = slot
        sleep(self.delays.get("after_focus", 0.3))

    def click(self, slot: Slot, name: str) -> None:
        if name not in slot.points:
            sys.exit(f'Point "{name}" is not calibrated. Run: python calibrate.py {name}')
        rx, ry = slot.points[name]
        roblox_click(slot.win.left + rx, slot.win.top + ry)

    def do(self, slot: Slot, step: dict, fields: dict) -> None:
        tag = f"    [{slot.label}]"
        if "click" in step:
            name = step["click"]
            if self.dry_run:
                where = slot.points.get(name, "NOT CALIBRATED yet")
                print(f"{tag} click {name} {where}" + (f" -> expect {step['expect']}" if "expect" in step else ""))
                return
            expect, expect_not = step.get("expect"), step.get("expect_not")
            tries = 1 + int(step.get("retries", 2 if (expect or expect_not) else 0))
            timeout = float(step.get("timeout", self.delays.get("expect_timeout", 3)))
            for attempt in range(tries):
                for _ in range(step.get("times", 1)):
                    self.click(slot, name)
                if not (expect or expect_not):
                    break
                if expect and self.wait_for(slot, expect, timeout):
                    break
                if expect_not and self.wait_for(slot, expect_not, timeout, negate=True):
                    break
                if attempt + 1 < tries:
                    log(f"[{slot.label}]   {name}: screen didn't change, retrying ({attempt + 1}/{tries - 1})")
            else:
                raise StepFailed(f"after clicking {name}, expected {expect or 'not ' + expect_not}")
        elif "wait_for" in step:
            if self.dry_run:
                print(f"{tag} wait for {step['wait_for']}")
                return
            if not self.wait_for(slot, step["wait_for"], float(step.get("timeout", 6))):
                raise StepFailed(f"{step['wait_for']} never appeared")
            return
        elif "type" in step:
            text = step["type"].format(**fields)
            if self.dry_run:
                print(f"{tag} type {text!r}")
                return
            if step.get("paste"):
                import pyperclip

                pyperclip.copy(text)
                roblox_key("ctrl+v")
            else:
                self.gui.write(text, interval=self.delays.get("key_interval", 0.03))
        elif "key" in step:
            if self.dry_run:
                print(f"{tag} key {step['key']}")
                return
            roblox_key(step["key"])
        elif "scroll" in step:
            if self.dry_run:
                print(f"{tag} scroll {step['scroll']}")
                return
            self.gui.scroll(int(step["scroll"]))
        else:
            sys.exit(f"Unknown step in config flow: {step}")
        sleep(self.delays.get("after_step", 0.4))

    # ---------- per-slot job: a generator that yields how long it wants to wait ----------

    def steps(self, slot: Slot, steps: list, fields: dict, state: dict):
        for step in steps:
            if stop_event.is_set():
                raise Stopped
            # --test runs everything except the real publish (it cancels instead).
            if step.get("live_only") and self.test:
                continue
            if step.get("test_only") and not self.test:
                continue
            if "wait" in step:
                w = step["wait"]
                yield float(self.delays.get(w, w)) if isinstance(w, str) else float(w)
            else:
                if step.get("publishes"):
                    # Mark before clicking: once this click lands, the outfit is public,
                    # even if what CAC shows afterwards confuses the bot.
                    state["publish_clicked"] = True
                self.do(slot, step, fields)

    def record(self, slot: Slot, outfit: dict) -> None:
        self.published["published"].append({
            "name": outfit["name"],
            "account": slot.label,
            "at": datetime.now().isoformat(timespec="seconds"),
        })
        save_json(PUBLISHED_PATH, self.published)

    def job(self, slot: Slot):
        flow = self.config["flow"]
        max_fail = int(self.config.get("max_failures_per_window", 3))
        while self.queue and slot.count < self.limit:
            outfit = self.queue.pop(0)
            fields = {"name": outfit["name"], "description": outfit.get("description", "")}
            state = {"publish_clicked": False}
            log(f'[{slot.label}] building "{outfit["name"]}" ({len(outfit["items"])} items)')
            try:
                yield from self.steps(slot, flow.get("start", []), fields, state)
                for item_id in outfit["items"]:
                    yield from self.steps(slot, flow.get("per_item", []), {**fields, "item_id": item_id}, state)
                yield from self.steps(slot, flow.get("finish", []), fields, state)
            except StepFailed as e:
                slot.failed += 1
                log(f'[{slot.label}] problem with "{outfit["name"]}": {e}')
                self.snapshot(slot, "failed")
                if state["publish_clicked"]:
                    # The real Publish button was already pressed, so count it as published.
                    self.record(slot, outfit)
                    slot.count += 1
                    log(f'[{slot.label}] (publish was already clicked; counted as published)')
                else:
                    log(f'[{slot.label}] skipped "{outfit["name"]}" (it stays unpublished; next run retries it)')
                if not self.recover(slot):
                    log(f"[{slot.label}] couldn't get back to the main screen; stopping this window.")
                    self.snapshot(slot, "stuck")
                    return
                if slot.failed >= max_fail:
                    log(f"[{slot.label}] {slot.failed} problems; stopping this window.")
                    return
                continue
            slot.count += 1
            if self.dry_run or self.test:
                mode = "dry run" if self.dry_run else "test, cancelled"
                log(f'[{slot.label}] would publish "{outfit["name"]}" ({mode})')
            else:
                self.record(slot, outfit)
                log(f'[{slot.label}] published "{outfit["name"]}"')
            if self.queue and slot.count < self.limit:
                yield self.gap

    # ---------- scheduler: always resume the slot whose wait ends first ----------

    def run(self, slots: list[Slot]) -> None:
        clock = 0.0 if self.dry_run else time.monotonic()
        heap = [(max(s.ready_at, clock), i) for i, s in enumerate(slots)]
        heapq.heapify(heap)
        jobs = [self.job(s) for s in slots]
        started = set()
        while heap:
            ready, i = heapq.heappop(heap)
            if self.dry_run:
                clock = ready
            else:
                sleep(max(0.0, ready - time.monotonic()))
            self.focus(slots[i])
            if i not in started and not self.dry_run:
                started.add(i)
                if not self.recover(slots[i]):
                    log(f"[{slots[i].label}] CAC isn't on its main screen and couldn't be closed back to it; skipping this window.")
                    continue
            try:
                wait = next(jobs[i])
            except StopIteration:
                log(f"[{slots[i].label}] finished ({slots[i].count} published, {slots[i].failed} skipped)")
                continue
            now = clock if self.dry_run else time.monotonic()
            heapq.heappush(heap, (now + wait, i))


# ---------- setting up windows ----------

def states_for(config: dict, win) -> dict:
    """Screen checks (pixel colours) for this window's size."""
    return config.get("states", {}).get(size_key(win.width, win.height), {})


def points_for(config: dict, win, label: str) -> dict:
    """Button positions for this window's size (CAC's layout changes with window size)."""
    key = size_key(win.width, win.height)
    profiles = config.get("points", {})
    if key not in profiles:
        have = ", ".join(profiles) or "none"
        sys.exit(
            f"[{label}] Roblox window is {key} but buttons are only calibrated for: {have}. "
            f"Resize the window to one of those, or run calibrate.py with a window this size."
        )
    return profiles[key]


def fake_points(config: dict) -> dict:
    profiles = config.get("points", {})
    return next(iter(profiles.values()), {})


class FakeWin:
    left = top = 0


def make_slots(config: dict, args) -> list[Slot]:
    title = config["window_title"]
    accounts = config.get("accounts", [])

    if args.launch:
        if not accounts:
            sys.exit('Put your RAM account names in config.json -> "accounts" first.')
        if args.dry_run:
            return [Slot(a, FakeWin(), fake_points(config), {}) for a in accounts]
        import ram

        ram_cfg = config.get("ram", {})
        ram.check_running(ram_cfg)
        slots = []
        for i, account in enumerate(accounts):
            known = {w._hWnd for w in ram.roblox_windows(title)}
            log(f"Launching {account} via Roblox Account Manager...")
            try:
                ram.launch_account(ram_cfg, account)
            except ram.RamError as e:
                log(f"  {e}")
                continue
            win = ram.wait_for_new_window(title, known, ram_cfg.get("window_timeout", 90))
            if not win:
                log(f"  no new Roblox window for {account}; skipping it")
                continue
            ram.place_window(win, i, config.get("launch_window_size"))
            ready = time.monotonic() + float(ram_cfg.get("game_load_seconds", 30))
            slots.append(Slot(account, win, points_for(config, win, account), states_for(config, win), ready))
            log(f"  {account} window found; starts once CAC has loaded")
            sleep(float(ram_cfg.get("seconds_between_launches", 8)))
        return slots

    if args.attach:
        if args.dry_run:
            return [Slot(f"window{i + 1}", FakeWin(), fake_points(config), {}) for i in range(2)]
        import ram

        wins = ram.roblox_windows(title)
        if not wins:
            sys.exit("No Roblox windows open.")
        slots = []
        for i, win in enumerate(wins):
            label = accounts[i] if i < len(accounts) else f"window{i + 1}"
            slots.append(Slot(label, win, points_for(config, win, label), states_for(config, win)))
        return slots

    if args.dry_run:
        return [Slot("main", FakeWin(), fake_points(config), {})]
    win = find_window(title)
    return [Slot("main", win, points_for(config, win, "main"), states_for(config, win))]


def check_items(outfits: list) -> list:
    """Drop item IDs that Roblox doesn't know about. Uses the public asset details endpoint."""
    import requests

    cache: dict[int, bool] = {}
    for outfit in outfits:
        good = []
        for item_id in outfit["items"]:
            if item_id not in cache:
                try:
                    r = requests.get(f"https://economy.roblox.com/v2/assets/{item_id}/details", timeout=10)
                    cache[item_id] = r.ok
                    if r.ok:
                        log(f"  {item_id}: {r.json().get('Name')}")
                    time.sleep(0.3)
                except requests.RequestException:
                    cache[item_id] = True  # network trouble: keep the item rather than drop it
            if cache[item_id]:
                good.append(item_id)
            else:
                log(f'  skipping unknown item {item_id} in "{outfit["name"]}"')
        outfit["items"] = good
    return [o for o in outfits if o["items"]]


def main() -> None:
    parser = argparse.ArgumentParser(description=__doc__, formatter_class=argparse.RawDescriptionHelpFormatter)
    mode = parser.add_mutually_exclusive_group()
    mode.add_argument("--launch", action="store_true", help="launch config accounts via Roblox Account Manager")
    mode.add_argument("--attach", action="store_true", help="use every Roblox window that's already open")
    parser.add_argument("--dry-run", action="store_true", help="print steps, don't touch mouse/keyboard")
    parser.add_argument("--test", action="store_true", help="do everything in-game but cancel instead of publishing")
    parser.add_argument("--check-items", action="store_true", help="validate item IDs before starting")
    parser.add_argument("--limit", type=int, help="max outfits per window this run")
    parser.add_argument("--redo", action="store_true", help="ignore published.json and publish everything again")
    args = parser.parse_args()

    config = load_config()
    outfits = load_json(OUTFITS_PATH)["outfits"]
    published = load_json(PUBLISHED_PATH, default={"published": []})
    done = set() if args.redo else {p["name"] for p in published["published"]}

    queue = [o for o in outfits if o["name"] not in done]
    if args.check_items:
        log("Checking item IDs...")
        queue = check_items(queue)
    if not queue:
        log("Nothing to publish: every outfit in outfits.json is already in published.json.")
        return
    limit = args.limit or config.get("max_publish_per_run", 20)

    if not args.dry_run:
        import keyboard

        keyboard.add_hotkey("end", stop_event.set)

    try:
        slots = make_slots(config, args)
        if not slots:
            sys.exit("No Roblox windows to work with.")
        total = min(len(queue), limit * len(slots))
        log(f"{len(slots)} window(s), {total} outfit(s) to publish"
            f"{' (dry run)' if args.dry_run else ''}. Press End to stop.")
        if not args.dry_run:
            log("Starting in 5 seconds. Don't touch the mouse.")
            sleep(5)
        Bot(config, args.dry_run, args.test, queue, limit, published).run(slots)
    except Stopped:
        log("Stopped by End. Progress so far is saved; run again to continue.")
        return
    except Exception as e:  # includes pyautogui.FailSafeException (mouse in corner)
        log(f"Stopped: {type(e).__name__}: {e}")
        raise SystemExit(1)
    log("Done.")


if __name__ == "__main__":
    main()
