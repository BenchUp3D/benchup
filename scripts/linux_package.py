"""Shared payload staging for Linux packages; never compiles the executable."""
import os
import shutil

import desktop_release as release

DESKTOP_FILES = (
    "usr/bin/benchup-app",
    f"usr/share/applications/{release.APP_ID}.desktop",
    f"usr/share/mime/packages/{release.APP_ID}.xml",
    f"usr/share/icons/hicolor/256x256/apps/{release.APP_ID}.png",
    "usr/share/thumbnailers/benchup.thumbnailer",
)
VENDOR_LICENSES = (
    "LICENSE-miniz", "LICENSE-openskp", "LICENSE-earcut",
    "LICENSE-flatbuffers", "LICENSE-fflate",
)


def required_files(license_dir="usr/share/doc/benchup", license_name="copyright"):
    return (*DESKTOP_FILES, f"{license_dir}/{license_name}",
            "usr/share/doc/benchup/README.md",
            *(f"{license_dir}/licenses/skp/{name}" for name in VENDOR_LICENSES))


def validate_executable(executable):
    if not executable.is_file() or not os.access(executable, os.X_OK):
        raise ValueError(f"Missing staged Linux executable: {executable}.")
    with executable.open("rb") as stream:
        header = stream.read(20)
    if header[:6] != b"\x7fELF\x02\x01" or header[18:20] != b"\x3e\x00":
        raise ValueError("Linux packaging requires an x86-64 ELF executable.")


def stage(pack_dir, root, license_dir="usr/share/doc/benchup", license_name="copyright"):
    def copy(source, relative):
        target = root / relative
        target.parent.mkdir(parents=True, exist_ok=True)
        shutil.copyfile(source, target)
        target.chmod(0o644)

    copy(pack_dir / "benchup-app", "usr/bin/benchup-app")
    (root / "usr/bin/benchup-app").chmod(0o755)
    for extension, target in (("desktop", "applications"), ("xml", "mime/packages")):
        name = f"{release.APP_ID}.{extension}"
        copy(release.ROOT / "packaging/linux" / name, f"usr/share/{target}/{name}")
    for size in (16, 32, 48, 256, 512):
        copy(release.ROOT / f"assets/branding/icon-{size}.png",
             f"usr/share/icons/hicolor/{size}x{size}/apps/{release.APP_ID}.png")
    copy(release.ROOT / "assets/branding/logo.svg",
         f"usr/share/icons/hicolor/scalable/apps/{release.APP_ID}.svg")
    copy(pack_dir / "LICENSE", f"{license_dir}/{license_name}")
    copy(pack_dir / "README.md", "usr/share/doc/benchup/README.md")
    licenses = pack_dir / "licenses/skp"
    if any(not (licenses / name).is_file() for name in VENDOR_LICENSES):
        raise ValueError("Missing staged SketchUp reader licenses.")
    for source in sorted(licenses.glob("LICENSE*")):
        copy(source, f"{license_dir}/licenses/skp/{source.name}")
    thumbnailer = root / "usr/share/thumbnailers/benchup.thumbnailer"
    thumbnailer.parent.mkdir(parents=True, exist_ok=True)
    thumbnailer.write_text(
        "[Thumbnailer Entry]\nExec=benchup-app --thumbnail %i %o\n"
        "MimeType=application/x-benchup;\n", encoding="utf-8",
    )
    thumbnailer.chmod(0o644)
    for directory in (root, *[path for path in root.rglob("*") if path.is_dir()]):
        directory.chmod(0o755)
