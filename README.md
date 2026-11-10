# BenchUp

![BenchUp editor screenshot](assets/screenshots/benchup.jpg "BenchUp v0.6.7 desktop workspace")

[![Try BenchUp in your browser](https://img.shields.io/badge/Try_BenchUp_in_your_browser-d63b32?style=for-the-badge)](https://benchup3d.github.io/editor/play/)

## Download for desktop

[![Download for Windows x64](https://img.shields.io/badge/Windows_x64-0078D4?style=for-the-badge)](https://github.com/BenchUp3D/editor/releases/latest/download/io.github.BenchUp3D.BenchUp-win-x64-Setup.exe)
[![Download for macOS Apple Silicon](https://img.shields.io/badge/macOS_Apple_Silicon-6E40C9?style=for-the-badge)](https://github.com/BenchUp3D/editor/releases/latest/download/io.github.BenchUp3D.BenchUp-osx-arm64-Setup.pkg)
[![Download for Linux x64](https://img.shields.io/badge/Linux_x64-E95420?style=for-the-badge)](https://github.com/BenchUp3D/editor/releases/latest/download/io.github.BenchUp3D.BenchUp-linux-x64.AppImage)
[![See all downloads](https://img.shields.io/badge/All_downloads-555555?style=for-the-badge)](https://github.com/BenchUp3D/editor/releases/latest)

A free, open-source 3D modeling app for woodworking and furniture design, written in Rust. Plan individual boards, wood joints, and complete assemblies with exact measurements before heading to the workshop. The same editor runs as a native Windows, macOS, or Linux application and in a desktop browser through WebAssembly.

The workspace includes Select, Line, Rectangle, Circle, Polygon, Freehand, Push/Pull, Scale, Offset, Move, Rotate, Paint, Tape Measure, Protractor, Dimensions, a Union flyout, Erase, and Orbit, automatic geometry snapping, filled planar faces, measured lines, full-enclosure box selection, groups and components, a right-click object menu, geometry clipboard, undo/redo, local autosave, and portable `.bup` project files, with support for legacy `.k3d` and `.ketchup` projects. It starts with an empty model. File → Import/Export adds SketchUp `.skp` import and Wavefront `.obj` export. Mobile and cloud storage are outside this version.

## Woodworking features

Model shelves, tables, cabinets, workbenches, and other furniture with precisely sized parts. Use groups, reusable components, and tags to keep the assembly organized, then inspect dimensions and fit from any angle.

### Wood joinery

Create mortise-and-tenon, half-lap, tongue-and-groove, rabbet, dovetail, and finger joints between boards, including angled connections. Adjust joint dimensions and clearance with live previews, inspect machining diagnostics and cut guides, and trim excess material past a joint while keeping the original boards editable.

![Finger joint between two selected boards, showing the interlocking geometry and grain-direction guides](assets/screenshots/wood-joinery.png)

### Wood grain orientation

Set the grain direction on groups and components and show grain-direction guides directly on the model. Review how the grain runs across each board and through the full assembly before building. Grain orientation is saved with the project and follows the parts when they rotate.

### Plan materials and workshop work

Assign stock materials and wood types to parts, configure sanding and finishing products, and prepare cut lists, cutting plans, and workshop labels. Export cutting plans and time and cost estimates to PDF to take the design into the workshop.

## Documentation

This README covers developer installation, building, verification, and deployment.
For instructions on using BenchUp, see the [user documentation](docs/README.md).

## Prerequisites

- Rust 1.95 or newer, installed with rustup; the repository was checked with Rust 1.97.1.
- A C/C++17 compiler for the native SketchUp reader (GCC/Clang on Linux, Xcode Clang on macOS, or MSVC on Windows). Browser builds do not require it.
- A GPU/driver supported by wgpu. The browser prefers WebGPU and falls back to WebGL2.
- On Linux, development libraries for X11/Wayland, xkbcommon, and libudev. For Debian/Ubuntu: `sudo apt install build-essential pkg-config libx11-dev libxcb1-dev libxkbcommon-dev libwayland-dev libegl1-mesa-dev libudev-dev`.
- On Windows, the MSVC Rust toolchain and Visual Studio Build Tools with the **Desktop development with C++** workload and a Windows SDK. SpaceMouse support uses Windows system APIs; no separate HIDAPI or libusb installation is required.
- On macOS, Xcode Command Line Tools (`xcode-select --install`), including Clang and the macOS SDK. SpaceMouse support uses the system IOKit, CoreFoundation, and AppKit frameworks; no Homebrew HIDAPI or libusb package is required.

Linux desktop builds use libudev through the `hidapi` dependency to discover SpaceMouse devices. The `libudev-dev` package supplies the headers and `pkg-config` metadata needed to compile this support, even if no SpaceMouse is connected. If compilation fails with `Unable to find libudev` or `Package 'libudev' not found`, install `libudev-dev` and retry. Browser builds do not use `hidapi`. See the upstream [HIDAPI build prerequisites](https://github.com/libusb/hidapi/blob/master/BUILD.md#prerequisites).

## Run on desktop

```sh
cargo run -p benchup-app
```

On Linux, register the application launcher and icons once after compiling:

```sh
python3 scripts/install_linux_desktop.py
```

This installs into your user data directory (`$XDG_DATA_HOME`, or `~/.local/share`) without root access. GNOME on Wayland uses the application's ID and matching desktop entry to find its icon. Restart BenchUp after installing. The launcher points to `target/debug/benchup-app`, so later development compilations update the same executable. For a release installation, pass its path: `python3 scripts/install_linux_desktop.py target/release/benchup-app`. Keep that executable in place, or rerun the installer with its new path if you move it.

## Run in a browser

Install the browser target and Trunk once:

```sh
rustup target add wasm32-unknown-unknown
cargo install --locked trunk
```

From the repository root:

```sh
trunk serve
```

Open <http://127.0.0.1:8080>. Keep the app on the same origin/port to restore the same saved model. No Node.js, JavaScript application framework, or backend is required. Trunk generates the JavaScript needed to load the Rust WebAssembly program.

Development builds use light optimization for application code and stronger optimization for dependencies and the geometry core, on both desktop and WebAssembly. Debug symbols, debug assertions, and overflow checks remain enabled. These optimizations target the cost of menu hovering, scrolling, and rendering in debug mode. The profile takes effect on the next development compilation; changing it recompiles affected dependencies once.

## Application messages

The English catalog is [assets/i18n/en.json](assets/i18n/en.json). Use
`tr!("area.message")` for a label and `tr!("area.message", name = name)` for a
template with a named variable. Keep the key stable when editing the text.
Templates use Rust format syntax, including precision such as `{value:.2}`.
For a quantity, put both complete messages under `one` and `other` and pass
`count = quantity` before the other arguments. English uses `one` for exactly
1 and `other` for zero and all other quantities.

Rust embeds English at build time and the browser startup screen reads the
same catalog. Keep storage keys, shortcuts, file-format tokens and wire codes
separate from display messages. Release-note content lives in its own feed.

## Compile for distribution

Desktop, on the OS you want to target:

```sh
cargo build -p benchup-app --release --locked
```

The executable is `target/release/benchup-app` (`benchup-app.exe` on Windows). These commands produce executables, not installers or signed application bundles. Native builds should run on the corresponding OS; this project does not configure cross-linkers or signing identities.

### Automated release builds

[Build releases](.github/workflows/build.yml) runs when a `v*` tag is pushed
(for example, `git push origin v0.1.0`). The tag must be a stable `vX.Y.Z`
version matching `[workspace.package].version` in `Cargo.toml`. It builds with
stable Rust and the committed `Cargo.lock`, runs update regression tests, then
packages the desktop app with Velopack and the browser app with Trunk:

| Artifact | Platform | Install / run |
|---|---|---|
| `benchup-windows-x64` | Windows x64 (MSVC) | `*-win-x64-Setup.exe` (per-user installer) |
| `benchup-macos-arm64` | macOS Apple Silicon | `*-osx-arm64-Setup.pkg` or the portable `.app` zip |
| `benchup-linux-x64` | Linux x64 (built on Ubuntu 22.04) | `*-linux-x64.AppImage` |
| `benchup-wasm` | Desktop browsers (WebAssembly) | Extract `benchup-vX.Y.Z-wasm.zip` and serve it with a static web server |

After all desktop and WebAssembly builds succeed, the workflow creates a GitHub Release for the
pushed tag with the prepared release notes, installers, portable packages, full
`.nupkg` updates, one `releases.<platform>.json` feed per OS/architecture, the browser app ZIP, and
`SHA256SUMS`. It validates the packages against their feed hashes and publishes
only after every platform succeeds and all assets have uploaded. Reruns can
replace assets on an unfinished draft; a published version cannot be overwritten.
To ship another update, first add its user-facing notes to
`assets/release-notes.json` (newest version first, keeping previous releases),
then bump the workspace version and lockfile. Validate and preview the notes:

```sh
python3 scripts/desktop_release.py version --tag vX.Y.Z
python3 scripts/desktop_release.py notes --tag vX.Y.Z
```

Refresh the README screenshot by launching an existing desktop executable as
described below. Deploy requests do not run local builds or tests; GitHub Actions
performs the desktop and browser builds, tests, and packaging. Commit and push
the release changes and notes before creating and pushing the matching tag.
Release validation rejects missing or mismatched notes. The same
catalog supplies the GitHub Release description and **Help → Release notes**
in the app. Publishing uses GitHub's automatic token. Signing needs the credentials below.

### Release signing

Without signing secrets, CI produces **unsigned packages for development**.
Before public distribution, configure signing and set the repository Actions
variable `BENCHUP_REQUIRE_SIGNING=true`. This makes packaging fail if Windows or
macOS signing is missing. Checksums detect damaged downloads; they do not
replace publisher signatures. Release access and signing credentials should be
restricted to maintainers. Velopack's [signing guide](https://docs.velopack.io/packaging/signing)
describes certificate/account setup.

For Windows, configure these repository Actions secrets:

| Secret | Value |
|---|---|
| `AZURE_CREDENTIALS` | Azure service-principal credentials JSON for `azure/login`, with permission to sign using the chosen profile |
| `WINDOWS_AZURE_SIGNING` | JSON with `Endpoint`, `CodeSigningAccountName`, and `CertificateProfileName` for Azure Artifact Signing |

The workflow logs into Azure and passes a temporary configuration file to
Velopack's Windows signing integration. The account needs the **Artifact Signing
Certificate Profile Signer** role. Windows SDK signing tools are supplied by
the Windows build runner.

For macOS, export the Developer ID Application and Developer ID Installer
certificates **with their private keys** together into a password-protected
`.p12`, then configure:

| Secret | Value |
|---|---|
| `MACOS_CERTIFICATES_P12` | Base64 of the `.p12`, on one line |
| `MACOS_CERTIFICATE_PASSWORD` | Password protecting the `.p12` |
| `MACOS_SIGN_APP_IDENTITY` | Exact Developer ID Application signing identity |
| `MACOS_SIGN_INSTALL_IDENTITY` | Exact Developer ID Installer signing identity |
| `APPLE_ID` | Apple developer account email |
| `APPLE_TEAM_ID` | Apple developer team ID |
| `APPLE_APP_PASSWORD` | App-specific password for notarization |

CI imports the certificates into a temporary keychain, stores a notarytool
profile, and asks Velopack to sign and notarize the app and installer. It removes
the `.p12` after importing and deletes the keychain even if packaging fails.
Partially configured signing fails instead of silently producing unsigned files.

The Rust dependency, `VPK_VERSION` in the workflow, and the version in
`scripts/desktop_release.py` are pinned together at `1.2.161`. Update them
together. Only the packaging machine needs .NET 8 for the `vpk` CLI; the Rust
app does not require users to install .NET.

Before shipping publicly, install one packaged version on each supported OS,
make a test project, publish a newer version, and verify automatic download,
restart, restored work/file association, the opt-out preference, and failure
recovery. Native update tests run in tag CI; package installation and OS trust
dialogs still need this platform smoke test.

### Browser distribution

Tagged GitHub Releases include `benchup-vX.Y.Z-wasm.zip`, containing the browser
entry point, JavaScript loader, WebAssembly module, branding, translations, and
license files. Extract the ZIP into a static web server's document directory;
open its URL rather than opening `index.html` directly from disk. The archive
is also available as the `benchup-wasm` workflow artifact and is covered by the
release's `SHA256SUMS`.

To compile the browser distribution yourself:

```sh
trunk build --release --locked
```

Serve the resulting `dist/` directory with a static web server. Use HTTPS for WebGPU (localhost is also supported). `public_url = "./"` allows serving from a subdirectory. There is no service worker or external asset dependency.

### GitHub Pages website

The presentation page lives in [`site/`](site/index.html). The
[Deploy GitHub Pages](.github/workflows/pages.yml) workflow downloads the
`benchup-wasm` artifact from the successful release run and extracts it into
`dist/site/play/`. Each tagged release compiles the browser app once; GitHub
Releases and Pages use the same bundle. Manual Pages runs build the browser app
from the selected ref instead. The workflow adds the landing page and existing
branding and screenshot assets to `dist/site/`, and deploys them together. The
main browser entry point (`index.html`) and `trunk serve` continue to open the
editor directly.

To enable publishing for `BenchUp3D/editor`:

1. In the repository, open **Settings → Pages → Build and deployment** and set
   **Source** to **GitHub Actions**. See the
   [GitHub Pages setup guide](https://docs.github.com/en/pages/getting-started-with-github-pages/configuring-a-publishing-source-for-your-github-pages-site#publishing-with-a-custom-github-actions-workflow).
2. Push a `v*` tag (for example, `git push origin v0.1.0`), or run
   **Deploy GitHub Pages** from the Actions tab with `main` selected. Version
   tag pushes start the release workflow. Pages packages and deploys the
   same commit only after that workflow succeeds, including publishing the
   desktop and WebAssembly release. The updated Pages workflow must be on the default branch
   for this automatic trigger to work. Ordinary branch pushes do not trigger
   either workflow.
3. After the deployment succeeds, the landing page is at
   <https://benchup3d.github.io/editor/> and its **Try in browser** links open
   <https://benchup3d.github.io/editor/play/>. The workflow's environment link shows
   the actual URL if a custom domain is configured.

All local page and editor asset links are relative, so the same package supports
the repository path or a custom domain. GitHub Pages supplies HTTPS. The editor
uses browser-local autosave; visitors should download `.bup` projects they
want to keep. GitHub Pages is not cloud model storage.

The landing page includes a canonical URL, Open Graph image metadata, and a
large-image Twitter/X card using the editor screenshot. During deployment,
the workflow sets these absolute URLs from the GitHub Pages configuration,
including a configured custom domain. Social previews require the deployed
public HTTPS page and image; the local preview URL cannot be fetched by social
networks.

## Development and verification

Refresh the README screenshot from an existing desktop executable:

```sh
python3 -B scripts/capture_readme_desktop.py
```

On Linux, this requires X11 or XWayland, ImageMagick's `import` command, and
Python Pillow. The script selects the newest existing executable from the native
release and debug output directories; `--binary /path/to/benchup-app` selects one
explicitly. It launches the desktop app in an isolated temporary profile, loads
[`assets/examples/Example.bup`](assets/examples/Example.bup) with its saved
camera and display settings, dismisses open dialogs and popups, captures the
settled window at 1920 × 1080, and updates `assets/screenshots/benchup.jpg` and its
README link. Visually verify that no modal remains open before using the image.
It never compiles the app, runs tests, or starts benchmarks. The screenshot
reflects the existing executable; a missing executable is reported without
building one.

```sh
cargo check --workspace --locked
cargo check --workspace --target wasm32-unknown-unknown --locked
cargo test --workspace --locked
```

The vendored SketchUp reader, adapter, and browser shortcut handling can also be tested without compiling the application:

```sh
node --test scripts/test_skp_import.cjs
node --test scripts/test_web_shortcuts.cjs
```

An optional offscreen GPU test renders the actual UI and custom viewport to PNGs without opening a window:

```sh
cargo test -p benchup-app gpu_renders_empty_and_modeled_workspace --locked -- --ignored --nocapture
```

Snapshots appear in `target/qa/`. This test requires a working native GPU adapter. Normal tests cover document validation, geometry, inference, camera math, selection, undo/redo, and synthetic keyboard/pointer interactions through egui.

Measure actual viewport frame intervals in the running Web app without starting a build:

```sh
node scripts/benchmark_viewport_web.mjs --headed --project /path/to/Project.bup --seconds 25 --min-fps 45 --output /tmp/benchup-viewport.json
```

The benchmark uses an isolated browser profile, preserves the complete project,
warms up for five seconds, and records FPS, frame timings, the hardware adapter,
geometry buffer rebuilds, screenshots, and the project hash. It rejects software
adapters, blank screenshots, GPU validation errors, and missing measurements.
Use `--mode select` to alternate selection while picking across the scene, or
`--mode groups` to repeatedly enter/leave nested groups. `--mode draw-line`
and `--mode draw-rectangle` drive real pointer/keyboard events with snapping,
preview, geometry creation and autosave enabled; their FPS includes the commit
stall and counts actual GPU frame submissions. Reports include commit latency
and the number of created edges. `--mode measure-tape` measures live tape
snapping with a saved guide present and checks that another guide is saved.
`--mode walk` places a person on a real floor, walks with WASD and turns the
camera. `--after-walk` checks an editor tool after exiting Walk and restoring its
camera. The reports count Walk bridge mutations so repeated browser capture
updates cannot silently return. `--require-pointer-lock` additionally requires
browser mouse capture; without it, a reported browser refusal uses ordinary
mouse look. Each run pins its loaded artifact against development-server reloads.
`--validate-snapping` compares indexed and exhaustive inference on the actual
project, including axis/plane locks, editing scopes and both projections.
`--validate-state`
checks immediate selection/context restoration after reload and isolation from
another project; `--validate-shader` checks edge occlusion on the GPU.
Pass `--legacy` to compare
the CPU projection path on the same scene, or `--profile` for a separate CPU
profile (profiling adds overhead). The default server is `http://127.0.0.1:8080/`;
use `--url` or `--browser` to override the server or Chromium executable.
Large opaque perspective scenes retain their face and edge geometry on the GPU;
other display modes continue to use the precision renderer. Selection, hover,
and editing scope update a small GPU state texture without rebuilding geometry.
Exact inspector measurements run in a background worker. Selection and editing
scope persist immediately in a small record tied to the current autosave,
avoiding a complete document rewrite for each click.

Run the repeatable CPU interaction benchmarks explicitly (10/100-face drawing,
10/100-object Move, a 4,096-face imported model, axes on/off with 512 groups,
and 256/1,024 shadow casters over a large receiving face):

```sh
cargo test -p benchup-app --locked --config 'profile.test.package.benchup-app.opt-level=2' --config 'profile.test.package.benchup-core.opt-level=2' workload_benchmark -- --ignored --nocapture --test-threads=1
```

These settings optimize application geometry and rendering code while retaining
the existing test dependencies. Timings exclude model construction and GPU
presentation; compare runs on the same machine without other CPU-heavy work.
The shadow workload reports regeneration after sun changes and CPU mesh/edge
rendering separately.
The normal suite also exercises all 22 tools through interrupted gestures,
focus loss, resizing, and document reload, plus display styles and effects.

For a sequential feature audit, including the normally ignored GPU comparisons
and per-tool CPU workloads, run:

```sh
python3 scripts/run_feature_audit.py
```

This uses an isolated temporary autosave directory and writes the feature
checklist, individual test results, GPU comparisons, timings, command logs, and
remaining platform checks to `target/qa/feature-audit/report.txt` and
`report.json`. Every display field has an independent before/after GPU case;
normal interaction tests click the individual toggles, face-style choices,
color pickers, paint swatches, and project confirmation buttons.

Verified during implementation: native Linux and browser WASM Cargo checks; core and UI interaction tests; offscreen Vulkan rendering of empty, modeled, and compact workspaces. Windows/macOS execution, browser startup, and runtime WebGPU/WebGL2 fallback still require platform smoke testing. Production builds and hosted deployment were not run.

## Layout

- `crates/benchup-core`: double-precision document geometry, planar face discovery/triangulation, inference, picking, camera, tool definitions, and bounded history. It has no GUI or GPU dependencies.
- `crates/benchup-app`: egui workspace, gesture routing, local persistence, native/browser launchers, and the wgpu/WGSL edge renderer. Model coordinates remain independent of GPU resources.
- `index.html` and `Trunk.toml`: static browser host and WASM development configuration.

The viewport uses depth-tested filled triangles and antialiased triangle strips for consistent edge widths, CPU projection/clipping, and the same shader and editor behavior on every target. Rendering is event-driven rather than a continuous idle loop.

### Account server

Google login and account sessions are provided by the separate Ketchup Server
repository at `/home/dcoulombe/dev/ketchup-server`. See its `README.md` for OAuth
configuration and startup instructions.

The account server is deployment configuration, with no address field in the app.
Launch the desktop app with `benchup-app --server-url https://accounts.example.com`
or `cargo run -p benchup-app -- --server-url http://localhost:8081`.
Both `--server-url URL` and `--server-url=URL` are supported.

Alternatively, copy `.env.example` to `.env` in the workspace root and set
`BENCHUP_SERVER_URL`. The example uses port 8080 for the local Web client through
Trunk; for native-only development, use port 8081. Builds embed this value for desktop and Web; changing `.env`
causes Cargo to refresh it on the next build. You can also supply it directly:

```sh
BENCHUP_SERVER_URL=https://accounts.example.com cargo build -p benchup-app
BENCHUP_SERVER_URL=https://accounts.example.com trunk build --release
```

Desktop precedence is launch argument, runtime `BENCHUP_SERVER_URL`, `.env` in the
current working directory, then the compiled value. Web uses the compiled value.
There is no implicit server address; without configuration the modeling app runs
with the account avatar, its separator, and the login dialog hidden. Saved user preferences
cannot override the server address.

For local desktop development, use `http://localhost:8081`. For the local Web
client, compile with `BENCHUP_SERVER_URL=http://localhost:8080` to use the
`Trunk.toml` account proxies to the server on port 8081. These proxies use
`no_redirect = true` so Google login redirects reach the browser. Restart
`trunk serve` after changing proxy configuration. Other browser deployments
need a same-origin reverse proxy or the server's explicitly configured CORS and
compatible cookie policies. Use HTTPS outside localhost.

`.env` supports `BENCHUP_SERVER_URL=value`, single/double quoted values, optional
`export`, and trailing comments. It is ignored by Git; do not put secrets in the
URL because Web builds expose it to the browser.

Desktop update feeds follow the repository that built the application. GitHub
Actions embeds `github.repository`, so fork releases update from that fork's
GitHub Releases. Local builds use the GitHub `origin` remote; source archives
without Git metadata default to `BenchUp3D/editor`. Set `BENCHUP_UPDATE_REPOSITORY`
to `owner/repository` at build time to select an explicit feed. The fork must
publish its own compatible installer and update assets through the release
workflow. Already installed upstream binaries retain their original update feed.

Forks use their own GitHub repository for website source, documentation, license,
and release links. Pages resolves these links from `GITHUB_REPOSITORY` during
staging. Enable Actions in a fork and run **Synchronize repository links** once
(or push a README change) to update the README links; later README changes on
the default branch synchronize automatically. Set the repository variable
`PAGES_BASE_URL` when using a custom Pages domain. The fork must enable Pages
and publish its own releases before its browser and download links are available.


### BenchUp identity migration

The application, desktop package, and bundle ID are now `BenchUp` /
`io.github.BenchUp3D.BenchUp`; the executable is `benchup-app` and new
projects use `.bup`. Existing `.k3d` and `.ketchup` projects remain readable.
Desktop workspaces, update preferences, restart metadata, and inspector settings
are copied from the old KetchUp locations on first launch without deleting the
original data. Browser storage migrates on the same origin; a change of hostname
requires exporting projects before moving because browsers isolate local storage.
The old `KETCHUP_DATA_DIR`, `KETCHUP_SERVER_URL`, and
`KETCHUP_UPDATE_REPOSITORY` variables remain accepted as compatibility aliases;
the `BENCHUP_*` equivalents take precedence.

The official migration is a transfer/rename of the **original** repository from
`Ketchup3D/app` to `BenchUp3D/editor`. Transfer and rename may happen in either
order. Existing 0.6.5 applications retain the original GitHub URL in their binary;
GitHub's repository redirects let those applications reach the new releases and
download their assets. Do not create a new repository or fork at `Ketchup3D/app`:
[GitHub removes the transfer redirects when the old location is reused](https://docs.github.com/en/repositories/creating-and-managing-repositories/transferring-a-repository).
The organization/repository transfer itself remains a GitHub administration step.

Before an official release builds, CI runs:

```sh
python3 -B scripts/verify_update_transfer.py --repository BenchUp3D/editor
```

This uses public, unauthenticated GitHub API requests, like the installed updater.
It requires the old and new repository URLs to return the same numeric repository
ID and the same releases. A missing redirect, an incorrect transfer, or a recreated
old repository blocks the release before publication. Forks skip this official
transfer requirement and continue using their own release source.

The first BenchUp release must be newer than 0.6.5 (for example 0.6.6). Packaging
explicitly uses `--delta None`; publication validation rejects delta feeds so old
clients receive a complete replacement with the new app ID and executable. Keep
the existing `win-x64`, `osx-arm64`, and `linux-x64` channels. New official builds
normalize both intermediate repository names to `BenchUp3D/editor`.

The pinned Velopack 1.2.161 client is exercised in CI with an old 0.6.5 manifest,
HTTP transfer/rename redirects, synthetic full packages and real checksums on all
three channels. It checks discovery, download validation, pending-update discovery,
and the new executable metadata. The test intentionally does not use BenchUp's
new update-preparation code, which is absent from the already-installed clients.
An actual installed-app replacement/restart with signed release packages remains
a separate OS-level verification; the fixture test does not execute an installer.
