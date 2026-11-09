"""Regenerate synthetic update packages; these markers are never executed."""
from pathlib import Path
import hashlib
import json
import zipfile

ROOT = Path(__file__).resolve().parent
ID = "io.github.BenchUp3D.BenchUp"
for channel in ["win-x64", "osx-arm64", "linux-x64"]:
    executable = "benchup-app.exe" if channel == "win-x64" else "benchup-app"
    name = f"{ID}-0.6.6-{channel}-full.nupkg"
    manifest = (f"<package><metadata><id>{ID}</id><version>0.6.6</version>"
                f"<title>BenchUp</title><channel>{channel}</channel>"
                f"<mainExe>{executable}</mainExe></metadata></package>")
    with zipfile.ZipFile(ROOT / name, "w", compression=zipfile.ZIP_STORED) as archive:
        for path, data in [(ID + ".nuspec", manifest),
                           ("lib/app/" + executable, "Synthetic application marker; never execute."),
                           ("lib/app/Squirrel.exe", "Synthetic updater marker; never execute.")]:
            entry = zipfile.ZipInfo(path, date_time=(2026, 1, 1, 0, 0, 0))
            archive.writestr(entry, data)
    data = (ROOT / name).read_bytes()
    feed = {"Assets": [{"PackageId": ID, "Version": "0.6.6", "Type": "Full",
                        "FileName": name, "Size": len(data),
                        "SHA1": hashlib.sha1(data).hexdigest(),
                        "SHA256": hashlib.sha256(data).hexdigest()}]}
    (ROOT / f"releases.{channel}.json").write_text(json.dumps(feed, indent=2) + "\n")
