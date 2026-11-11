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
    """Switch direct Azure sources and runner mirror lists to the Ubuntu archive."""
    sources = [root / 'sources.list']
    sources += sorted((root / 'sources.list.d').glob('*.list'))
    sources += sorted((root / 'sources.list.d').glob('*.sources'))
    changed = False
    for path in sources:
        if not path.is_file():
            continue
        original = path.read_text()
        fallback = original.replace('azure.archive.ubuntu.com', 'archive.ubuntu.com')
        # Hosted runners can put the Azure hostname in apt-mirrors.txt instead
        # of these source files. Bypass that list on retry so a fresh apt process
        # uses the archive directly, without selecting the failed Azure mirror.
        for uri in ('mirror+file:/etc/apt/apt-mirrors.txt',
                    'mirror+file:///etc/apt/apt-mirrors.txt'):
            fallback = fallback.replace(uri, 'https://archive.ubuntu.com/ubuntu')
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
        print('No Azure Ubuntu mirror or runner mirror list configured; '
              'preserving apt failure.', flush=True)
        return result
    print('::warning::Ubuntu apt attempt failed; retrying with direct '
          'archive.ubuntu.com sources', flush=True)
    # Refresh indexes again, including when the initial failure was a download.
    return install(packages)


if __name__ == '__main__':
    if not sys.argv[1:]:
        sys.exit('Usage: sudo python3 scripts/install_linux_dependencies.py PACKAGE ...')
    sys.exit(main(sys.argv[1:]))
