# Security policy

This policy covers BenchUp, the desktop and browser 3D editor in this repository,
formerly named KetchUp. It does not cover the separate Ketchup Server project.

## Supported versions

Security fixes target the [latest stable release](https://github.com/BenchUp3D/editor/releases/latest).
Older releases do not receive security backports. Please update before checking
whether an issue still occurs. Reports affecting development builds are also
welcome; include the commit identifier.

## Reporting a vulnerability

Please report suspected vulnerabilities privately before sharing exploit details
in a public issue, pull request, or discussion.

Use GitHub's [Report a vulnerability](https://github.com/BenchUp3D/editor/security/advisories/new)
form if private vulnerability reporting is available. If the form is unavailable,
open a [public issue](https://github.com/BenchUp3D/editor/issues/new) asking
maintainers for a private security contact. Include no vulnerability details,
sample files, credentials, or exploit code in that public request; wait for a
private channel before sending them.

Include the following in a private report where possible:

- The affected BenchUp version or commit, installation source, and whether you
  used the desktop or browser application.
- Your operating system and, for the browser application, browser and version.
- Reproduction steps and a minimal sample project or import file.
- The expected and observed behavior, security impact, and any prerequisites.
- Relevant logs or a crash trace, with personal information and secrets removed.
- Whether the issue reproduces in the latest stable release, if you can safely
  check, and any suggested mitigation or fix.

Use synthetic or sanitized samples rather than confidential project files. A
complete exploit is not required to report a suspected vulnerability.

## Scope

Examples of relevant reports include:

- Malicious `.bup`, legacy `.k3d` or `.ketchup`, or imported `.skp` files causing
  memory corruption, code execution, unintended file access, or excessive
  resource consumption.
- Security defects in native C/C++ code, Rust boundaries, bundled dependencies,
  or browser integration that affect BenchUp.
- Unauthorized disclosure or modification of project data or local files.
- Compromise of desktop installers, update handling, release artifacts, or this
  repository's build and publishing workflows.

For ordinary modeling errors, UI defects, or performance problems without a
suspected security impact, use the public issue tracker. If you are unsure
whether a problem is security-related, report it privately first.

## Handling and disclosure

Maintainers will assess reports, request additional information when needed, and
coordinate fixes and disclosure with the reporter. Response and fix times depend
on severity, reproducibility, and maintainer availability; no fixed response or
resolution deadline is guaranteed.

Please allow time to investigate and make a fix available before publishing
technical details. Confirmed vulnerabilities may be documented in
[GitHub Security Advisories](https://github.com/BenchUp3D/editor/security/advisories)
and release notes, with affected versions and mitigations where available.
Reporters can request credit or remain anonymous.
