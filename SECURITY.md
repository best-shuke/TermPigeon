# Security policy

## Trust model

TermPigeon is intentionally owner-only. It rejects non-private chats and every Telegram user ID
except `TERMPIGEON_OWNER_USER_ID`. This is an access-control boundary, not a multi-user permission
system.

The default deployment keeps Codex in `workspace-write` mode. Enabling
`TERMPIGEON_DANGEROUSLY_BYPASS_APPROVALS_AND_SANDBOX` removes that boundary. When the service runs
as root, possession of the Telegram account or Bot Token must be treated as root-equivalent access.

## Secrets that must never be committed

- `.env` and `/etc/term-pigeon.env`
- Telegram Bot Tokens
- `$TERMPIGEON_CODEX_HOME/auth.json`
- OpenAI API keys or third-party provider credentials
- persisted session state and Codex session data

Before publishing a fork, inspect both tracked files and Git history. Adding a secret to
`.gitignore` does not remove it from existing commits.

## Reporting a vulnerability

Do not open a public issue containing credentials, tokens, private endpoints, or exploit details.
Use the repository host's private security-advisory channel when available. Revoke exposed Bot
Tokens and Codex/OpenAI credentials immediately; code changes alone do not invalidate them.
