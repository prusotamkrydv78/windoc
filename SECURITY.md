# Security policy

## Supported versions

This project is pre-release. Only the latest commit on the main branch receives security fixes. There are no supported release builds yet.

## Reporting a vulnerability

Please report vulnerabilities privately. Do not open a public issue.

- Contact: **[maintainer to add a private security contact before the first public release]**
- Include the affected commit, steps to reproduce, the impact you observed and any proof-of-concept.
- If you need to share files, say so in your first message and we will agree a private channel.

## What to expect

- Acknowledgement within 5 working days.
- An initial assessment, including whether we consider it in scope, within 14 days.
- Coordinated disclosure: we will agree a disclosure date with you once a fix is available. We aim to fix critical issues within 30 days.
- Credit in the release notes if you want it.

## Scope

In scope:

- The application and every crate in this repository.
- Plugin package handling: manifest parsing, signature and hash checks, ZIP extraction (for example path traversal or decompression bombs).
- Settings and data files under `~/.winisland` and `%APPDATA%\WinIsland`.
- Any network request the application makes.

Out of scope:

- **Native plugins themselves.** Plugins are DLLs that run inside the application with the user's full privileges. Installing a plugin is equivalent to running a program from its author. Report plugin bugs to the plugin's author. Report flaws in how the host loads, validates or isolates plugins to us.
- Vulnerabilities in Windows or in third-party dependencies, unless this project uses them in an unsafe way. Please still tell us so we can update.
- Attacks that require an attacker to already control the user's account.

## Current security posture

- The inherited updater and plugin marketplace are disabled in code until this project has its own signed release feed and catalog key. See `docs/PRODUCT_ENGINEERING_WORKFLOW.md`.
- No telemetry is collected. See `docs/PRIVACY_MODEL.md`.
