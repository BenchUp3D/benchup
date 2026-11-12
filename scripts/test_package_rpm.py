"""RPM packaging regression checks without compiling, building, or installing."""
from pathlib import Path
import subprocess
import tarfile
import tempfile
import unittest
from unittest.mock import patch

import package_rpm as rpm


class RpmPackageTests(unittest.TestCase):
    def setUp(self):
        temporary = tempfile.TemporaryDirectory()
        self.addCleanup(temporary.cleanup)
        self.directory = Path(temporary.name)
        self.pack_dir = self.directory / "staged"
        self.pack_dir.mkdir()
        self.binary = self.pack_dir / "benchup-app"
        self.binary.write_bytes(b"\x7fELF\x02\x01" + b"\0" * 12 + b"\x3e\x00" + b"payload")
        self.binary.chmod(0o755)
        for name in ("LICENSE", "README.md", *(
            f"licenses/skp/{name}" for name in rpm.linux_package.VENDOR_LICENSES
        )):
            file = self.pack_dir / name
            file.parent.mkdir(parents=True, exist_ok=True)
            file.write_text(name)
        self.current = "1.2.3"
        self.output_dir = self.directory / "dist"
        self.spec = (rpm.release.ROOT / "packaging/linux/benchup.spec").read_text()

    def test_rpm_staging_preserves_binary_and_uses_fedora_license_paths(self):
        root = self.directory / "payload"
        rpm.linux_package.stage(self.pack_dir, root, rpm.LICENSE_DIR, "LICENSE")
        for relative in rpm.REQUIRED_FILES:
            self.assertTrue((root / relative).is_file(), relative)
        self.assertEqual((root / "usr/bin/benchup-app").read_bytes(), self.binary.read_bytes())
        self.assertEqual((root / "usr/share/licenses/benchup/LICENSE").read_text(), "LICENSE")
        self.assertFalse((root / "DEBIAN").exists())
        self.assertFalse((root / "usr/share/doc/benchup/copyright").exists())

    def test_packaging_reuses_existing_binary_without_recompilation_or_stripping(self):
        commands = []

        def run(args, **kwargs):
            commands.append(args)
            if args[0] == "rpmbuild":
                spec = Path(args[-1])
                work_dir = spec.parents[1]
                with tarfile.open(work_dir / "SOURCES/payload.tar") as archive:
                    binary = archive.extractfile("usr/bin/benchup-app")
                    self.assertEqual(binary.read(), self.binary.read_bytes())
                    self.assertEqual(archive.getmember("usr/bin/benchup-app").mode, 0o755)
                    for relative in rpm.REQUIRED_FILES:
                        self.assertTrue(archive.getmember(relative).isfile())
                staged_spec = spec.read_text()
                self.assertNotIn("%build\n", staged_spec)
                self.assertNotIn("AutoReqProv: no", staged_spec)
                self.assertIn("%global __os_install_post %{nil}", staged_spec)
                self.assertIn("%global _binary_payload w1.gzdio", staged_spec)
                file = work_dir / "RPMS/x86_64" / rpm.release.rpm_filename(self.current)
                file.parent.mkdir()
                file.write_bytes(b"\xed\xab\xee\xdbfixture")
            return subprocess.CompletedProcess(args, 0)

        with patch.object(rpm.subprocess, "run", side_effect=run), patch.object(rpm, "validate") as validate:
            rpm.package(self.pack_dir, self.output_dir, self.current)
        self.assertEqual([command[0] for command in commands], ["desktop-file-validate", "rpmbuild"])
        self.assertIn("benchup_version 1.2.3", commands[-1])
        self.assertEqual((self.output_dir / "benchup-1.2.3-1.x86_64.rpm").read_bytes(),
                         b"\xed\xab\xee\xdbfixture")
        validate.assert_called_once_with(self.output_dir, self.current)

    def test_missing_executable_wrong_architecture_or_licenses_prevents_packaging(self):
        with patch.object(rpm.subprocess, "run") as run:
            (self.pack_dir / "licenses/skp/LICENSE-openskp").unlink()
            with self.assertRaisesRegex(ValueError, "Missing staged SketchUp reader licenses"):
                rpm.package(self.pack_dir, self.output_dir, self.current)
            self.binary.write_bytes(b"\x7fELF\x02\x01" + b"\0" * 12 + b"\xb7\x00")
            with self.assertRaisesRegex(ValueError, "x86-64"):
                rpm.package(self.pack_dir, self.output_dir, self.current)
            self.binary.unlink()
            with self.assertRaisesRegex(ValueError, "Missing staged Linux executable"):
                rpm.package(self.pack_dir, self.output_dir, self.current)
        run.assert_not_called()

    def validate(self, metadata=None, dependencies=None, omit=None, binary_mode="-rwxr-xr-x"):
        self.output_dir.mkdir(exist_ok=True)
        (self.output_dir / rpm.release.rpm_filename(self.current)).write_bytes(b"\xed\xab\xee\xdbfixture")
        if dependencies is None:
            dependencies = "\n".join(line.removeprefix("Requires: ")
                                     for line in self.spec.splitlines() if line.startswith("Requires: "))
            dependencies += "\nlibc.so.6(GLIBC_2.34)(64bit)\n"
        entries = []
        for relative in rpm.REQUIRED_FILES:
            if relative != omit:
                mode = binary_mode if relative == "usr/bin/benchup-app" else "-rw-r--r--"
                entries.append(f"/{relative}\t{mode}\troot\troot")
        results = [metadata or "benchup\n1.2.3\n1\nx86_64\n", dependencies, "\n".join(entries)]
        with patch.object(rpm, "query", side_effect=results), patch.object(rpm.subprocess, "run") as run:
            rpm.validate(self.output_dir, self.current)
        self.assertEqual(run.call_args.args[0][:3], ["rpm", "--checksig", "--nosignature"])

    def test_valid_package_passes_publication_checks(self):
        self.validate()

    def test_incorrect_metadata_prevents_publication(self):
        for metadata in ("benchup\n1.2.2\n1\nx86_64\n", "benchup\n1.2.3\n2\nx86_64\n",
                         "benchup\n1.2.3\n1\naarch64\n", "other-app\n1.2.3\n1\nx86_64\n"):
            with self.subTest(metadata=metadata), self.assertRaisesRegex(ValueError, "metadata"):
                self.validate(metadata=metadata)

    def test_missing_automatic_or_dynamic_dependencies_prevents_publication(self):
        explicit = "\n".join(line.removeprefix("Requires: ") for line in self.spec.splitlines()
                             if line.startswith("Requires: "))
        for dependencies in ("", explicit, "libc.so.6(GLIBC_2.34)(64bit)\n"):
            with self.subTest(dependencies=dependencies), self.assertRaisesRegex(ValueError, "dependencies"):
                self.validate(dependencies=dependencies)

    def test_missing_file_or_wrong_permissions_prevents_publication(self):
        for relative in rpm.REQUIRED_FILES:
            with self.subTest(relative=relative), self.assertRaisesRegex(ValueError, "Missing RPM package file"):
                self.validate(omit=relative)
        with self.assertRaisesRegex(ValueError, "incorrect permissions"):
            self.validate(binary_mode="-rw-r--r--")

    def test_digest_failure_prevents_metadata_checks(self):
        self.output_dir.mkdir()
        file = self.output_dir / rpm.release.rpm_filename(self.current)
        file.write_bytes(b"\xed\xab\xee\xdbfixture")
        with patch.object(rpm.subprocess, "run", side_effect=subprocess.CalledProcessError(1, "rpm")), \
                patch.object(rpm, "query") as query:
            with self.assertRaises(subprocess.CalledProcessError):
                rpm.validate(self.output_dir, self.current)
        query.assert_not_called()


if __name__ == "__main__":
    unittest.main()
