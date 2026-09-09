# Changelog

All notable changes to this project will be documented here. The format is based on
[Keep a Changelog](https://keepachangelog.com/en/1.1.0/), and the project follows semantic
versioning.

## [Unreleased]

### Added

- Ordered conversation queue with immediate Telegram receipt confirmations.
- Inbound message and Codex job lifecycle logging without logging message contents.
- English documentation and contributor guidance.
- GitHub Actions checks for formatting, tests, Clippy, and release builds.
- Automated secret scanning for every push and pull request.

### Changed

- `/new` and `/clear` are the documented conversation-reset commands.
- Installer now restarts an already-running service after replacing the binary.
- systemd restarts the bot even if its polling loop exits successfully.
- Recommended logging includes teloxide warnings for polling and network diagnostics.
- Project branding, package names, and deployment paths now use TermPigeon.
