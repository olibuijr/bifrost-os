import importlib.util, os, tempfile, unittest
from unittest import mock

spec = importlib.util.spec_from_file_location("pc_publish", os.path.join(os.path.dirname(__file__), "pc-publish.py"))
pub = importlib.util.module_from_spec(spec)
spec.loader.exec_module(pub)


class NoReferenceAssets(unittest.TestCase):
    """The reference assets are never packed: the publisher has no way to name them any more."""

    def test_publisher_has_no_reference_asset_path(self):
        self.assertFalse(hasattr(pub, "ref_asset_files"))
        self.assertFalse(hasattr(pub, "REF_KINDS"))

    def test_license_status_runs_the_guard_on_the_package(self):
        with mock.patch.object(pub.subprocess, "run") as run:
            run.return_value.returncode = 1
            self.assertEqual(pub.license_status("/x/pkg.pkg.tar.zst"), 1)
        argv = run.call_args[0][0]
        self.assertTrue(argv[1].endswith("tools/check-package-licenses.py"))
        self.assertEqual(argv[2], "/x/pkg.pkg.tar.zst")


class PublicRepoExport(unittest.TestCase):
    def repository(self, root):
        source = os.path.join(root, "repo")
        os.makedirs(source)
        pkg = os.path.join(source, "springboard-shell-1.2.3.123-1-x86_64.pkg.tar.zst")
        with open(pkg, "wb") as f:
            f.write(b"package")
        dbtar = os.path.join(source, "bifrost.db.tar.zst")
        with open(dbtar, "wb") as f:
            f.write(b"pacman database")
        os.symlink("bifrost.db.tar.zst", os.path.join(source, "bifrost.db"))
        return source, pkg

    def test_exports_packages_then_database_to_the_live_static_path(self):
        with tempfile.TemporaryDirectory() as d:
            source, pkg = self.repository(d)
            site = os.path.join(d, "site")
            order = []
            copy = pub._atomic_repo_copy

            def record(src, dst, immutable=False):
                order.append(os.path.basename(dst))
                return copy(src, dst, immutable=immutable)

            with mock.patch.object(pub, "_atomic_repo_copy", side_effect=record):
                repo_dir, files = pub.publish_public_repo(source, site)

            self.assertEqual(repo_dir, os.path.join(site, "arch", "x86_64"))
            self.assertEqual(order[-1], "bifrost.db", "the ordinary pacman database URL is replaced last")
            self.assertEqual(files[-2:], ["bifrost.db.tar.zst", "bifrost.db"])
            with open(os.path.join(repo_dir, os.path.basename(pkg)), "rb") as f:
                self.assertEqual(f.read(), b"package")

    def test_package_names_are_immutable(self):
        with tempfile.TemporaryDirectory() as d:
            source, pkg = self.repository(d)
            site = os.path.join(d, "site")
            repo_dir, _ = pub.publish_public_repo(source, site)
            with open(pkg, "wb") as f:
                f.write(b"different package under the same version")

            with self.assertRaises(FileExistsError):
                pub.publish_public_repo(source, site)
            with open(os.path.join(repo_dir, os.path.basename(pkg)), "rb") as f:
                self.assertEqual(f.read(), b"package")

    def test_export_rejects_source_and_destination_symlinks_that_escape(self):
        with tempfile.TemporaryDirectory() as d:
            source, _ = self.repository(d)
            outside = os.path.join(d, "outside")
            os.makedirs(outside)
            os.symlink(os.path.join(outside, "escaped.pkg.tar.zst"),
                       os.path.join(source, "springboard-shell-9.9.9-1-x86_64.pkg.tar.zst"))
            with self.assertRaises(ValueError):
                pub.publish_public_repo(source, os.path.join(d, "site"))

            clean_source, _ = self.repository(os.path.join(d, "second"))
            site = os.path.join(d, "second-site")
            os.makedirs(site)
            os.symlink(outside, os.path.join(site, "arch"))
            with self.assertRaises(ValueError):
                pub.publish_public_repo(clean_source, site)


if __name__ == "__main__":
    unittest.main()
