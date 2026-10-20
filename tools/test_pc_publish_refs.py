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


class RepositorySigning(unittest.TestCase):
    def test_no_key_means_unsigned_with_a_loud_warning(self):
        self.assertIsNone(pub.signing_config({}))
        self.assertIsNone(pub.signing_config({"BIFROST_REPO_SIGN_KEY": "  "}))
        self.assertEqual(pub.signing_args(None), [])
        self.assertEqual(pub.signing_env(None), {})
        self.assertIn("UNSIGNED", pub.unsigned_warning())

    def test_a_key_signs_with_its_own_gnupg_home_under_out_lab(self):
        cfg = pub.signing_config({"BIFROST_REPO_SIGN_KEY": "ABCD1234"})
        self.assertEqual(pub.signing_args(cfg), ["--sign", "--key", "ABCD1234"])
        self.assertTrue(cfg["gnupghome"].endswith("out/lab/gnupg"))
        self.assertEqual(pub.signing_env(cfg), {"GNUPGHOME": cfg["gnupghome"]})
        other = pub.signing_config({"BIFROST_REPO_SIGN_KEY": "K", "BIFROST_REPO_GNUPGHOME": "/srv/keys"})
        self.assertEqual(other["gnupghome"], "/srv/keys")

    def test_a_configured_key_that_is_missing_stops_the_publish(self):
        cfg = pub.signing_config({"BIFROST_REPO_SIGN_KEY": "ABCD1234"})
        missing = mock.Mock(return_value=mock.Mock(returncode=2))
        with self.assertRaises(SystemExit):
            pub.check_signing_key(cfg, runner=missing)
        pub.check_signing_key(cfg, runner=mock.Mock(return_value=mock.Mock(returncode=0)))

    def test_signatures_are_exported_next_to_what_they_sign(self):
        with tempfile.TemporaryDirectory() as d:
            source = os.path.join(d, "repo")
            os.makedirs(source)
            name = "springboard-shell-1.2.3.123-1-x86_64.pkg.tar.zst"
            for fname, data in ((name, b"package"), (name + ".sig", b"psig"), ("bifrost.db.tar.zst", b"db"),
                                ("bifrost.db.tar.zst.sig", b"dbsig")):
                with open(os.path.join(source, fname), "wb") as f:
                    f.write(data)
            os.symlink("bifrost.db.tar.zst", os.path.join(source, "bifrost.db"))
            os.symlink("bifrost.db.tar.zst.sig", os.path.join(source, "bifrost.db.sig"))
            repo_dir, files = pub.publish_public_repo(source, os.path.join(d, "site"))
            self.assertEqual(files[-1], "bifrost.db", "the ordinary database URL is still replaced last")
            for fname in (name + ".sig", "bifrost.db.tar.zst.sig", "bifrost.db.sig"):
                self.assertIn(fname, files)
                self.assertTrue(os.path.isfile(os.path.join(repo_dir, fname)), fname)
            with open(os.path.join(repo_dir, "bifrost.db.sig"), "rb") as f:
                self.assertEqual(f.read(), b"dbsig")

    def test_an_unsigned_database_removes_the_stale_database_signature(self):
        with tempfile.TemporaryDirectory() as d:
            source = os.path.join(d, "repo")
            os.makedirs(source)
            with open(os.path.join(source, "springboard-shell-1.2.4.1-1-x86_64.pkg.tar.zst"), "wb") as f:
                f.write(b"package")
            with open(os.path.join(source, "bifrost.db.tar.zst"), "wb") as f:
                f.write(b"db")
            os.symlink("bifrost.db.tar.zst", os.path.join(source, "bifrost.db"))
            site = os.path.join(d, "site")
            repo_dir = os.path.join(site, "arch", "x86_64")
            os.makedirs(repo_dir)
            for stale in ("bifrost.db.sig", "bifrost.db.tar.zst.sig"):
                with open(os.path.join(repo_dir, stale), "wb") as f:
                    f.write(b"old")
            pub.publish_public_repo(source, site)
            self.assertFalse(os.path.exists(os.path.join(repo_dir, "bifrost.db.sig")))
            self.assertFalse(os.path.exists(os.path.join(repo_dir, "bifrost.db.tar.zst.sig")))


if __name__ == "__main__":
    unittest.main()
