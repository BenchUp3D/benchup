#!/usr/bin/env python3
"""Package the existing Linux release executable as a Debian amd64 package.

No Cargo invocation, root privileges, downloaded tools, or additional compilation.
"""
import argparse
from pathlib import Path
import subprocess
import tempfile

import desktop_release as release
import linux_package

# winit/wgpu load these through dlopen; dpkg-shlibdeps sees only linked libraries.
RUNTIME_DEPENDENCIES = (
    "libegl1", "libgl1", "libvulkan1", "libwayland-client0",
    "libwayland-cursor0", "libwayland-egl1", "libx11-6", "libx11-xcb1", "libxcb1",
    "libxcursor1", "libxi6", "libxkbcommon0", "libxkbcommon-x11-0", "libxrandr2",
)
REQUIRED_FILES = linux_package.required_files()


def linked_dependencies(executable, work_dir):
    debian = work_dir / "debian"
    debian.mkdir()
    (debian / "control").write_text(
        "Source: benchup\n\nPackage: benchup\nArchitecture: amd64\n",
        encoding="utf-8",
    )
    result = subprocess.run(
        ["dpkg-shlibdeps", "-O", f"-e{executable.resolve()}"],
        cwd=work_dir, check=True, stdout=subprocess.PIPE, text=True,
    )
    for line in result.stdout.splitlines():
        if line.startswith("shlibs:Depends="):
            dependencies = line.removeprefix("shlibs:Depends=").strip()
            if dependencies:
                return dependencies
    raise ValueError("dpkg-shlibdeps did not report runtime dependencies.")


def stage(pack_dir, root, current, linked):
    linux_package.stage(pack_dir, root)
    installed_size = sum(file.stat().st_size for file in root.rglob("*") if file.is_file())
    control = root / "DEBIAN/control"
    control.parent.mkdir()
    control.write_text(
        f"Package: benchup\nVersion: {current}\nArchitecture: amd64\n"
        "Section: graphics\nPriority: optional\nMaintainer: BenchUp contributors\n"
        f"Installed-Size: {(installed_size + 1023) // 1024}\n"
        f"Depends: {linked}, {', '.join(RUNTIME_DEPENDENCIES)}\n"
        "Recommends: ca-certificates, shared-mime-info, desktop-file-utils\n"
        "Homepage: https://benchup3d.github.io/editor/\n"
        "Description: 3D modeling for woodworking and furniture design\n"
        " Plan boards, wood joints, and complete assemblies with exact measurements.\n",
        encoding="utf-8",
    )
    control.parent.chmod(0o755)
    control.chmod(0o644)


def validate(directory, current):
    file = release.require_deb(directory, current)
    result = subprocess.run(
        ["dpkg-deb", "--showformat=${Package}\n${Version}\n${Architecture}\n${Depends}\n",
         "--show", str(file)], check=True, stdout=subprocess.PIPE, text=True,
    )
    fields = result.stdout.splitlines()
    if len(fields) != 4 or fields[:3] != ["benchup", current, "amd64"] or not fields[3].strip():
        raise ValueError(f"Unexpected Debian package metadata: {file.name}.")
    result = subprocess.run(
        ["dpkg-deb", "--contents", str(file)],
        check=True, stdout=subprocess.PIPE, text=True,
    )
    entries = {line.split(maxsplit=5)[-1]: line for line in result.stdout.splitlines()}
    for relative in REQUIRED_FILES:
        entry = entries.get(f"./{relative}", "")
        mode = "-rwxr-xr-x" if relative == "usr/bin/benchup-app" else "-rw-r--r--"
        if not entry.startswith(f"{mode} root/root "):
            raise ValueError(f"Missing Debian package file or incorrect permissions: {relative}.")


def package(pack_dir, output_dir, current):
    executable = pack_dir / "benchup-app"
    linux_package.validate_executable(executable)
    output_dir.mkdir(parents=True, exist_ok=True)
    with tempfile.TemporaryDirectory(prefix="benchup-deb-") as temporary:
        work_dir = Path(temporary)
        linked = linked_dependencies(executable, work_dir)
        root = work_dir / "package"
        stage(pack_dir, root, current, linked)
        subprocess.run(
            ["desktop-file-validate", str(root / REQUIRED_FILES[1])], check=True,
        )
        # gzip level 1 keeps packaging fast and supports older dpkg versions.
        subprocess.run(
            ["dpkg-deb", "--root-owner-group", "-Zgzip", "-z1", "--build",
             str(root), str((output_dir / release.deb_filename(current)).resolve())],
            check=True,
        )
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
        parser.exit(1, f"Debian packaging failed: {error}\n")


if __name__ == "__main__":
    main()
