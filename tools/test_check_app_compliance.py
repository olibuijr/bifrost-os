import importlib.util
from pathlib import Path
import unittest

HERE = Path(__file__).resolve().parent
spec = importlib.util.spec_from_file_location("app_compliance", HERE / "check-app-compliance.py")
compliance = importlib.util.module_from_spec(spec)
spec.loader.exec_module(compliance)


class ResidualWakeDetection(unittest.TestCase):
    def test_files_drag_and_debounce_deadlines_are_not_worker_polling(self):
        files = HERE.parent / "springboard/apps/files/src/main.rs"
        lines = files.read_text().splitlines()
        start = next(i for i, line in enumerate(lines) if "fn next_wake(" in line)
        body = compliance.rust_function_body(lines, start)
        source = "\n".join(p.read_text() for p in (files.parent / "ops.rs", files.parent / "search.rs"))

        self.assertIn("next_deadline", body)
        self.assertIn("t + THUMB_POLL_MS", body)
        self.assertFalse(compliance.res02_required(body, source, []))

    def test_pending_receiver_poll_is_reported_despite_unrelated_wake_facilities(self):
        body = """fn next_wake(&self, t: f64) -> Option<f64> {
            if self.job.is_some() { Some(t + 0.05) } else { None }
        }"""
        source = """
            use std::sync::mpsc::Receiver;
            fn wake_fd(&self) -> Option<i32> { self.monitor_fd }
            DispatchQueue::main().async_(|| {});
        """

        self.assertTrue(compliance.res02_required(body, source, []))


class EditMenuMerge(unittest.TestCase):
    """CHR-01: the shell merges an app menu titled Edit, so only the SDK's standard Edit menu is a second copy."""

    def test_an_app_edit_menu_with_its_own_items_is_not_reported(self):
        for code in (
            'AppMenu { title: "Edit".into(), items: find_items(page, self.find.open) },',
            'title: "Edit".into(),',
            'vec![self.file_menu(), self.edit_menu(), self.format_menu(), self.view_menu()]',
            'fn edit_menu(&self) -> AppMenu {',
            '("Edit".into(), vec![MenuEntry::new("Find", "find")])',
        ):
            self.assertIsNone(compliance.chr01(code), code)

    def test_the_sdk_standard_edit_menu_is_reported(self):
        self.assertIsNotNone(compliance.chr01('app.set_menus(vec![MenuEntry::edit_menu(), file]);'))
        self.assertIsNotNone(compliance.chr01('let edit = bifrost_app::view::MenuEntry::edit_menu ( );'))

    def test_standard_items_spelled_out_are_chr02_only(self):
        code = 'AppMenuItem::new(MENU_COPY, "Copy").key("cmd+c"),'
        self.assertIsNone(compliance.chr01(code))
        self.assertEqual(compliance.STD_EDIT_ITEM.search(code).group(1), "Copy")
        self.assertIsNone(compliance.STD_EDIT_ITEM.search('AppMenuItem::new(MENU_FIND, "Find in Page"),'))

    def test_the_shell_still_merges_edit(self):
        bar = (HERE.parent / "springboard/src/shell/desktop_bar.rs").read_text()
        start = bar.index("fn shell_menu(")
        self.assertIn('(Menu::Edit,"Edit")', bar[start:start + 400])


class ManifestPermissions(unittest.TestCase):
    """APP-06: the known permissions are the ones the shell's Policy::from_permissions accepts."""

    SOURCE = """
pub const DEVICES_PERMISSION: &str = "audio-devices";
impl Policy {
    pub fn holds(&self, name: &str) -> bool {
        match name { "system-monitor" => self.system_monitor, _ => false }
    }
    pub fn from_permissions(id: &str, permissions: &[String]) -> io::Result<Self> {
        for permission in permissions {
            match permission.as_str() {
                "network" => p.network = true,
                DEVICES_PERMISSION if id == DEVICES_APP_ID => p.audio_devices = true,
                "camera" if id == CAMERA_APP_ID => p.camera = true,
                // "retired" => p.retired = true,
                _ => return Err(PolicyError::UnsupportedPermission(permission.clone()).into()),
            }
        }
        Ok(p)
    }
}
"""

    def test_names_are_read_from_the_match_arms(self):
        self.assertEqual(compliance.manifest_permissions(self.SOURCE), {"network", "audio-devices", "camera"})

    def test_a_source_without_the_function_or_the_constant_is_an_error(self):
        with self.assertRaises(ValueError):
            compliance.manifest_permissions("fn other() {}")
        with self.assertRaises(ValueError):
            compliance.manifest_permissions(self.SOURCE.replace('pub const DEVICES_PERMISSION: &str = "audio-devices";', ""))

    def test_the_list_follows_the_shell_source(self):
        source = compliance.APP_POLICY.read_text()
        self.assertEqual(compliance.KNOWN_PERMS, compliance.manifest_permissions(source))
        for name in ("calendars", "audio-devices", "gpu", "network", "keychain", "remote-desktop"):
            self.assertIn(name, compliance.KNOWN_PERMS)
            self.assertIn(f'"{name}"', source)
        # entitlements are a separate manifest key, never a permission
        for name in ("system-monitor", "hardware-info", "log-store"):
            self.assertNotIn(name, compliance.KNOWN_PERMS)

    def test_platform_permissions_are_known_and_an_invented_one_is_not(self):
        self.assertEqual(compliance.permission_findings({"calendars", "audio-devices", "gpu"}, "example", ""), [])
        self.assertEqual(compliance.permission_findings({"teleport"}, "example", ""), ["unknown permission 'teleport'"])


class KeychainUse(unittest.TestCase):
    """APP-06: the SDK's exchange_notes sync and the passwords store use the keychain on the app's behalf."""

    def test_exchange_notes_sync_counts_as_keychain_use(self):
        for code in (
            "exchange: bifrost_app::exchange_notes::Sync,",
            "use bifrost_app::exchange_notes::{Op, Sync};",
            "let id = passwords::fill(ctx, site, user);",
            "keychain_groups::get(ctx, GROUP, site, user);",
            "bifrost_app::keychain::add(ctx, SERVICE, account, secret);",
        ):
            self.assertEqual(compliance.permission_findings({"keychain"}, "example", code), [], code)

    def test_keychain_without_a_use_is_still_reported(self):
        for code in (
            "",
            "use bifrost_app::exchange_notes::{ItemId, Op, Report};",
            "let password_field = self.passwords_shown;",
        ):
            self.assertEqual(
                compliance.permission_findings({"keychain"}, "example", code),
                ["permission 'keychain' requested but no code uses it"],
                code,
            )

    def test_notes_declares_keychain_and_uses_the_sync(self):
        notes = HERE.parent / "springboard/apps/notes-desktop"
        self.assertIn('"keychain"', (notes / "manifest.toml").read_text())
        code = "\n".join(line.split("//")[0] for line in (notes / "src/main.rs").read_text().splitlines())
        self.assertEqual(compliance.permission_findings({"keychain"}, notes.name, code), [])


if __name__ == "__main__":
    unittest.main()
