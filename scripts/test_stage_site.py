import json
import os
from pathlib import Path
import tempfile
import unittest
from unittest.mock import patch
import xml.etree.ElementTree as ET

from stage_site import DEFAULT_URL, FILES, stage


class StageSiteTests(unittest.TestCase):
    def test_staging_preserves_links_regardless_of_repository_and_pages_configuration(self):
        source = Path(__file__).resolve().parents[1] / 'site'
        with tempfile.TemporaryDirectory() as directory, patch.dict(os.environ, {
            'GITHUB_REPOSITORY': 'someone/wood-editor',
            'PAGES_BASE_URL': 'https://fork.example.org/',
        }):
            target = Path(directory)
            (target / 'play').mkdir()
            (target / 'play/index.html').write_text('existing editor')
            stage(source, target)
            for name in FILES:
                self.assertEqual((target / name).read_bytes(), (source / name).read_bytes())
            page = (target / 'index.html').read_text()
            data = json.loads(page.split('<script type="application/ld+json">')[1].split('</script>')[0])
            self.assertEqual(data['url'], DEFAULT_URL)
            self.assertIn(f'href="{DEFAULT_URL}"', page)
            self.assertIn('https://github.com/BenchUp3D/editor/releases/latest', page)
            locations = ET.parse(target / 'sitemap.xml').findall('{*}url/{*}loc')
            self.assertEqual([node.text for node in locations], [DEFAULT_URL])
            self.assertIn(DEFAULT_URL + 'sitemap.xml', (target / 'robots.txt').read_text())
            self.assertIn(f'({DEFAULT_URL})', (target / 'llms.txt').read_text())
            self.assertTrue((target / 'assets/screenshots/benchup.jpg').is_file())
            self.assertEqual((target / 'play/index.html').read_text(), 'existing editor')


if __name__ == '__main__':
    unittest.main()
