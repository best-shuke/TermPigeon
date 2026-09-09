# TermPigeon

English | [简体中文](README.md)

An owner-only, self-hosted Telegram bridge for remotely controlling local coding agents. It lets
you inspect files, check logs, modify projects, and operate a server from a private Telegram chat
while preserving conversation context between messages. The current release integrates Codex CLI;
future backends can include Claude Code and other local coding agents.

> This is an independent community project and is not affiliated with Telegram, OpenAI, or
> Anthropic. The current release invokes a locally installed Codex CLI and uses the Telegram Bot
> API as its message transport.

## Features

- Private-chat and single-owner allowlist enforcement
- Persistent Codex threads through `codex exec --json` and `codex exec resume`
- `/new` and `/clear` for starting a fresh conversation
- Immediate receipt confirmation and an ordered, bounded task queue
- Model, reasoning effort, Fast mode, and response-verbosity controls
- Telegram-safe rich-text conversion, message splitting, and plain-text fallback
- Atomic state persistence across service restarts
- Host and service status commands
- Hardened systemd deployment with automatic restart

## Bot commands

- `/new` — start a fresh conversation
- `/clear` — clear the saved context and start a fresh conversation
- `/model` — view or change the model
- `/effort` — view or change reasoning effort
- `/fast` — toggle Fast mode
- `/verbosity` — change response verbosity
- `/settings` — show the current Codex settings
- `/defaults` — restore defaults without clearing context
- `/services` — list application services, timers, and containers
- `/status` — show host and Codex session status
- `/help` — show help

Model and feature availability depends on the installed Codex CLI version and your account.

## How it works

```text
Telegram private chat
    -> owner and chat validation
TermPigeon (Rust / teloxide)
    -> ordered task queue
local Codex CLI (JSONL protocol)
    -> configured working directory and tools
```

The first message in a conversation runs `codex exec --json`. TermPigeon saves the thread ID from the
event stream and uses `codex exec resume` for subsequent messages. `/new` and `/clear` discard the
saved thread ID without deleting Codex's local session files.

## Requirements

- Linux with systemd
- A working Codex CLI installation
- Rust 1.92 or newer for compilation
- A Telegram bot token from [@BotFather](https://t.me/BotFather)
- Your numeric Telegram user ID

If the Codex distribution also includes `codex-code-mode-host`, keep it in the same directory and
at the same version as `codex`.

## Installation

```bash
git clone https://github.com/best-shuke/TermPigeon.git
cd TermPigeon
cp .env.example .env
chmod 600 .env
```

Fill in at least these values:

```dotenv
TELOXIDE_TOKEN=123456:replace-with-your-token
TERMPIGEON_OWNER_USER_ID=123456789
TERMPIGEON_CODEX_BIN=/usr/local/bin/codex
TERMPIGEON_CODEX_HOME=/var/lib/term-pigeon/codex-home
TERMPIGEON_WORKDIR=/srv
TERMPIGEON_STATE_DIR=/var/lib/term-pigeon/state
TERMPIGEON_DANGEROUSLY_BYPASS_APPROVALS_AND_SANDBOX=false
```

All paths must be absolute, and `TERMPIGEON_WORKDIR` must already exist.

Authenticate the dedicated Codex home used by the service:

```bash
sudo install -d -m 0700 /var/lib/term-pigeon/codex-home
sudo env CODEX_HOME=/var/lib/term-pigeon/codex-home \
  /usr/local/bin/codex login --device-auth
```

Then build, install, and start the service:

```bash
./scripts/install.sh
sudo systemctl status term-pigeon.service
sudo journalctl -u term-pigeon.service -f
```

Running the installer again performs a locked test/build, replaces the installed artifacts, and
restarts the service so the new binary is always active.

## Security

The public-deployment default is:

```dotenv
TERMPIGEON_DANGEROUSLY_BYPASS_APPROVALS_AND_SANDBOX=false
```

This runs Codex in `workspace-write` mode. Enabling the bypass flag removes that boundary. Because
the bundled systemd unit runs as root, control of the Telegram account or bot token then becomes
equivalent to root access. Use a dedicated bot, enable Telegram two-step verification, keep the
owner allowlist strict, and rotate credentials regularly.

Never commit `.env`, `/etc/term-pigeon.env`, `$TERMPIGEON_CODEX_HOME/auth.json`, session data, or
credentials.
See [SECURITY.md](SECURITY.md) for the full trust model.

## Development

```bash
cargo fmt --check
cargo test --locked
cargo clippy --locked --all-targets -- -D warnings
cargo build --release --locked
```

## License

[MIT](LICENSE)
