# Security Policy

## Reporting a vulnerability

Please do not open public GitHub issues for suspected security vulnerabilities.

Instead, report them privately to the project maintainer through GitHub security advisories or direct maintainer contact if available.

Please include:

- affected version or commit
- operating system
- reproduction steps
- expected impact
- any proof-of-concept details needed to verify the issue

You can expect an acknowledgment as soon as practical, followed by triage and a coordinated fix/release process if the report is confirmed.

## Scope

Security-relevant areas may include:

- shell bootstrap behavior in `mu.sh`
- project file loading and parsing
- audio/MIDI device integration
- camera mode process spawning and external command execution
