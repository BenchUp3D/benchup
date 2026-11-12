"""Debian packaging regression tests; no app compilation or package installation."""
from pathlib import Path
import shutil
import subprocess
import tempfile
import unittest
from unittest.mock import patch

import package_deb as deb


class DebianPackageTests(unittest.TestCase):
    def setUp(self):
        temporary = tempfile.TemporaryDirectory()
        self.addCleanup(temporary.cleanup)
        self.directory = Path(temporary.name)
        self.pack_dir = self.directory / "staged"
        self.pack_dir.mkdir()
        self.binary = self.pack_dir / "benchup-app"
        self.binary.write_bytes(b"\x7fELF\x02\x01" + b"\0" * 12 + b"\x3e\x00" + b"payload")
        self.binary.chmod(0o755)
        for name in ("LICENSE", "README.md", "licenses/skp/LICENSE-openskp",
                     "licenses/skp/LICENSE-miniz", "licenses/skp/LICENSE-earcut",
                     "licenses/skp/LICENSE-flatbuffers", "licenses/skp/LICENSE-fflate"):
            file = self.pack_dir / name
            file.parent.mkdir(parents=True, exist_ok=True)
            file.write_text(name)
        self.current = "1.2.3"
        self.output_dir = self.directory / "dist"

    def test_staging_preserves_binary_licenses_and_desktop_integration(self):
        root = self.directory / "root"
        deb.stage(self.pack_dir, root, self.current, "libc6 (>= 2.34), libudev1 (>= 183)")
        for relative in deb.REQUIRED_FILES:
            self.assertTrue((root / relative).is_file(), relative)
        self.assertEqual((root / "usr/bin/benchup-app").read_bytes(), self.binary.read_bytes())
        self.assertEqual((root / "usr/bin/benchup-app").stat().st_mode & 0o777, 0o755)
        control = (root / "DEBIAN/control").read_text()
        self.assertIn("Version: 1.2.3\nArchitecture: amd64\n", control)
        self.assertIn("Depends: libc6 (>= 2.34), libudev1 (>= 183), ", control)
        for dependency in deb.RUNTIME_DEPENDENCIES:
            self.assertIn(dependency, control)
        self.assertIn("Homepage: https://benchup3d.github.io/editor/\n", control)
        self.assertEqual((root / "usr/share/doc/benchup/copyright").read_text(), "LICENSE")
        self.assertIn("Exec=benchup-app %f", (root / deb.REQUIRED_FILES[1]).read_text())
        self.assertIn("Exec=benchup-app --thumbnail %i %o", (root / deb.REQUIRED_FILES[4]).read_text())
        self.assertFalse(any(root.rglob("*.nupkg")))

    def test_dependency_detection_fails_instead_of_shipping_missing_dependencies(self):
        with patch.object(deb.subprocess, "run", return_value=subprocess.CompletedProcess([], 0, "")):
            with self.assertRaisesRegex(ValueError, "did not report"):
                deb.linked_dependencies(self.binary, self.directory)

    def test_missing_vendor_license_prevents_packaging(self):
        (self.pack_dir / "licenses/skp/LICENSE-openskp").unlink()
        with self.assertRaisesRegex(ValueError, "Missing staged SketchUp reader licenses"):
            deb.stage(self.pack_dir, self.directory / "root", self.current, "libc6")

    @unittest.skipUnless(shutil.which("dpkg-shlibdeps"), "requires dpkg-dev")
    def test_dependency_detection_with_an_existing_system_executable(self):
        # Exercise dpkg-shlibdeps without compiling or packaging anything.
        dependencies = deb.linked_dependencies(Path("/usr/bin/true"), self.directory)
        self.assertIn("libc6", dependencies)

    def test_packaging_reuses_existing_binary_and_uses_fast_rootless_compression(self):
        commands = []

        def run(args, **kwargs):
            commands.append(args)
            if args[0] == "dpkg-shlibdeps":
                return subprocess.CompletedProcess(args, 0, "shlibs:Depends=libc6 (>= 2.34)\n")
            if args[0] == "dpkg-deb":
                root = Path(args[-2])
                self.assertEqual((root / "usr/bin/benchup-app").read_bytes(), self.binary.read_bytes())
                self.assertIn("Depends: libc6 (>= 2.34)", (root / "DEBIAN/control").read_text())
                Path(args[-1]).write_bytes(b"!<arch>\nfixture")
            return subprocess.CompletedProcess(args, 0)

        with patch.object(deb.subprocess, "run", side_effect=run), patch.object(deb, "validate") as validate:
            deb.package(self.pack_dir, self.output_dir, self.current)
        self.assertEqual([command[0] for command in commands],
                         ["dpkg-shlibdeps", "desktop-file-validate", "dpkg-deb"])
        self.assertEqual(commands[-1][1:5], ["--root-owner-group", "-Zgzip", "-z1", "--build"])
        self.assertEqual(Path(commands[-1][-1]).name, "benchup_1.2.3_amd64.deb")
        validate.assert_called_once_with(self.output_dir, self.current)

    def test_incorrect_architecture_and_missing_executable_are_rejected_before_packaging(self):
        with patch.object(deb.subprocess, "run") as run:
            for contents in (b"not an ELF executable", b"\x7fELF\x02\x01" + b"\0" * 12 + b"\xb7\x00"):
                self.binary.write_bytes(contents)
                with self.assertRaisesRegex(ValueError, "x86-64"):
                    deb.package(self.pack_dir, self.output_dir, self.current)
            self.binary.unlink()
            with self.assertRaisesRegex(ValueError, "Missing staged"):
                deb.package(self.pack_dir, self.output_dir, self.current)
        run.assert_not_called()

    def validate(self, metadata=None, omit=None, binary_mode="-rwxr-xr-x"):
        self.output_dir.mkdir(exist_ok=True)
        (self.output_dir / deb.release.deb_filename(self.current)).write_bytes(b"!<arch>\nfixture")
        entries = []
        for relative in deb.REQUIRED_FILES:
            if relative != omit:
                mode = binary_mode if relative == "usr/bin/benchup-app" else "-rw-r--r--"
                entries.append(f"{mode} root/root 1024 2026-11-12 12:00 ./{relative}")
        results = [subprocess.CompletedProcess([], 0, metadata or "benchup\n1.2.3\namd64\nlibc6 (>= 2.34)\n"),
                   subprocess.CompletedProcess([], 0, "\n".join(entries))]
        with patch.object(deb.subprocess, "run", side_effect=results):
            deb.validate(self.output_dir, self.current)

    def test_valid_package_passes_publication_checks(self):
        self.validate()

    def test_wrong_version_architecture_or_dependencies_prevents_publication(self):
        for metadata in ("benchup\n1.2.2\namd64\nlibc6\n", "benchup\n1.2.3\narm64\nlibc6\n",
                         "benchup\n1.2.3\namd64\n\n", "other-app\n1.2.3\namd64\nlibc6\n"):
            with self.subTest(metadata=metadata), self.assertRaisesRegex(ValueError, "metadata"):
                self.validate(metadata=metadata)

    def test_missing_files_or_nonexecutable_binary_prevents_publication(self):
        for relative in deb.REQUIRED_FILES:
            with self.subTest(relative=relative), self.assertRaisesRegex(ValueError, "Missing Debian package file"):
                self.validate(omit=relative)
        with self.assertRaisesRegex(ValueError, "incorrect permissions"):
            self.validate(binary_mode="-rw-r--r--")


if __name__ == "__main__":
    unittest.main()
