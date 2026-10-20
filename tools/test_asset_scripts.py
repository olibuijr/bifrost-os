#!/usr/bin/env python3
"""Tests for the system-assets scripts: packaging/arch/prune-stale-assets (the scriptlet's cleanup of unowned asset
files) and packaging/arch/seed-data (the data dir follows the package's assets), plus the pc-publish staging step.
Everything runs in temporary directories with a fake package owner; nothing real is read or changed.

Run: python3 tools/test_asset_scripts.py
"""
import importlib.util
import os
import shutil
import subprocess
import sys
import tempfile
import unittest

HERE = os.path.dirname(os.path.abspath(__file__))
ROOT = os.path.dirname(HERE)
PRUNE = os.path.join(ROOT, "packaging", "arch", "prune-stale-assets")
SEED = os.path.join(ROOT, "packaging", "arch", "seed-data")
ALLOWLIST = os.path.join(ROOT, "packaging", "assets-allowlist.txt")

LIST = """
# comment
SFUI.ttf                 # trailing comment
fonts/Open-Variable.ttf
wallpapers/*.raw
icons/bifrost.*.raw
"""

# stands in for `pacman -Qo`: prints pacman's "FILE is owned by PACKAGE VERSION" for the files named in $OWNED_FILES
FAKE_OWNER = """#!/bin/sh
for f in "$@"; do
    if grep -qxF -- "$f" "$OWNED_FILES"; then echo "$f is owned by fake-package 1.0-1"; else echo "error: No package owns $f" >&2; fi
done
exit 1
"""


def write_tree(root, rels, text=None):
    for rel in rels:
        path = os.path.join(root, *rel.split("/"))
        os.makedirs(os.path.dirname(path), exist_ok=True)
        with open(path, "w") as f:
            f.write(text if text is not None else rel)


def tree(root):
    out = []
    for dirpath, _, files in os.walk(root):
        for name in files:
            out.append(os.path.relpath(os.path.join(dirpath, name), root).replace(os.sep, "/"))
    return sorted(out)


class Base(unittest.TestCase):
    def setUp(self):
        self.tmp = tempfile.mkdtemp(prefix="asset-scripts-")
        self.addCleanup(shutil.rmtree, self.tmp, True)
        self.assets = os.path.join(self.tmp, "assets")
        self.system = os.path.join(self.tmp, "system-assets")
        self.data = os.path.join(self.tmp, "data")
        self.allow = os.path.join(self.tmp, "allowlist.txt")
        self.owned = os.path.join(self.tmp, "owned.txt")
        self.owner = os.path.join(self.tmp, "fake-owner")
        with open(self.allow, "w") as f:
            f.write(LIST)
        open(self.owned, "w").close()
        with open(self.owner, "w") as f:
            f.write(FAKE_OWNER)
        os.chmod(self.owner, 0o755)


class PruneStaleAssets(Base):
    def prune(self, allow=None):
        env = {**os.environ, "PRUNE_OWNER_CMD": self.owner, "OWNED_FILES": self.owned}
        return subprocess.run(["sh", PRUNE, self.assets, self.system, allow or self.allow], env=env, capture_output=True, text=True)

    def test_superseded_and_unlisted_files_go_and_the_rest_stays(self):
        write_tree(self.assets, ["SFUI.ttf", "icons/bifrost.dock.raw", "icons/bifrost.old.raw", "icons/com.vendor.mail.raw",
                                 "VendorSymbols.otf", "wallpapers/a.raw", "wallpapers/index.ini"])
        write_tree(self.system, ["SFUI.ttf", "icons/bifrost.dock.raw", "wallpapers/a.raw", "wallpapers/b.raw"])
        r = self.prune()
        self.assertEqual(r.returncode, 0, r.stderr)
        # superseded: SFUI.ttf, bifrost.dock, a.raw; not allowlisted: com.vendor.mail, VendorSymbols, index.ini (not listed);
        # allowlisted without a counterpart: bifrost.old stays
        self.assertEqual(tree(self.assets), ["icons/bifrost.old.raw"])
        self.assertIn("removed", r.stdout)
        self.assertIn("icons/com.vendor.mail.raw (not on the allowlist)", r.stdout)
        self.assertIn("SFUI.ttf (superseded by system-assets)", r.stdout)

    def test_files_a_package_owns_are_never_touched(self):
        write_tree(self.assets, ["SFUI.ttf", "icons/com.vendor.mail.raw", "icons/org.app.raw"])
        write_tree(self.system, ["SFUI.ttf"])
        with open(self.owned, "w") as f:
            f.write(f"{self.assets}/SFUI.ttf\n{self.assets}/icons/org.app.raw\n")
        r = self.prune()
        self.assertEqual(r.returncode, 0, r.stderr)
        self.assertEqual(tree(self.assets), ["SFUI.ttf", "icons/org.app.raw"])

    def test_it_is_idempotent_and_removes_emptied_directories(self):
        write_tree(self.assets, ["icons/com.vendor.mail.raw", "SFUI.ttf"])
        write_tree(self.system, ["SFUI.ttf"])
        self.assertEqual(self.prune().returncode, 0)
        self.assertEqual(tree(self.assets), [])
        self.assertFalse(os.path.exists(os.path.join(self.assets, "icons")), "the emptied directory is removed")
        again = self.prune()
        self.assertEqual(again.returncode, 0)
        self.assertEqual(again.stdout.strip(), "", "a second run has nothing to say")

    def test_nothing_happens_until_the_package_has_system_assets(self):
        write_tree(self.assets, ["SFUI.ttf", "icons/com.vendor.mail.raw"])
        r = self.prune()  # SYSTEM does not exist
        self.assertEqual(r.returncode, 0)
        self.assertEqual(tree(self.assets), ["SFUI.ttf", "icons/com.vendor.mail.raw"])
        os.makedirs(self.system)  # present but empty
        self.assertEqual(self.prune().returncode, 0)
        self.assertEqual(tree(self.assets), ["SFUI.ttf", "icons/com.vendor.mail.raw"])

    def test_a_missing_assets_directory_is_fine(self):
        write_tree(self.system, ["SFUI.ttf"])
        self.assertEqual(self.prune().returncode, 0)

    def test_without_an_allowlist_only_superseded_files_go(self):
        write_tree(self.assets, ["SFUI.ttf", "icons/com.vendor.mail.raw"])
        write_tree(self.system, ["SFUI.ttf"])
        r = self.prune(allow=os.path.join(self.tmp, "absent.txt"))
        self.assertEqual(r.returncode, 0, r.stderr)
        self.assertEqual(tree(self.assets), ["icons/com.vendor.mail.raw"])
        self.assertIn("no allowlist", r.stdout)

    def test_a_star_does_not_cross_a_directory(self):
        write_tree(self.assets, ["icons/bifrost.a/b.raw", "icons/bifrost.dock.raw"])
        write_tree(self.system, ["SFUI.ttf"])
        self.assertEqual(self.prune().returncode, 0)
        self.assertEqual(tree(self.assets), ["icons/bifrost.dock.raw"])

    def test_names_with_spaces_are_handled(self):
        write_tree(self.assets, ["icons/com.my app.raw"])
        write_tree(self.system, ["SFUI.ttf"])
        self.assertEqual(self.prune().returncode, 0)
        self.assertEqual(tree(self.assets), [])

    def test_the_real_allowlist_keeps_own_icons_and_drops_vendor_ones(self):
        write_tree(self.assets, ["icons/bifrost.dock.raw", "icons/is.olibuijr.notes.raw", "icons/com.vendor.mail.raw", "SFUI.ttf", "sounds/click.pcm"])
        write_tree(self.system, ["SFUI.ttf"])
        r = self.prune(allow=ALLOWLIST)
        self.assertEqual(r.returncode, 0, r.stderr)
        self.assertEqual(tree(self.assets), ["icons/bifrost.dock.raw", "icons/is.olibuijr.notes.raw"])


class SeedData(Base):
    def seed(self):
        env = {**os.environ, "SEED_SYSTEM_ASSETS": self.system, "SEED_ASSETS": self.assets, "SEED_DATA": self.data}
        return subprocess.run(["sh", SEED], env=env, capture_output=True, text=True)

    def read(self, rel):
        with open(os.path.join(self.data, *rel.split("/"))) as f:
            return f.read()

    def test_the_legacy_directory_seeds_without_overwriting(self):
        write_tree(self.assets, ["SFUI.ttf", "icons/bifrost.dock.raw"])
        write_tree(self.data, ["SFUI.ttf"], text="changed")
        r = self.seed()
        self.assertEqual(r.returncode, 0, r.stderr)
        self.assertEqual(self.read("SFUI.ttf"), "changed")
        self.assertEqual(self.read("icons/bifrost.dock.raw"), "icons/bifrost.dock.raw")
        self.assertTrue(os.path.exists(os.path.join(self.data, ".seeded")))

    def test_system_assets_replace_changed_files_and_retire_icons(self):
        write_tree(self.data, ["icons/com.vendor.mail.raw", "icons/bifrost.dock.raw", "SFUI.ttf", "notes.txt"], text="old")
        write_tree(self.system, ["icons/bifrost.dock.raw", "SFUI.ttf", "wallpapers/a.raw"])
        write_tree(self.assets, ["icons/legacy.raw"])
        r = self.seed()
        self.assertEqual(r.returncode, 0, r.stderr)
        self.assertEqual(self.read("icons/bifrost.dock.raw"), "icons/bifrost.dock.raw", "the package's icon replaces the old one")
        self.assertEqual(self.read("wallpapers/a.raw"), "wallpapers/a.raw")
        self.assertFalse(os.path.exists(os.path.join(self.data, "icons", "com.vendor.mail.raw")), "a retired icon is removed")
        self.assertFalse(os.path.exists(os.path.join(self.data, "icons", "legacy.raw")), "the legacy directory is not read when the package has assets")
        self.assertEqual(self.read("notes.txt"), "old", "files the package does not have are left alone")
        self.assertIn("retired icon com.vendor.mail.raw", r.stdout)

    def test_an_unchanged_set_is_not_copied_again_and_a_changed_set_is(self):
        write_tree(self.system, ["SFUI.ttf"], text="v1")
        self.assertEqual(self.seed().returncode, 0)
        self.assertEqual(self.read("SFUI.ttf"), "v1")
        # the user's copy edited by hand survives a boot while the package set is the same
        write_tree(self.data, ["SFUI.ttf"], text="edited")
        self.assertEqual(self.seed().returncode, 0)
        self.assertEqual(self.read("SFUI.ttf"), "edited")
        # an update that changes the set brings the file back
        write_tree(self.system, ["SFUI.ttf"], text="v2 longer")
        self.assertEqual(self.seed().returncode, 0)
        self.assertEqual(self.read("SFUI.ttf"), "v2 longer")

    def test_user_wallpapers_and_fonts_are_never_deleted(self):
        write_tree(self.data, ["wallpapers/mine.raw", "fonts/Installed.ttf"], text="mine")
        write_tree(self.system, ["wallpapers/a.raw", "icons/bifrost.dock.raw"])
        self.assertEqual(self.seed().returncode, 0)
        self.assertEqual(self.read("wallpapers/mine.raw"), "mine")
        self.assertEqual(self.read("fonts/Installed.ttf"), "mine")

    def test_a_failure_never_blocks_the_session_and_is_retried(self):
        write_tree(self.system, ["a.raw", "icons/b.raw"], text="new")
        # a directory sits where the file is expected, so replacing it fails
        os.makedirs(os.path.join(self.data, "a.raw", "keep"))
        r = self.seed()
        self.assertEqual(r.returncode, 0, "seeding is best effort")
        self.assertIn("cannot replace a.raw", r.stderr)
        self.assertEqual(self.read("icons/b.raw"), "new", "the other files are still copied")
        self.assertFalse(os.path.exists(os.path.join(self.data, ".system-assets")), "no stamp after a failure, so the next start retries")
        self.assertFalse(os.path.exists(os.path.join(self.data, ".seeded")))
        shutil.rmtree(os.path.join(self.data, "a.raw"))
        self.assertEqual(self.seed().returncode, 0)
        self.assertEqual(self.read("a.raw"), "new")
        self.assertTrue(os.path.exists(os.path.join(self.data, ".system-assets")))


class FirmwareHook(Base):
    HOOK = os.path.join(ROOT, "packaging", "arch", "initcpio", "install", "bifrost-firmware")

    def build(self, state="accepted", stamp="7.1.0 20260101-1", kernel=None, firmware="20260101-1"):
        store = os.path.join(self.tmp, "store")
        shutil.rmtree(store, True)
        root = os.path.join(self.tmp, "buildroot")
        shutil.rmtree(root, True)
        write_tree(root, ["usr/lib/firmware/i915/keep_1.bin", "usr/lib/firmware/i915/renamed_2.bin"], text="x")
        write_tree(store, ["firmware"], text="i915/keep_1.bin\n")
        write_tree(store, ["state"], text=state)
        if stamp is not None:
            write_tree(store, ["firmware-stamp"], text=stamp)
        env = {**os.environ, "BIFROST_RECOVERY_DIR": store, "BIFROST_FIRMWARE_VERSION": firmware, "BUILDROOT": root}
        env.pop("KERNELVERSION", None)
        if kernel:
            env["KERNELVERSION"] = kernel
        r = subprocess.run(["bash", "-c", f"source '{self.HOOK}'; build"], env=env, capture_output=True, text=True)
        self.assertEqual(r.returncode, 0, r.stderr)
        return tree(os.path.join(root, "usr", "lib", "firmware"))

    def test_a_matching_stamp_prunes_to_the_recorded_list(self):
        self.assertEqual(self.build(kernel="7.1.0"), ["i915/keep_1.bin"])

    def test_an_update_in_flight_keeps_all_firmware(self):
        both = ["i915/keep_1.bin", "i915/renamed_2.bin"]
        for state in ("system-prepared", "system-probation", "prepared", "probation"):
            self.assertEqual(self.build(state=state), both, state)

    def test_a_new_kernel_or_firmware_package_keeps_all_firmware(self):
        both = ["i915/keep_1.bin", "i915/renamed_2.bin"]
        self.assertEqual(self.build(kernel="7.2.0"), both)
        self.assertEqual(self.build(firmware="20260301-1"), both)
        self.assertEqual(self.build(stamp=None), both, "a list without a stamp is not trusted")


class PublishStaging(unittest.TestCase):
    def load(self):
        spec = importlib.util.spec_from_file_location("pc_publish", os.path.join(HERE, "pc-publish.py"))
        sys.path.insert(0, HERE)
        self.addCleanup(sys.path.remove, HERE)
        mod = importlib.util.module_from_spec(spec)
        spec.loader.exec_module(mod)
        return mod

    def test_only_allowlisted_files_are_staged_for_the_package(self):
        mod = self.load()
        tmp = tempfile.mkdtemp(prefix="asset-publish-")
        self.addCleanup(shutil.rmtree, tmp, True)
        assets, work = os.path.join(tmp, "assets"), os.path.join(tmp, "work")
        os.makedirs(work)
        write_tree(assets, ["SFUI.ttf", "icons/bifrost.dock.raw", "icons/com.vendor.mail.raw", "sounds/click.pcm"])
        env = mod.stage_system_assets(work, assets)
        self.assertEqual(env, {"PC_SYSTEM_ASSETS": os.path.join(work, "system-assets")})
        self.assertEqual(tree(env["PC_SYSTEM_ASSETS"]), ["SFUI.ttf", "icons/bifrost.dock.raw"])

    def test_no_assets_directory_means_no_system_assets_in_the_package(self):
        mod = self.load()
        tmp = tempfile.mkdtemp(prefix="asset-publish-")
        self.addCleanup(shutil.rmtree, tmp, True)
        self.assertEqual(mod.stage_system_assets(tmp, os.path.join(tmp, "absent")), {})


class PackagingWiring(unittest.TestCase):
    def read(self, *parts):
        with open(os.path.join(ROOT, *parts)) as f:
            return f.read()

    def test_the_update_package_installs_system_assets_and_the_helpers(self):
        files = self.read("packaging", "arch", "package-files.sh")
        self.assertIn("usr/share/springboard/system-assets", files)
        self.assertIn("usr/lib/springboard/prune-stale-assets", files)
        self.assertIn("PC_SYSTEM_ASSETS", self.read("packaging", "arch", "fast", "PKGBUILD"))
        self.assertIn("PC_SYSTEM_ASSETS", self.read("tools", "pc-publish.py"))

    def test_the_scriptlet_prunes_and_the_guard_may_overwrite(self):
        self.assertIn("prune-stale-assets", self.read("packaging", "arch", "springboard.install"))
        self.assertIn("/usr/share/springboard/system-assets/*", self.read("packaging", "arch", "recovery", "guard.rs"))

    def test_the_scriptlet_never_restarts_privd_under_a_running_update(self):
        script = self.read("packaging", "arch", "springboard.install")
        lines = [l for l in script.splitlines() if "restart bifrost-privd.service" in l]
        # the direct restart (no update running) and the deferred try-restart
        self.assertEqual(len(lines), 2, lines)
        self.assertEqual(sum("try-restart" in l for l in lines), 1)
        self.assertIn("systemd-run", script, "the update's cgroup is left alone: the restart waits in its own unit")
        self.assertIn("[b]ifrost-update-guard (install|rollback)", script)
        path = os.path.join(ROOT, "packaging", "arch", "springboard.install")
        self.assertEqual(subprocess.run(["bash", "-n", path], capture_output=True).returncode, 0)

    def test_seeding_failures_do_not_gate_the_session_unit(self):
        unit = self.read("packaging", "arch", "springboard.service")
        self.assertIn("ExecStartPre=-/usr/lib/springboard/seed-data", unit)

    def test_the_helpers_are_executable(self):
        for name in ("prune-stale-assets", "seed-data"):
            self.assertTrue(os.access(os.path.join(ROOT, "packaging", "arch", name), os.X_OK), name)


if __name__ == "__main__":
    unittest.main()
