import tempfile
import unittest
from pathlib import Path
from unittest.mock import Mock, call, patch

import install_linux_dependencies as deps


class LinuxDependenciesTests(unittest.TestCase):
    def test_rewrites_both_source_formats_preserving_other_repositories(self):
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            (root / 'sources.list.d').mkdir()
            files = [root / 'sources.list', root / 'sources.list.d/ubuntu.sources',
                     root / 'sources.list.d/extra.list']
            for path in files:
                path.write_text('URIs: http://azure.archive.ubuntu.com/ubuntu/\n'
                                'https://security.ubuntu.com/ubuntu noble-security\n')
            self.assertTrue(deps.use_fallback_mirror(root))
            for path in files:
                self.assertEqual(path.read_text(),
                                 'URIs: http://archive.ubuntu.com/ubuntu/\n'
                                 'https://security.ubuntu.com/ubuntu noble-security\n')
            self.assertFalse(deps.use_fallback_mirror(root))

    def test_runner_mirror_list_sources_use_archive_directly(self):
        for uri in ('mirror+file:/etc/apt/apt-mirrors.txt',
                    'mirror+file:///etc/apt/apt-mirrors.txt'):
            with self.subTest(uri=uri), tempfile.TemporaryDirectory() as directory:
                root = Path(directory)
                (root / 'sources.list.d').mkdir()
                legacy = root / 'sources.list'
                legacy.write_text(
                    f'deb [arch=amd64] {uri} jammy main restricted\n'
                    f'deb {uri} jammy-updates main universe\n'
                    f'deb {uri} jammy-backports main universe\n'
                    f'deb {uri} jammy-security main universe\n')
                deb822 = root / 'sources.list.d/ubuntu.sources'
                deb822.write_text(
                    'Types: deb deb-src\n'
                    f'URIs: {uri}\n'
                    'Suites: noble noble-updates noble-backports noble-security\n'
                    'Components: main restricted universe multiverse\n'
                    'Signed-By: /usr/share/keyrings/ubuntu-archive-keyring.gpg\n')
                originals = {path: path.read_text() for path in (legacy, deb822)}
                extra = root / 'sources.list.d/microsoft-prod.list'
                third_party = ('deb [arch=amd64 signed-by=/usr/share/keyrings/microsoft.gpg] '
                               'https://packages.microsoft.com/ubuntu/22.04/prod '
                               'jammy main\n')
                extra.write_text(third_party)
                mirror_list = root / 'apt-mirrors.txt'
                mirrors = ('http://azure.archive.ubuntu.com/ubuntu/\n'
                           'https://archive.ubuntu.com/ubuntu/\n'
                           'https://security.ubuntu.com/ubuntu/\n')
                mirror_list.write_text(mirrors)

                self.assertTrue(deps.use_fallback_mirror(root))
                for path, original in originals.items():
                    self.assertEqual(path.read_text(), original.replace(
                        uri, 'https://archive.ubuntu.com/ubuntu'))
                self.assertEqual(extra.read_text(), third_party)
                self.assertEqual(mirror_list.read_text(), mirrors)
                self.assertFalse(deps.use_fallback_mirror(root))

    def test_unrelated_sources_and_unreferenced_mirror_list_are_unchanged(self):
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            source = root / 'sources.list'
            original = ('deb https://archive.ubuntu.com/ubuntu jammy main\n'
                        'deb https://security.ubuntu.com/ubuntu jammy-security main\n'
                        'deb mirror+file:/etc/apt/custom-mirrors.txt jammy main\n')
            source.write_text(original)
            mirror_list = root / 'apt-mirrors.txt'
            mirrors = 'http://azure.archive.ubuntu.com/ubuntu/\n'
            mirror_list.write_text(mirrors)

            self.assertFalse(deps.use_fallback_mirror(root))
            self.assertEqual(source.read_text(), original)
            self.assertEqual(mirror_list.read_text(), mirrors)

    def test_missing_sources_do_not_trigger_fallback(self):
        with tempfile.TemporaryDirectory() as directory:
            self.assertFalse(deps.use_fallback_mirror(Path(directory)))

    @patch.object(deps.subprocess, 'run')
    def test_runner_failure_rewrites_sources_then_refreshes_and_installs(self, run):
        for failure in ('update', 'install'):
            with self.subTest(failure=failure), tempfile.TemporaryDirectory() as directory:
                root = Path(directory)
                source = root / 'sources.list'
                source.write_text('deb mirror+file:/etc/apt/apt-mirrors.txt jammy main\n')
                commands = []

                def apt_attempt(command, check):
                    commands.append(command)
                    is_first_attempt = 'mirror+file:' in source.read_text()
                    if is_first_attempt:
                        return Mock(returncode=100 if command[1 + len(deps.APT_OPTIONS)]
                                    == failure else 0)
                    self.assertEqual(source.read_text(),
                                     'deb https://archive.ubuntu.com/ubuntu jammy main\n')
                    return Mock(returncode=0)

                run.side_effect = apt_attempt
                fallback = deps.use_fallback_mirror
                with patch.object(deps, 'use_fallback_mirror',
                                  side_effect=lambda: fallback(root)):
                    self.assertEqual(deps.main(['libudev-dev']), 0)

                update = ['apt-get', *deps.APT_OPTIONS, 'update']
                install = ['apt-get', *deps.APT_OPTIONS, 'install',
                           '--no-install-recommends', '-y', 'libudev-dev']
                first_attempt = [update] if failure == 'update' else [update, install]
                self.assertEqual(commands, [*first_attempt, update, install])

    @patch.object(deps, 'use_fallback_mirror')
    @patch.object(deps, 'install', return_value=0)
    def test_success_does_not_change_mirrors(self, install, fallback):
        self.assertEqual(deps.main(['pkg-config']), 0)
        fallback.assert_not_called()
        install.assert_called_once_with(['pkg-config'])

    @patch.object(deps, 'use_fallback_mirror', return_value=True)
    @patch.object(deps, 'install', side_effect=[100, 0])
    def test_failure_retries_packages_on_fallback(self, install, fallback):
        self.assertEqual(deps.main(['libudev-dev']), 0)
        self.assertEqual(install.call_args_list, [call(['libudev-dev'])] * 2)
        fallback.assert_called_once_with()

    @patch.object(deps, 'use_fallback_mirror', return_value=True)
    @patch.object(deps, 'install', side_effect=[100, 42])
    def test_fallback_failure_is_propagated(self, install, fallback):
        self.assertEqual(deps.main(['pkg']), 42)

    @patch.object(deps, 'use_fallback_mirror', return_value=False)
    @patch.object(deps, 'install', return_value=100)
    def test_unrelated_failure_is_preserved(self, install, fallback):
        self.assertEqual(deps.main(['pkg']), 100)
        install.assert_called_once()

    @patch.object(deps.subprocess, 'run')
    def test_update_failure_stops_before_install(self, run):
        run.return_value = Mock(returncode=100)
        self.assertEqual(deps.install(['pkg']), 100)
        run.assert_called_once_with(['apt-get', *deps.APT_OPTIONS, 'update'], check=False)

    @patch.object(deps.subprocess, 'run')
    def test_install_has_short_timeouts_and_propagates_download_failure(self, run):
        run.side_effect = [Mock(returncode=0), Mock(returncode=100)]
        self.assertEqual(deps.install(['pkg']), 100)
        self.assertEqual(run.call_args_list, [
            call(['apt-get', *deps.APT_OPTIONS, 'update'], check=False),
            call(['apt-get', *deps.APT_OPTIONS, 'install',
                  '--no-install-recommends', '-y', 'pkg'], check=False),
        ])
        self.assertIn('Acquire::http::Timeout=10', deps.APT_OPTIONS)
        self.assertIn('Acquire::https::Timeout=10', deps.APT_OPTIONS)
        self.assertIn('Acquire::Retries=0', deps.APT_OPTIONS)
        self.assertIn('APT::Update::Error-Mode=any', deps.APT_OPTIONS)


if __name__ == '__main__':
    unittest.main()
