#!/usr/bin/env python3
"""Verify the public URLs embedded in KetchUp 0.6.5 still reach this repository.

Run before publishing an official BenchUp release. This does not modify GitHub.
"""
import argparse
import json
import re
import urllib.error
import urllib.request

CURRENT = "BenchUp3D/editor"
LEGACY = "Ketchup3D/app"
API = "https://api.github.com/repos/"


def fetch_json(url):
    request = urllib.request.Request(url, headers={
        "Accept": "application/vnd.github+json",
        "User-Agent": "BenchUp-update-migration-check",
    })
    # Public, unauthenticated requests intentionally match installed 0.6.5 clients.
    # urllib follows GitHub's transfer/rename redirects, including numeric repo URLs.
    with urllib.request.urlopen(request, timeout=20) as response:
        payload = response.read(2 * 1024 * 1024 + 1)
    if len(payload) > 2 * 1024 * 1024:
        raise ValueError("GitHub migration response is unexpectedly large")
    return json.loads(payload)


def verify(repository, fetch=fetch_json):
    if not re.fullmatch(r"[A-Za-z0-9_.-]+/[A-Za-z0-9_.-]+", repository):
        raise ValueError("Repository must be owner/repository")
    owner, name = repository.lower().split("/")
    if owner not in ("ketchup3d", "benchup3d") or name not in ("app", "editor"):
        return "Skipped official KetchUp transfer check for a fork."
    old = fetch(API + LEGACY)
    new = fetch(API + CURRENT)
    if (not isinstance(old, dict) or not isinstance(new, dict)
            or not isinstance(new.get("id"), int) or old.get("id") != new["id"]
            or old.get("full_name", "").lower() != CURRENT.lower()
            or new.get("full_name", "").lower() != CURRENT.lower()):
        raise ValueError(f"{LEGACY} must redirect to the same repository as {CURRENT}. "
                         "Transfer/rename the original repository; do not recreate its old location.")
    query = "/releases?per_page=10&page=1"  # Exact path used by the 0.6.5 GithubSource.
    old_releases = fetch(API + LEGACY + query)
    new_releases = fetch(API + CURRENT + query)
    if not isinstance(old_releases, list) or not isinstance(new_releases, list):
        raise ValueError("The old and new public releases endpoints must return release lists")
    def release_ids(releases):
        ids = []
        for release in releases:
            if not isinstance(release, dict) or not isinstance(release.get("id"), int):
                raise ValueError("Invalid public release metadata")
            ids.append(release["id"])
        return set(ids)
    if release_ids(old_releases) != release_ids(new_releases):
        raise ValueError("The old releases endpoint does not expose the current repository's releases")
    return f"Verified public updater migration: {LEGACY} -> {CURRENT} (repository {new['id']})."


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--repository", required=True)
    args = parser.parse_args()
    try:
        print(verify(args.repository))
    except (ValueError, OSError, urllib.error.URLError) as error:
        parser.exit(1, f"Update migration verification failed: {error}\n")


if __name__ == "__main__":
    main()
