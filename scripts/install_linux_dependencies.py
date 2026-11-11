#!/usr/bin/env python3
"""Install CI dependencies with a bounded Azure mirror attempt and fallback."""

from pathlib import Path
import subprocess
import sys

APT_OPTIONS = [
    '-o', 'Acquire::http::Timeout=10',
    '-o', 'Acquire::https::Timeout=10',
    '-o', 'Acquire::Retries=0',
    # Otherwise apt can report success after failing to refresh some indexes.
    '-o', 'APT::Update::Error-Mode=any',
]


def use_fallback_mirror(root=Path('/etc/apt')):
    """Handle both legacy source lists and Ubuntu's deb822 source format."""
    sources = [root / 'sources.list']
    sources += sorted((root / 'sources.list.d').glob('*.list'))
    sources += sorted((root / 'sources.list.d').glob('*.sources'))
    changed = False
    for path in sources:
        if not path.is_file():
            continue
        original = path.read_text()
        fallback = original.replace('azure.archive.ubuntu.com', 'archive.ubuntu.com')
        if fallback != original:
            path.write_text(fallback)
            changed = True
    return changed


def install(packages):
    for command in (
        ['update'],
        ['install', '--no-install-recommends', '-y', *packages],
    ):
        result = subprocess.run(['apt-get', *APT_OPTIONS, *command], check=False)
        if result.returncode:
            return result.returncode
    return 0


def main(packages):
    result = install(packages)
    if not result:
        return 0
    if not use_fallback_mirror():
        print('No Azure Ubuntu mirror configured; preserving apt failure.', flush=True)
        return result
    print('::warning::Azure apt attempt failed; retrying with archive.ubuntu.com', flush=True)
    # Refresh indexes again, including when the initial failure was a download.
    return install(packages)


if __name__ == '__main__':
    if not sys.argv[1:]:
        sys.exit('Usage: sudo python3 scripts/install_linux_dependencies.py PACKAGE ...')
    sys.exit(main(sys.argv[1:]))
