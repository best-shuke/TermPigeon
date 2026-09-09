# Contributing

Thanks for helping improve TermPigeon.

## Before opening a change

1. Do not include Telegram tokens, Codex credentials, session files, private endpoints, or logs
   containing message contents.
2. Keep the single-owner security boundary explicit. Changes that add multi-user access need a
   separate threat model and should not silently reuse the existing state store.
3. Open an issue first for protocol, persistence-format, or privilege-model changes.

## Local checks

Run all checks before submitting a pull request:

```bash
cargo fmt --check
cargo test --locked
cargo clippy --locked --all-targets -- -D warnings
```

Tests should cover new parsers, state migrations, and message formatting behavior. User-visible
changes should update both `README.md` and `README.en.md` when applicable.

## Commit and pull-request guidance

- Keep commits focused and explain the behavior change in the subject.
- Describe security implications and deployment changes in the pull request.
- Include the Codex CLI version used for protocol-related testing.
- Do not paste private Telegram updates or full Codex transcripts into public issues.

By contributing, you agree that your contribution is licensed under the repository's MIT License.
