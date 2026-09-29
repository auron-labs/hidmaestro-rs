# Security Policy

## Reporting a Vulnerability

Report suspected vulnerabilities through GitHub's private vulnerability
reporting for this repository:

https://github.com/auron-labs/hidmaestro-rs/security/advisories/new

Please do not open a public issue for a security problem.

## Scope

The Windows x64 bundle can install a user-mode HID driver and create virtual
controllers, and its bridge accepts a named-pipe client. Reports about privilege
boundaries, driver installation, pipe access control, or unsafe input handling
are in scope.

## Supported Versions

Security fixes target the latest release.
