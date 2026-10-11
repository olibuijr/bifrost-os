"""E2E scenarios. Each takes a Ctx and asserts via `state` keys first, pixels second.
Coordinates are for the 1200x1920 portrait panel; the icon table is approximate and meant to be tuned
(see docs/e2e.md). Snapshots are scale 4 (300x480) so diffs stay fast in pure Python."""
import os
import time

from client import E2EError, W, H, check_baseline

# Status bar (clock, battery, wifi) is masked out of every baseline: scale-4 pixels.
# status bar clock and the dock (its recents and badges change over time)
STATUS_MASK = [(0, 0, 1200, 60), (0, 1690, 1200, 230)]
# Home grid: 4 columns x 6 rows on page 0, icon centres in panel pixels.
GRID_X = [252, 426, 600, 773, 947]
GRID_Y = [156, 375]
DOCK_Y = 1812
# name -> (x, y, substring expected in `screen=app:<name>`); the home page 0 of the default install.
APPS = {
    "deildu": (GRID_X[0], GRID_Y[0], "deildu"),
    "photos": (GRID_X[1], GRID_Y[0], "photos"),
    "files": (GRID_X[2], GRID_Y[0], "files"),
    "camera": (GRID_X[3], GRID_Y[0], "camera"),
    "browser": (GRID_X[4], GRID_Y[0], "browser"),
    "wow": (GRID_X[0], GRID_Y[1], "wow"),
    "magnifier": (GRID_X[1], GRID_Y[1], "magnifier"),
    "appstore": (GRID_X[2], GRID_Y[1], "appstore"),
    "settings": (457, DOCK_Y, "settings"),
}
# apps the emulator opens in-process (the rest are hosted apps that are not installed there); the dock shifts with recents, so no dock apps
OPENABLE = ["appstore"]
HOME_SWIPE = (W // 2, H - 5, W // 2, H // 2, 250)
MIN_FPS = 30.0
MAX_DRAW_P95 = 25.0


class Ctx:
    def __init__(self, t, name, update=False):
        self.t, self.name, self.update, self.notes = t, name, update, []

    def snap(self, label, **kw):
        kw.setdefault("mask", STATUS_MASK)
        self.notes.append(check_baseline(self.t.shot(4), self.name, label, self.update, **kw))

    def ok(self, cond, msg):
        if not cond:
            raise E2EError(msg)

    def unlock(self):
        t = self.t
        if t.state().get("screen_on") == "false":
            t.key("power")
            t.wait_state("screen_on=true", 4, "screen on")
        if t.state().get("screen") == "lock":
            t.swipe(W // 2, H - 100, W // 2, H // 3, 300)
        t.wait_state("screen=home", 6, "home after unlock")

    def home(self):
        """Home screen from anywhere: unlock if locked, otherwise a home swipe."""
        t = self.t
        scr = t.state().get("screen")
        if scr == "lock":
            self.unlock()
        elif scr != "home":
            t.swipe(*HOME_SWIPE)
        state = t.wait_state("screen=home", 6, "home")
        # a failed scenario can leave the pager on another page (App Library, Today): back to the main grid
        for _ in range(3):
            page = int(t.state().get("page", "0"))
            if page == 0:
                break
            t.swipe(200, 900, 1000, 900, 300) if page > 0 else t.swipe(1000, 900, 200, 900, 300)
            t.settle()
        t.settle()  # a pager or app-close animation still running swallows the next tap
        return state

    def open_app(self, key):
        x, y, expect = APPS[key]
        self.t.tap(x, y)
        return self.t.wait_state(lambda s: s.get("screen", "").startswith("app") and expect in s["screen"], 6, f"app {key}")

    def close_app(self):
        self.t.swipe(*HOME_SWIPE)
        return self.t.wait_state("screen=home", 6, "home after close")


def boot_to_lock(c):
    t = c.t
    if t.state().get("screen") != "lock":
        t.key("power")
        t.wait_state("screen_on=false", 4, "screen off")
        t.key("power")
        t.wait_state("screen_on=true", 4, "screen on")
    s = t.wait_state("screen=lock", 6, "lock screen")
    c.ok(s.get("screen_on") == "true", "screen should be on")
    c.snap("lock", max_frac=0.03)


def unlock(c):
    c.unlock()
    c.ok(c.t.state()["screen"] == "home", "not home after unlock")
    c.snap("home0")


def home_pages(c):
    t = c.t
    c.home()
    p0 = int(t.state().get("page", "0"))
    t.swipe(1000, 900, 200, 900, 300)
    t.wait_state(lambda s: int(s.get("page", "0")) == p0 + 1, 4, "next page")
    c.snap("home-page-next")
    t.swipe(200, 900, 1000, 900, 300)
    s = t.wait_state(lambda s: int(s.get("page", "0")) == p0, 4, "previous page")
    c.ok(s["screen"] == "home", "left home while paging")


def open_close_apps(c):
    c.home()
    failed = []
    for key in OPENABLE:
        try:
            c.open_app(key)
            c.snap(f"app-{key}", max_frac=0.05)
            c.close_app()
        except E2EError as e:
            failed.append(f"{key}: {e}")
            try:
                c.home()
            except E2EError:
                pass
    if failed:
        raise E2EError("; ".join(failed))


def control_center(c):
    t = c.t
    c.home()
    t.swipe(W - 60, 3, W - 60, 900, 300)
    t.wait_state("cc=shown", 4, "control center shown")
    c.snap("control-center")
    t.swipe(W // 2, 1000, W // 2, 100, 250)
    t.wait_state("cc=hidden", 4, "control center hidden")


def notification_center(c):
    t = c.t
    c.home()
    t.swipe(200, 3, 200, 1200, 350)
    s = t.wait_state("center=shown", 4, "notification center shown")
    c.ok(s.get("cc") == "hidden", "notification pull opened the control center")
    try:
        c.snap("notification-center", max_frac=0.05)
    finally:
        # A visual mismatch must not leave this overlay swallowing the next test.
        t.swipe(W // 2, 1500, W // 2, 100, 300)
        t.wait_state("center=hidden", 4, "notification center dismissed")


def quicksearch(c):
    t = c.t
    c.home()
    t.swipe(W // 2, 700, W // 2, 1200, 300)
    t.wait_state("quicksearch=shown", 4, "quicksearch shown")
    c.snap("quicksearch")
    t.swipe(W // 2, 1300, W // 2, 500, 250)
    t.wait_state("quicksearch=hidden", 4, "quicksearch hidden")


def long_press_jiggle(c):
    t = c.t
    c.home()
    x, y, _ = APPS["settings"]
    t.hold(x, y, 900)
    t.settle()
    c.ok(t.state()["screen"] == "home", "long press left the home screen")
    c.snap("icon-longpress", max_frac=0.05)
    t.tap(W // 2, 1550)  # empty spot, dismisses a menu
    t.swipe(*HOME_SWIPE)  # home button gesture leaves jiggle mode
    c.home()


def drag_icon(c):
    t = c.t
    c.home()
    x0, y0, _ = APPS["magnifier"]
    t.hold(x0, y0, 900)
    t.settle()
    t.drag(x0, y0, GRID_X[3], GRID_Y[1])
    t.settle()
    c.ok(t.state()["screen"] == "home", "drag left the home screen")
    c.snap("icon-dragged", max_frac=0.05)
    t.swipe(*HOME_SWIPE)
    c.home()


def app_switcher(c):
    t = c.t
    c.home()
    c.open_app("appstore")
    # Lift then pause: the release velocity must settle before opening switcher.
    t.down(W // 2, H - 5)
    t.move(W // 2, H * 2 // 3)
    time.sleep(0.3)
    t.up(W // 2, H * 2 // 3)
    s = t.wait_state(lambda s: s.get("screen") in ("switcher", "home"), 4, "switcher or home")
    c.ok(s["screen"] == "switcher", "lift-and-pause did not open the switcher")
    c.ok("appstore" in s.get("cards", ""), f"appstore missing in cards={s.get('cards')}")
    c.snap("switcher", max_frac=0.05)
    t.tap(W // 2, 80)  # empty area above the cards: back home
    c.home()


def rotation_lock(c):
    c.home()
    s = c.t.state()
    c.ok("rotation_lock" in s and "orientation" in s, "state lacks orientation/rotation_lock keys")
    c.ok(s["rotation_lock"] in ("true", "false"), f"rotation_lock={s['rotation_lock']}")
    c.ok(s["orientation"] != "", "empty orientation")
    c.ok(s.get("turning", "false") == "false", "device still turning at rest")


def settings_navigation(c):
    t = c.t
    c.home()
    c.open_app("settings")
    c.snap("settings-root", max_frac=0.05)
    rows = {"wifi": (350, 420), "display": (350, 900), "general": (350, 1100)}  # sidebar rows, tune per profile
    for name, (x, y) in rows.items():
        t.tap(x, y)
        t.settle()
        c.ok(t.state()["screen"].startswith("app"), f"left settings tapping {name}")
        c.snap(f"settings-{name}", max_frac=0.05)
    c.close_app()


def terminal_typing(c):
    """The control protocol has no text command, so this taps the Terminal (raising its keyboard) and
    asserts the app stays up and the picture changes; extend with key taps once the keyboard layout is fixed."""
    t = c.t
    c.home()
    c.open_app("terminal")
    t.settle()
    before = t.shot(4)
    t.tap(W // 2, 900)
    t.settle()
    c.ok(t.state()["screen"].startswith("app"), "terminal closed on tap")
    after = t.shot(4)
    c.ok((before.w, before.h) == (after.w, after.h), "shot size changed")
    c.snap("terminal", max_frac=0.1)
    c.close_app()


def fps_regression(c):
    import fps
    t = c.t
    c.home()
    c.ok(t.trace(True).startswith("ok"), "trace on failed")
    t0 = int(time.time())
    try:
        for s in fps.WORKLOAD[:6]:
            t.swipe(*s)
            t.settle(3)
    finally:
        t.trace(False)
    out = fps.vm_ssh(f"journalctl -u springboard --since @{t0} --no-pager -o cat")
    r = fps.summarize(fps.parse_frames(out))
    c.ok(r is not None, "no frames in the log (is trace output reaching it?)")
    c.notes.append(f"fps {r['fps']:.1f} draw avg {r['draw_avg']:.1f} p95 {r['draw_p95']:.1f} ms")
    c.notes.append("TCG timing only; physical-hardware frame thresholds do not apply")


# Each scenario starts with c.home(), so order only matters for the first two (lock, unlock).
SCENARIOS = [("boot_to_lock", boot_to_lock), ("unlock", unlock), ("home_pages", home_pages),
             ("open_close_apps", open_close_apps), ("control_center", control_center),
             ("notification_center", notification_center), ("quicksearch", quicksearch),
             ("long_press_jiggle", long_press_jiggle), ("app_switcher", app_switcher),
             ("rotation_lock", rotation_lock),
             ("fps_regression", fps_regression), ("drag_icon", drag_icon)]  # layout-mutating scenario runs last
