"""Release checks that do not build or install the application."""
import hashlib
import base64
import contextlib
import copy
import io
import json
import os
import plistlib
from pathlib import Path
import subprocess
import sys
import tempfile
import tomllib
import unittest
from unittest.mock import patch

import desktop_release as release
import configure_macos_signing as macos_signing


class DesktopReleaseTests(unittest.TestCase):
    def setUp(self):
        self.temporary = tempfile.TemporaryDirectory()
        self.addCleanup(self.temporary.cleanup)
        self.directory = Path(self.temporary.name)
        self.current = tomllib.loads((release.ROOT / "Cargo.toml").read_text())["workspace"]["package"]["version"]
        self.tag = f"v{self.current}"

    def assets(self, platform):
        channel, _, _, suffix = release.PLATFORMS[platform]
        filename = f"{release.APP_ID}-{self.current}-{channel}-full.nupkg"
        contents = b"test package contents"
        (self.directory / filename).write_bytes(contents)
        (self.directory / f"{release.APP_ID}-{channel}-Setup{suffix}").write_bytes(b"installer")
        if platform == "linux-x64":
            (self.directory / release.deb_filename(self.current)).write_bytes(b"!<arch>\nfixture")
            (self.directory / release.rpm_filename(self.current)).write_bytes(b"\xed\xab\xee\xdbfixture")
        asset = {
            "PackageId": release.APP_ID, "Version": self.current, "Type": "Full",
            "FileName": filename, "Size": len(contents),
            "SHA256": hashlib.sha256(contents).hexdigest().upper(),
        }
        feed = self.directory / f"releases.{channel}.json"
        feed.write_text(json.dumps({"Assets": [asset]}), encoding="utf-8-sig")
        return feed, asset

    def test_packaging_uses_new_identity_executable_and_bundle_on_every_platform(self):
        self.assertEqual(release.APP_ID, "io.github.BenchUp3D.BenchUp")
        for platform in release.PLATFORMS:
            with self.subTest(platform=platform):
                executable = release.PLATFORMS[platform][1]
                self.assertEqual(executable, "benchup-app.exe" if platform.startswith("windows") else "benchup-app")
                (self.directory / executable).touch()
                with patch.dict(os.environ, {}, clear=True), patch.object(release.subprocess, "run") as run, patch.object(release, "validate_assets"):
                    release.package(platform, self.tag, self.directory, self.directory / "dist")
                args = run.call_args.args[0]
                self.assertEqual(args[args.index("--packId") + 1], release.APP_ID)
                self.assertEqual(args[args.index("--packTitle") + 1], "BenchUp")
                self.assertEqual(args[args.index("--mainExe") + 1], executable)
                self.assertEqual(args[args.index("--channel") + 1], release.PLATFORMS[platform][0])
                self.assertEqual(args[args.index("--runtime") + 1], release.PLATFORMS[platform][0])
                self.assertEqual(args[args.index("--delta") + 1], "None")
                self.assertTrue(Path(args[args.index("--icon") + 1]).is_file())
                if platform.startswith("macos"):
                    self.assertNotIn("--bundleId", args)

    def test_macos_bundle_declares_project_opening_and_png_thumbnail_conformance(self):
        (self.directory / "benchup-app").touch()
        for platform in ("macos-arm64", "macos-x64"):
            with self.subTest(platform=platform):
                captured = []
                def inspect(args, **kwargs):
                    captured.append(plistlib.loads(Path(args[args.index("--plist") + 1]).read_bytes()))
                with patch.dict(os.environ, {}, clear=True), patch.object(release.subprocess, "run", side_effect=inspect), patch.object(release, "validate_assets"):
                    release.package(platform, self.tag, self.directory, self.directory / "dist")
                plist = captured[0]
                self.assertEqual(plist["CFBundleIdentifier"], release.APP_ID)
                self.assertEqual(plist["CFBundleVersion"], self.current)
                self.assertEqual(plist["CFBundleExecutable"], "benchup-app")
                doc = plist["CFBundleDocumentTypes"][0]
                uti = plist["UTExportedTypeDeclarations"][0]
                self.assertEqual(doc["LSItemContentTypes"], [uti["UTTypeIdentifier"]])
                self.assertEqual(uti["UTTypeTagSpecification"]["public.filename-extension"], ["bup"])
                self.assertIn("public.png", uti["UTTypeConformsTo"])

    def test_delta_in_migration_feed_prevents_publication(self):
        feed, asset = self.assets("windows-x64")
        delta = dict(asset, Type="Delta")
        feed.write_text(json.dumps({"Assets": [asset, delta]}))
        with self.assertRaisesRegex(ValueError, "full packages only"):
            release.validate_assets(self.directory, self.current, ["windows-x64"])

    def test_pre_rename_package_id_cannot_be_published_as_a_new_release(self):
        feed, asset = self.assets("linux-x64")
        asset["PackageId"] = "io.github.Ketchup3D.Ketchup"
        feed.write_text(json.dumps({"Assets": [asset]}))
        with self.assertRaisesRegex(ValueError, "identity"):
            release.validate_assets(self.directory, self.current, ["linux-x64"])

    def test_only_matching_stable_tags_are_accepted(self):
        self.assertEqual(release.version(self.tag), self.current)
        for invalid in ("latest", self.current, "v01.0.0", self.tag + "-beta.1", "v99999.0.0"):
            with self.subTest(tag=invalid), self.assertRaises(ValueError):
                release.version(invalid)

    def test_mismatched_packaging_tool_version_is_rejected(self):
        with patch.dict(os.environ, {"VPK_VERSION": "0.0.0"}):
            with self.assertRaisesRegex(ValueError, "workflow vpk version"):
                release.version(self.tag)

    def test_new_version_requires_notes_before_tagging(self):
        catalog = json.loads(release.RELEASE_NOTES.read_text())
        catalog[0]["version"] = "0.0.0"
        # A valid older catalog must still be rejected for the current version.
        catalog = catalog[:1]
        file = self.directory / "release-notes.json"
        file.write_text(json.dumps(catalog))
        with patch.object(release, "RELEASE_NOTES", file):
            with self.assertRaisesRegex(ValueError, "Prepare release notes"):
                release.version(self.tag)

    def test_missing_release_notes_file_prevents_tag_validation(self):
        with patch.object(release, "RELEASE_NOTES", self.directory / "missing.json"):
            with self.assertRaises(FileNotFoundError):
                release.version(self.tag)

    def test_release_catalog_rejects_invalid_or_empty_content(self):
        catalog = json.loads(release.RELEASE_NOTES.read_text())
        cases = [[], {}, [None], [catalog[0], catalog[0]], list(reversed(catalog))]
        for field, value in (
            ("version", "01.0.0"), ("version", None), ("date", "2026-02-30"),
            ("date", "20261025"), ("title", " "), ("sections", []),
            ("sections", [{"title": "Editor", "changes": []}]),
            ("sections", [{"title": "Editor", "changes": [" "]}]),
            ("sections", [{"title": "Editor", "changes": [42]}]),
            ("sections", [{"changes": ["A change"]}]),
        ):
            invalid = copy.deepcopy(catalog)
            invalid[0][field] = value
            cases.append(invalid)
        file = self.directory / "release-notes.json"
        with patch.object(release, "RELEASE_NOTES", file):
            for invalid in cases:
                with self.subTest(catalog=invalid):
                    file.write_text(json.dumps(invalid))
                    with self.assertRaises(ValueError):
                        release.version(self.tag)

    def test_notes_command_uses_the_same_catalog_as_the_app(self):
        catalog = release.release_notes(self.current)
        result = subprocess.run(
            [sys.executable, str(release.ROOT / "scripts/desktop_release.py"), "notes", "--tag", self.tag],
            check=True, capture_output=True, text=True,
        )
        self.assertEqual(result.stdout, release.format_notes(catalog[0]))
        self.assertIn(f"# BenchUp {self.current}", result.stdout)
        for section in catalog[0]["sections"]:
            self.assertIn(f"## {section['title']}", result.stdout)
            for change in section["changes"]:
                self.assertIn(f"- {change}", result.stdout)
        for older in catalog[1:]:
            self.assertNotIn(older["title"], result.stdout)

    def test_all_platforms_can_share_one_release_without_filename_collisions(self):
        for platform in release.PLATFORMS:
            self.assets(platform)
        release.validate_assets(self.directory, self.current, release.PLATFORMS)
        release.checksums(self.directory)
        first = (self.directory / "SHA256SUMS").read_bytes()
        release.checksums(self.directory)
        self.assertEqual(first, (self.directory / "SHA256SUMS").read_bytes())
        self.assertEqual(len(first.splitlines()), 3 * len(release.PLATFORMS) + 2)

    def test_missing_or_invalid_debian_package_prevents_publication(self):
        with self.assertRaises(FileNotFoundError):
            release.require_deb(self.directory, self.current)
        file = self.directory / release.deb_filename(self.current)
        for invalid in (b"", b"not a Debian package"):
            with self.subTest(contents=invalid):
                file.write_bytes(invalid)
                with self.assertRaisesRegex(ValueError, "Invalid Debian package"):
                    release.require_deb(self.directory, self.current)

    def test_missing_or_invalid_rpm_package_prevents_publication(self):
        with self.assertRaises(FileNotFoundError):
            release.require_rpm(self.directory, self.current)
        file = self.directory / release.rpm_filename(self.current)
        for invalid in (b"", b"not an RPM package"):
            with self.subTest(contents=invalid):
                file.write_bytes(invalid)
                with self.assertRaisesRegex(ValueError, "Invalid RPM package"):
                    release.require_rpm(self.directory, self.current)

    def test_publication_removes_only_packaging_metadata_before_checksumming(self):
        for platform, (channel, _, _, _) in release.PLATFORMS.items():
            self.assets(platform)
            (self.directory / f"assets.{channel}.json").write_text("{}")
            (self.directory / f"RELEASES-{channel}").write_text("legacy feed")
        (self.directory / "example-Portable.zip").write_bytes(b"portable")
        expected = {
            file.name for file in self.directory.iterdir()
            if not file.name.startswith(("assets.", "RELEASES-"))
        }
        expected.update({"benchup-linux-x64.deb", "benchup-linux-x64.rpm"})
        # Refresh stable downloads as well as their checksums on a draft rerun.
        for name in ("benchup-linux-x64.deb", "benchup-linux-x64.rpm"):
            (self.directory / name).write_bytes(b"stale package")
        # An existing checksum file must also be refreshed after pruning.
        (self.directory / "SHA256SUMS").write_text("stale metadata checksums")
        subprocess.run(
            [sys.executable, str(release.ROOT / "scripts/desktop_release.py"),
             "validate", "--tag", self.tag, "--output-dir", str(self.directory)],
            check=True, capture_output=True, text=True,
        )
        self.assertEqual({file.name for file in self.directory.iterdir()}, expected | {"SHA256SUMS"})
        for name, original in (
            ("benchup-linux-x64.deb", release.deb_filename(self.current)),
            ("benchup-linux-x64.rpm", release.rpm_filename(self.current)),
        ):
            self.assertEqual((self.directory / name).read_bytes(), (self.directory / original).read_bytes())
        lines = (self.directory / "SHA256SUMS").read_text().splitlines()
        self.assertEqual({line.split("  ", 1)[1] for line in lines}, expected)
        for line in lines:
            checksum, name = line.split("  ", 1)
            self.assertEqual(checksum, hashlib.sha256((self.directory / name).read_bytes()).hexdigest())

    def test_corrupt_package_prevents_publication(self):
        _, asset = self.assets("linux-x64")
        # Same length: this must exercise the checksum, not only the size gate.
        (self.directory / asset["FileName"]).write_bytes(b"x" * asset["Size"])
        with self.assertRaisesRegex(ValueError, "SHA256 mismatch"):
            release.validate_assets(self.directory, self.current, ["linux-x64"])

    def test_missing_platform_prevents_publication(self):
        self.assets("windows-x64")
        with self.assertRaises(FileNotFoundError):
            release.validate_assets(self.directory, self.current, release.PLATFORMS)

    def test_missing_macos_intel_feed_or_installer_prevents_publication(self):
        for platform in release.PLATFORMS:
            if platform != "macos-x64":
                self.assets(platform)
        with self.assertRaises(FileNotFoundError):
            release.validate_assets(self.directory, self.current, release.PLATFORMS)
        self.assets("macos-x64")
        (self.directory / f"{release.APP_ID}-osx-x64-Setup.pkg").unlink()
        with self.assertRaisesRegex(ValueError, "Missing installer.*osx-x64"):
            release.validate_assets(self.directory, self.current, release.PLATFORMS)

    def test_wrong_identity_duplicate_packages_and_unsafe_paths_are_rejected(self):
        for change in ("identity", "duplicate", "path", "size"):
            with self.subTest(change=change):
                feed, asset = self.assets("macos-arm64")
                if change == "identity":
                    asset["PackageId"] = "another-app"
                elif change == "path":
                    asset["FileName"] = "../outside.nupkg"
                elif change == "size":
                    asset["Size"] += 1
                assets = [asset, asset] if change == "duplicate" else [asset]
                feed.write_text(json.dumps({"Assets": assets}))
                with self.assertRaises(ValueError):
                    release.validate_assets(self.directory, self.current, ["macos-arm64"])

    def test_missing_installer_prevents_publication(self):
        self.assets("macos-arm64")
        for file in self.directory.glob("*.pkg"):
            file.unlink()
        with self.assertRaisesRegex(ValueError, "Missing installer"):
            release.validate_assets(self.directory, self.current, ["macos-arm64"])

    def test_required_signing_never_falls_back_to_unsigned_packages(self):
        for platform in ("windows-x64", "macos-arm64", "macos-x64"):
            with self.subTest(platform=platform), patch.dict(os.environ, {"BENCHUP_REQUIRE_SIGNING": "true"}, clear=True), patch.object(release.subprocess, "run") as run:
                (self.directory / release.PLATFORMS[platform][1]).touch()
                with self.assertRaisesRegex(ValueError, "signing is required"):
                    release.package(platform, self.tag, self.directory, self.directory / "dist")
                run.assert_not_called()

    def test_partial_signing_configuration_is_rejected(self):
        for platform, env in (
            ("windows-x64", {"AZURE_CREDENTIALS": "configured"}),
            ("macos-arm64", {"MACOS_SIGN_APP_IDENTITY": "configured"}),
            ("macos-x64", {"MACOS_SIGN_APP_IDENTITY": "configured"}),
        ):
            with self.subTest(platform=platform), patch.dict(os.environ, env, clear=True), patch.object(release.subprocess, "run") as run:
                (self.directory / release.PLATFORMS[platform][1]).touch()
                with self.assertRaises(ValueError):
                    release.package(platform, self.tag, self.directory, self.directory / "dist")
                run.assert_not_called()

    def test_both_macos_architectures_use_configured_signing_and_notarization(self):
        env = {
            "BENCHUP_REQUIRE_SIGNING": "true",
            "MACOS_SIGN_APP_IDENTITY": "Developer ID Application: BenchUp",
            "MACOS_SIGN_INSTALL_IDENTITY": "Developer ID Installer: BenchUp",
            "BENCHUP_MACOS_KEYCHAIN": "temporary.keychain-db",
            "BENCHUP_MACOS_NOTARY_PROFILE": "benchup-notary",
        }
        (self.directory / "benchup-app").touch()
        for platform in ("macos-arm64", "macos-x64"):
            with self.subTest(platform=platform), patch.dict(os.environ, env, clear=True), patch.object(release.subprocess, "run") as run, patch.object(release, "validate_assets"):
                release.package(platform, self.tag, self.directory, self.directory / "dist")
                args = run.call_args.args[0]
                for option, name in (
                    ("--signAppIdentity", "MACOS_SIGN_APP_IDENTITY"),
                    ("--signInstallIdentity", "MACOS_SIGN_INSTALL_IDENTITY"),
                    ("--keychain", "BENCHUP_MACOS_KEYCHAIN"),
                    ("--notaryProfile", "BENCHUP_MACOS_NOTARY_PROFILE"),
                ):
                    self.assertEqual(args[args.index(option) + 1], env[name])

    def test_windows_signing_file_is_removed_when_packaging_fails(self):
        captured = []

        def fail(args, **kwargs):
            path = Path(args[args.index("--azureTrustedSignFile") + 1])
            self.assertTrue(path.exists())
            captured.append(path)
            raise subprocess.CalledProcessError(1, args)

        config = {"Endpoint": "https://example.invalid", "CodeSigningAccountName": "account", "CertificateProfileName": "profile"}
        env = {"AZURE_CREDENTIALS": "configured", "WINDOWS_AZURE_SIGNING": json.dumps(config)}
        (self.directory / "benchup-app.exe").touch()
        with patch.dict(os.environ, env, clear=True), patch.object(release.subprocess, "run", side_effect=fail):
            with self.assertRaises(subprocess.CalledProcessError):
                release.package("windows-x64", self.tag, self.directory, self.directory / "dist")
        self.assertFalse(captured[0].exists())

    def test_partial_macos_certificate_configuration_fails_before_import(self):
        with patch.dict(os.environ, {"MACOS_CERTIFICATES_P12": "configured"}, clear=True), patch.object(macos_signing.subprocess, "run") as run:
            with self.assertRaisesRegex(SystemExit, "secrets are incomplete"):
                macos_signing.main()
            run.assert_not_called()

    def test_macos_import_failure_removes_certificate_and_hides_password_arguments(self):
        env = {name: "secret-value" for name in macos_signing.NAMES}
        env.update({
            "MACOS_CERTIFICATES_P12": base64.b64encode(b"fake-p12").decode(),
            "RUNNER_TEMP": str(self.directory),
            "GITHUB_ENV": str(self.directory / "environment"),
        })

        def fail_on_import(args, **kwargs):
            if args[1] == "import":
                raise subprocess.CalledProcessError(1, args)

        with patch.dict(os.environ, env, clear=True), patch.object(macos_signing.subprocess, "run", side_effect=fail_on_import), contextlib.redirect_stdout(io.StringIO()):
            with self.assertRaises(SystemExit) as error:
                macos_signing.main()
        self.assertNotIn("secret-value", str(error.exception))
        self.assertFalse((self.directory / "benchup-certificates.p12").exists())
        self.assertFalse((self.directory / "environment").exists())


if __name__ == "__main__":
    unittest.main()
