#!/usr/bin/env python3
"""Package the existing Linux release executable as a Fedora x86_64 RPM."""
import argparse
from pathlib import Path
import shutil
import subprocess
import tarfile
import tempfile

import desktop_release as release
import linux_package

LICENSE_DIR = "usr/share/licenses/benchup"
REQUIRED_FILES = linux_package.required_files(LICENSE_DIR, "LICENSE")


def query(file, *arguments):
    return subprocess.run(
        ["rpm", "-qp", *arguments, str(file)],
        check=True, stdout=subprocess.PIPE, text=True,
    ).stdout


def validate(directory, current):
    file = release.require_rpm(directory, current)
    subprocess.run(["rpm", "--checksig", "--nosignature", str(file)], check=True)
    fields = query(file, "--queryformat", "%{NAME}\n%{VERSION}\n%{RELEASE}\n%{ARCH}\n").splitlines()
    if fields != ["benchup", current, "1", "x86_64"]:
        raise ValueError(f"Unexpected RPM package metadata: {file.name}.")
    dependencies = query(file, "--requires").splitlines()
    # Check automatic symbol-version detection as well as dlopen dependencies.
    spec = (release.ROOT / "packaging/linux/benchup.spec").read_text(encoding="utf-8")
    required = [line.removeprefix("Requires: ") for line in spec.splitlines()
                if line.startswith("Requires: ")]
    if not set(required).issubset(dependencies) or not any(
        line.startswith("libc.so.6(GLIBC_") for line in dependencies
    ):
        raise ValueError("RPM package is missing runtime dependencies.")
    contents = query(file, "--queryformat",
                     "[%{FILENAMES}\t%{FILEMODES:perms}\t%{FILEUSERNAME}\t%{FILEGROUPNAME}\n]")
    entries = dict(line.split("\t", 1) for line in contents.splitlines())
    for relative in REQUIRED_FILES:
        mode = "-rwxr-xr-x" if relative == "usr/bin/benchup-app" else "-rw-r--r--"
        if entries.get(f"/{relative}") != f"{mode}\troot\troot":
            raise ValueError(f"Missing RPM package file or incorrect permissions: {relative}.")


def package(pack_dir, output_dir, current):
    linux_package.validate_executable(pack_dir / "benchup-app")
    output_dir.mkdir(parents=True, exist_ok=True)
    with tempfile.TemporaryDirectory(prefix="benchup-rpm-") as temporary:
        work_dir = Path(temporary)
        payload = work_dir / "payload"
        linux_package.stage(pack_dir, payload, LICENSE_DIR, "LICENSE")
        subprocess.run(
            ["desktop-file-validate", str(payload / linux_package.DESKTOP_FILES[1])], check=True,
        )
        for directory in ("BUILD", "BUILDROOT", "RPMS", "SOURCES", "SPECS", "SRPMS"):
            (work_dir / directory).mkdir()
        # The input archive is uncompressed; compress only the final RPM payload.
        with tarfile.open(work_dir / "SOURCES/payload.tar", "w") as archive:
            archive.add(payload / "usr", arcname="usr")
        spec = work_dir / "SPECS/benchup.spec"
        shutil.copyfile(release.ROOT / "packaging/linux/benchup.spec", spec)
        subprocess.run(
            ["rpmbuild", "-bb", "--define", f"_topdir {work_dir}",
             "--define", f"benchup_version {current}", str(spec)], check=True,
        )
        file = work_dir / "RPMS/x86_64" / release.rpm_filename(current)
        if not file.is_file():
            raise ValueError(f"Missing packaged RPM: {file.name}.")
        shutil.copyfile(file, output_dir / file.name)
    validate(output_dir, current)


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("command", choices=("package", "validate"))
    parser.add_argument("--tag", required=True)
    parser.add_argument("--pack-dir", type=Path, default=Path("package"))
    parser.add_argument("--output-dir", type=Path, default=Path("dist"))
    args = parser.parse_args()
    try:
        current = release.version(args.tag)
        if args.command == "package":
            package(args.pack_dir, args.output_dir, current)
        else:
            validate(args.output_dir, current)
    except (ValueError, OSError, KeyError, subprocess.CalledProcessError) as error:
        parser.exit(1, f"RPM packaging failed: {error}\n")


if __name__ == "__main__":
    main()
