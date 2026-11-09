Synthetic full packages used by `updates::transfer_tests` to exercise the pinned
Velopack 1.2.161 client shipped in KetchUp 0.6.5. Regenerate with
`python3 -B crates/benchup-app/tests/fixtures/update-transfer/generate.py`.

The packages contain a valid BenchUp manifest and inert text markers, with real
sizes, SHA-1 and SHA-256 hashes. They test redirect handling, download validation,
pending-update discovery, and the new executable handoff on all platform channels.
They do not install or execute an application. OS installation/restart still
requires a real signed package and an installed 0.6.5 application.
