"""Guard the privileged mkarchiso validation hook against recompiling the EFI loader as root."""
from pathlib import Path
import re
import unittest


SOURCE = Path(__file__).with_name("prepare-mkarchiso.py").read_text()
INJECT = re.search(r"INJECT = r'''(.*?)'''", SOURCE, re.DOTALL).group(1)
VALIDATOR = re.search(
    r"_validate_requirements_bootmode_uefi\.bifrost\(\) \{(.*?)\n\}", INJECT, re.DOTALL
).group(1)


class MkarchisoEfiValidationTests(unittest.TestCase):
    def test_root_validation_verifies_existing_loader_without_building(self):
        self.assertIn("tools/verify-efi-loader.sh", VALIDATOR)
        self.assertNotIn("tools/build-efi-boot.sh", VALIDATOR)
        self.assertIn("missing or stale", VALIDATOR)


if __name__ == "__main__":
    unittest.main()
