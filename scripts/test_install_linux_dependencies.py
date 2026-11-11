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
