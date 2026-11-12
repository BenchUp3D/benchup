#!/usr/bin/env python3
"""Stage the landing page and discovery files after Trunk builds the editor."""
import argparse
from pathlib import Path
import shutil

DEFAULT_URL = "https://benchup3d.github.io/editor/"
FILES = ("index.html", "styles.css", "llms.txt", "robots.txt", "sitemap.xml")


def stage(source: Path, destination: Path) -> None:
    destination.mkdir(parents=True, exist_ok=True)
    for name in FILES:
        shutil.copy2(source / name, destination / name)
    for directory in ("branding", "screenshots"):
        shutil.copytree(source.parent / "assets" / directory,
                        destination / "assets" / directory, dirs_exist_ok=True)


if __name__ == "__main__":
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--dist", type=Path, default=Path("dist/site"))
    args = parser.parse_args()
    stage(Path(__file__).resolve().parents[1] / "site", args.dist)
