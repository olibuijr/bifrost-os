#!/usr/bin/env python3
"""Tests for packaging/arch/wayland/session-policy (the session script's pure decisions).

Run: python3 -I tools/test_session_policy.py
"""
import os
import subprocess
import tempfile
import unittest

HERE = os.path.dirname(os.path.dirname(os.path.abspath(__file__)))
POLICY = os.path.join(HERE, "packaging", "arch", "wayland", "session-policy")


def run_policy(call, *args):
    quoted = " ".join("'" + a.replace("'", "'\\''") + "'" for a in args)
    proc = subprocess.run(
        ["bash", "-c", f"set -euo pipefail; source '{POLICY}'; {call} {quoted}"],
        capture_output=True, text=True, check=True,
    )
    return proc.stdout


def flags(renderer, chrome, mode=None):
    args = [renderer, chrome] + ([mode] if mode else [])
    out = run_policy("session_compositor_flags", *args)
    return [line for line in out.splitlines() if line]


class SessionCompositorFlags(unittest.TestCase):
    def test_default_selects_direct_apps(self):
        self.assertEqual(flags("", ""), ["BIFROST_DIRECT=1"])

    def test_vulkan_renderer_adds_the_renderer_word(self):
        self.assertEqual(flags("vulkan", ""), ["BIFROST_DIRECT=1", "BIFROST_WM_RENDERER=vulkan"])

    def test_chrome_switch_adds_chrome_word(self):
        self.assertEqual(flags("gles", "1"), ["BIFROST_DIRECT=1", "BIFROST_CHROME=1"])

    def test_full_switch_sets_all_three(self):
        self.assertEqual(flags("vulkan", "1", "normal"),
                         ["BIFROST_DIRECT=1", "BIFROST_WM_RENDERER=vulkan", "BIFROST_CHROME=1"])

    def test_chrome_zero_is_off(self):
        self.assertNotIn("BIFROST_CHROME=1", flags("vulkan", "0"))

    def test_safe_mode_has_no_flags_whatever_the_switches(self):
        self.assertEqual(flags("vulkan", "1", "safe"), [])


class SessionScript(unittest.TestCase):
    def setUp(self):
        with open(os.path.join(os.path.dirname(POLICY), "session")) as f:
            self.text = f.read()

    def test_no_other_compositor_is_launched(self):
        self.assertNotIn("labwc", self.text)

    def test_the_fallback_marker_selects_safe_mode(self):
        self.assertIn("/run/bifrost/compositor-fallback ]] && mode=safe", self.text)

    def test_safe_mode_has_its_own_exit_record(self):
        self.assertIn("/run/bifrost/wm-safe-exits", self.text)

    def test_display_refresh_starts_with_the_window_server(self):
        self.assertIn("/usr/lib/bifrost/display-refresh", self.text)

    def test_display_refresh_uses_the_privileged_socket_as_root(self):
        # the output manager is privileged-only: the ordinary socket lists no output manager for wlr-randr
        line = next(l for l in self.text.splitlines() if "/usr/lib/bifrost/display-refresh" in l and not l.lstrip().startswith("#"))
        self.assertIn('WAYLAND_DISPLAY="$priv_socket"', line)
        self.assertNotIn("as_user", line)

    def test_video_arg_uses_the_privileged_socket(self):
        with open(os.path.join(os.path.dirname(POLICY), "video-arg")) as f:
            self.assertIn("WAYLAND_DISPLAY='/run/bifrost/wm-privileged'", f.read())


class WmExitAction(unittest.TestCase):
    def setUp(self):
        self.dir = tempfile.TemporaryDirectory()
        self.file = os.path.join(self.dir.name, "wm-exits")

    def tearDown(self):
        self.dir.cleanup()

    def write(self, text):
        with open(self.file, "w") as f:
            f.write(text)

    def read(self):
        with open(self.file) as f:
            return [line.strip() for line in f if line.strip()]

    def test_first_exit_restarts_and_is_recorded(self):
        self.assertEqual(run_policy("wm_exit_action", "1000", self.file).strip(), "restart")
        self.assertEqual(self.read(), ["1000"])

    def test_exit_older_than_60_seconds_is_forgotten(self):
        self.write("900\n980\n")  # 900 is 100 s old, 980 is 20 s old
        self.assertEqual(run_policy("wm_exit_action", "1000", self.file).strip(), "restart")
        self.assertEqual(self.read(), ["980", "1000"])

    def test_exit_exactly_60_seconds_old_is_forgotten(self):
        self.write("940\n970\n")  # 940 is exactly 60 s old: not within 60 s
        self.assertEqual(run_policy("wm_exit_action", "1000", self.file).strip(), "restart")
        self.assertEqual(self.read(), ["970", "1000"])

    def test_third_exit_within_60_seconds_falls_back(self):
        self.write("950\n970\n")
        self.assertEqual(run_policy("wm_exit_action", "1000", self.file).strip(), "fallback")
        self.assertEqual(self.read(), ["950", "970", "1000"])

    def test_three_spread_out_exits_do_not_fall_back(self):
        self.write("100\n500\n")
        self.assertEqual(run_policy("wm_exit_action", "1000", self.file).strip(), "restart")

    def test_garbage_lines_are_ignored(self):
        self.write("junk\n970\n\n")
        self.assertEqual(run_policy("wm_exit_action", "1000", self.file).strip(), "restart")
        self.assertEqual(self.read(), ["970", "1000"])


def run_policy_full(call, *args):
    quoted = " ".join("'" + a.replace("'", "'\\''") + "'" for a in args)
    return subprocess.run(
        ["bash", "-c", f"set -euo pipefail; source '{POLICY}'; {call} {quoted}"],
        capture_output=True, text=True, check=True,
    )


class SessionSwitch(unittest.TestCase):
    def setUp(self):
        self.dir = tempfile.TemporaryDirectory()

    def tearDown(self):
        self.dir.cleanup()

    def test_a_missing_file_prints_nothing_and_says_nothing(self):
        proc = run_policy_full("session_switch", os.path.join(self.dir.name, "compositor"), "a-z-")
        self.assertEqual((proc.stdout, proc.stderr), ("", ""))

    def test_the_contents_are_reduced_to_the_allowed_characters(self):
        path = os.path.join(self.dir.name, "compositor")
        with open(path, "w") as f:
            f.write("bifrost-wm\n; rm -rf /\n")
        self.assertEqual(run_policy_full("session_switch", path, "a-z-").stdout, "bifrost-wmrm-rf")


class ShellExitNote(unittest.TestCase):
    def note(self, status):
        return run_policy("shell_exit_note", status).strip()

    def test_a_clean_exit_needs_no_note(self):
        self.assertEqual(self.note("0"), "")

    def test_the_stall_and_the_latch_are_named(self):
        self.assertIn("UI loop stalled", self.note("5"))
        self.assertIn("crash-loop latch", self.note("70"))

    def test_other_statuses_are_reported_plainly(self):
        self.assertEqual(self.note("139"), "shell exited 139")


class FakeDrm:
    """A /sys/class/drm and a /usr/lib/dri in a temporary directory."""

    def __init__(self):
        self.tmp = tempfile.TemporaryDirectory()
        self.drm = os.path.join(self.tmp.name, "drm")
        self.dri = os.path.join(self.tmp.name, "dri")
        self.drivers = os.path.join(self.tmp.name, "drivers")
        for d in (self.drm, self.dri, self.drivers):
            os.makedirs(d)

    def node(self, name, driver):
        os.makedirs(os.path.join(self.drm, name, "device"))
        target = os.path.join(self.drivers, driver)
        os.makedirs(target, exist_ok=True)
        os.symlink(target, os.path.join(self.drm, name, "device", "driver"))

    def connector(self, name):
        os.makedirs(os.path.join(self.drm, name))

    def dri_driver(self, *names):
        for n in names:
            open(os.path.join(self.dri, n + "_dri.so"), "w").close()

    def cleanup(self):
        self.tmp.cleanup()

    def override(self, arch="aarch64"):
        return run_policy("session_mesa_override", self.drm, self.dri, arch).strip()


class MesaOverride(unittest.TestCase):
    def setUp(self):
        self.fs = FakeDrm()
        self.addCleanup(self.fs.cleanup)

    def tablet(self):
        self.fs.node("card0", "sprd-dpu")
        self.fs.node("renderD128", "panfrost")
        self.fs.connector("card0-DSI-1")
        self.fs.dri_driver("panfrost", "rockchip", "kms_swrast")

    def test_unknown_display_driver_with_panfrost_render_node_gets_a_kmsro_driver(self):
        self.tablet()
        self.assertEqual(self.fs.override(), "rockchip")

    def test_the_first_installed_kmsro_driver_is_chosen(self):
        self.tablet()
        os.remove(os.path.join(self.fs.dri, "rockchip_dri.so"))
        self.fs.dri_driver("meson", "mediatek")
        self.assertEqual(self.fs.override(), "meson")

    def test_no_kmsro_driver_installed_prints_nothing(self):
        self.tablet()
        os.remove(os.path.join(self.fs.dri, "rockchip_dri.so"))
        self.assertEqual(self.fs.override(), "")

    def test_a_display_driver_mesa_knows_is_left_alone(self):
        self.fs.node("card0", "msm")
        self.fs.node("renderD128", "msm")
        self.fs.dri_driver("msm", "panfrost", "rockchip")
        self.assertEqual(self.fs.override(), "")

    def test_a_dashed_driver_name_matches_its_underscore_file(self):
        self.fs.node("card0", "sun4i-drm")
        self.fs.node("renderD128", "panfrost")
        self.fs.dri_driver("sun4i_drm", "panfrost", "rockchip")
        self.assertEqual(self.fs.override(), "")

    def test_a_pc_gpu_with_an_unknown_name_is_left_alone(self):
        # amdgpu has no amdgpu_dri.so (radeonsi serves it): card and render node share the unknown driver
        self.fs.node("card0", "amdgpu")
        self.fs.node("renderD128", "amdgpu")
        self.fs.dri_driver("radeonsi", "panfrost", "rockchip")
        self.assertEqual(self.fs.override(), "")

    def test_x86_is_never_overridden(self):
        self.tablet()
        self.assertEqual(self.fs.override("x86_64"), "")

    def test_a_second_card_with_a_known_driver_is_left_alone(self):
        self.tablet()
        self.fs.node("card1", "vkms")
        self.fs.dri_driver("vkms")
        self.assertEqual(self.fs.override(), "")

    def test_no_card_prints_nothing(self):
        self.fs.node("renderD128", "panfrost")
        self.fs.dri_driver("panfrost", "rockchip")
        self.assertEqual(self.fs.override(), "")

    def test_a_card_without_a_driver_link_counts_as_unknown(self):
        os.makedirs(os.path.join(self.fs.drm, "card0", "device"))
        self.fs.node("renderD128", "panfrost")
        self.fs.dri_driver("panfrost", "rockchip")
        self.assertEqual(self.fs.override(), "rockchip")

    def test_a_missing_class_dir_prints_nothing(self):
        out = run_policy("session_mesa_override", os.path.join(self.fs.tmp.name, "none"), self.fs.dri, "aarch64")
        self.assertEqual(out.strip(), "")


class WmRetryOverride(unittest.TestCase):
    def retry(self, current):
        return run_policy("session_wm_retry_override", current).strip()

    def test_a_chosen_kmsro_driver_retries_in_software(self):
        self.assertEqual(self.retry("rockchip"), "kms_swrast")

    def test_no_chosen_override_does_not_retry(self):
        self.assertEqual(self.retry(""), "")

    def test_software_has_nothing_further_to_fall_back_to(self):
        self.assertEqual(self.retry("kms_swrast"), "")


class WmEnv(unittest.TestCase):
    def setUp(self):
        self.dir = tempfile.TemporaryDirectory()
        self.addCleanup(self.dir.cleanup)
        self.path = os.path.join(self.dir.name, "wm.env")

    def words(self, text):
        with open(self.path, "w") as f:
            f.write(text)
        return run_policy("session_wm_env", self.path).splitlines()

    def test_a_missing_file_prints_nothing(self):
        out = run_policy_full("session_wm_env", os.path.join(self.dir.name, "none"))
        self.assertEqual((out.stdout, out.stderr), ("", ""))

    def test_plain_words_are_kept_in_order(self):
        self.assertEqual(self.words("BIFROST_WM_SCALE=1.5\nMESA_LOADER_DRIVER_OVERRIDE=panfrost\n"),
                         ["BIFROST_WM_SCALE=1.5", "MESA_LOADER_DRIVER_OVERRIDE=panfrost"])

    def test_comments_blank_lines_and_bad_keys_are_dropped(self):
        self.assertEqual(self.words("# note\n\nlower=1\n1BAD=1\nBAD KEY=1\nGOOD=1\n"), ["GOOD=1"])

    def test_values_with_shell_syntax_are_dropped(self):
        self.assertEqual(self.words("A=$(id)\nB=x y\nC=`id`\nD=a;b\nE=\"q\"\nF=ok\n"), ["F=ok"])

    def test_loader_and_identity_variables_are_refused(self):
        self.assertEqual(self.words("LD_PRELOAD=/x.so\nLD_LIBRARY_PATH=/x\nPATH=/x\nHOME=/x\nXDG_RUNTIME_DIR=/x\nOK=1\n"), ["OK=1"])

    def test_an_empty_value_is_kept(self):
        self.assertEqual(self.words("EMPTY=\n"), ["EMPTY="])

    def test_a_last_line_without_a_newline_is_read(self):
        self.assertEqual(self.words("A=1\nB=2"), ["A=1", "B=2"])

    def test_words_set_finds_a_key(self):
        proc = subprocess.run(["bash", "-c", f"source '{POLICY}'; session_words_set MESA_X A=1 MESA_X=y"])
        self.assertEqual(proc.returncode, 0)

    def test_words_set_does_not_match_a_longer_key(self):
        proc = subprocess.run(["bash", "-c", f"source '{POLICY}'; session_words_set MESA A=1 MESA_X=y"])
        self.assertEqual(proc.returncode, 1)

    def test_words_set_with_no_words_is_false(self):
        proc = subprocess.run(["bash", "-c", f"source '{POLICY}'; session_words_set MESA"])
        self.assertEqual(proc.returncode, 1)


class EfiCmdline(unittest.TestCase):
    def setUp(self):
        self.dir = tempfile.TemporaryDirectory()
        self.addCleanup(self.dir.cleanup)

    def touch(self, rel):
        path = os.path.join(self.dir.name, rel)
        os.makedirs(os.path.dirname(path), exist_ok=True)
        open(path, "w").close()
        return path

    def cmdline(self):
        return run_policy("session_efi_cmdline", self.dir.name).strip()

    def test_no_efi_directory_prints_nothing(self):
        self.assertEqual(self.cmdline(), "")

    def test_the_bifrost_loader_file_comes_first(self):
        self.touch("EFI/BOOT/cmdline.txt")
        boot_cfg = self.touch("EFI/BIFROST/boot.cfg")
        self.assertEqual(self.cmdline(), boot_cfg)

    def test_the_fallback_loader_file_is_used(self):
        path = self.touch("EFI/BOOT/cmdline.txt")
        self.assertEqual(self.cmdline(), path)


class VideoArgWithoutEfi(unittest.TestCase):
    def test_it_exits_quietly_when_there_is_no_command_line_file(self):
        with tempfile.TemporaryDirectory() as boot:
            proc = subprocess.run(
                ["python3", "-I", os.path.join(os.path.dirname(POLICY), "video-arg")],
                capture_output=True, text=True, env=dict(os.environ, BIFROST_BOOT_DIR=boot),
            )
        self.assertEqual((proc.returncode, proc.stdout, proc.stderr), (0, "", ""))


class SessionScriptTablet(unittest.TestCase):
    def setUp(self):
        with open(os.path.join(os.path.dirname(POLICY), "session")) as f:
            self.text = f.read()

    def test_wm_env_is_read_through_the_policy(self):
        self.assertIn("session_wm_env /etc/bifrost/wm.env", self.text)

    def test_the_mesa_override_comes_from_the_policy_and_wm_env_wins(self):
        self.assertIn("session_words_set MESA_LOADER_DRIVER_OVERRIDE", self.text)
        self.assertIn("session_mesa_override /sys/class/drm /usr/lib/dri", self.text)

    def test_a_failed_start_retries_once_in_software(self):
        self.assertIn("session_wm_retry_override", self.text)
        self.assertIn("/run/bifrost/mesa-software", self.text)

    def test_video_arg_runs_only_with_an_efi_command_line(self):
        self.assertIn('[[ -n "$(session_efi_cmdline /boot)" ]] && /usr/lib/bifrost/video-arg', self.text)


if __name__ == "__main__":
    unittest.main()
