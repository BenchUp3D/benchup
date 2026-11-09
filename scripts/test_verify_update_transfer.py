"""Offline checks for the release gate protecting already-installed updaters."""
import unittest
from urllib.error import HTTPError
from unittest.mock import Mock

import verify_update_transfer as migration


class TransferTests(unittest.TestCase):
    def fetch(self, old=None, new=None, old_releases=None, new_releases=None):
        repo = {"id": 1234, "full_name": migration.CURRENT}
        return Mock(side_effect=[
            repo if old is None else old, repo if new is None else new,
            [{"id": 42}] if old_releases is None else old_releases,
            [{"id": 42}] if new_releases is None else new_releases,
        ])

    def test_transferred_repository_retains_numeric_identity_and_public_release_access(self):
        for source in [migration.LEGACY, migration.CURRENT, "BenchUp3D/app", "Ketchup3D/editor"]:
            fetch = self.fetch()
            self.assertIn("Verified", migration.verify(source, fetch))
            self.assertEqual(fetch.call_args_list[2].args[0],
                             migration.API + migration.LEGACY + "/releases?per_page=10&page=1")
        self.assertIn("Verified", migration.verify(migration.CURRENT, self.fetch(old_releases=[], new_releases=[])))

    def test_recreated_old_repository_or_wrong_transfer_blocks_release(self):
        for old in [{"id": 999, "full_name": migration.LEGACY},
                    {"id": 1234, "full_name": "someone/else"}, {}, []]:
            with self.subTest(old=old), self.assertRaisesRegex(ValueError, "same repository"):
                migration.verify(migration.CURRENT, self.fetch(old=old))

    def test_lost_release_redirect_invalid_metadata_and_different_releases_block_release(self):
        for old_releases in [[], {}, [{"id": 99}], [{}]]:
            with self.subTest(releases=old_releases), self.assertRaises(ValueError):
                migration.verify(migration.CURRENT, self.fetch(old_releases=old_releases))
        error = HTTPError("url", 404, "missing", {}, None)
        self.addCleanup(error.close)
        with self.assertRaises(HTTPError):
            migration.verify(migration.CURRENT, Mock(side_effect=error))

    def test_forks_do_not_depend_on_the_official_transfer(self):
        fetch = Mock()
        self.assertIn("Skipped", migration.verify("someone/editor", fetch))
        fetch.assert_not_called()
        with self.assertRaises(ValueError):
            migration.verify("../invalid/repository", fetch)
