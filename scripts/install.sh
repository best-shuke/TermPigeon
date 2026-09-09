#!/usr/bin/env bash
set -euo pipefail

script_dir="$(cd -- "$(dirname -- "${BASH_SOURCE[0]}")" && pwd)"
project_dir="$(dirname -- "${script_dir}")"
env_source="${TERMPIGEON_ENV_FILE:-${project_dir}/.env}"
install_only="${1:-}"

if [[ -n "${install_only}" && "${install_only}" != "--install-only" ]]; then
  echo "error: unknown argument: ${install_only}" >&2
  exit 1
fi

for command_name in install systemctl; do
  if ! command -v "${command_name}" >/dev/null 2>&1; then
    echo "error: required command not found: ${command_name}" >&2
    exit 1
  fi
done

if [[ ! -f "${env_source}" ]]; then
  echo "error: environment file not found: ${env_source}" >&2
  echo "copy .env.example to .env and fill in the required values first" >&2
  exit 1
fi

if [[ "$(stat -c '%a' "${env_source}")" != "600" ]]; then
  echo "error: ${env_source} must have mode 0600" >&2
  exit 1
fi

for required_key in TELOXIDE_TOKEN TERMPIGEON_OWNER_USER_ID TERMPIGEON_CODEX_BIN TERMPIGEON_CODEX_HOME TERMPIGEON_WORKDIR TERMPIGEON_STATE_DIR; do
  if ! grep -Eq "^${required_key}=.+" "${env_source}"; then
    echo "error: ${required_key} is missing or empty in ${env_source}" >&2
    exit 1
  fi
done

if [[ "${install_only}" != "--install-only" ]]; then
  if ! command -v cargo >/dev/null 2>&1; then
    echo "error: required command not found: cargo" >&2
    exit 1
  fi
  (
    cd -- "${project_dir}"
    cargo test --locked
    cargo build --release --locked
  )
fi

if [[ "${EUID}" -ne 0 ]]; then
  if ! command -v sudo >/dev/null 2>&1; then
    echo "error: sudo is required for installation" >&2
    exit 1
  fi
  exec sudo env TERMPIGEON_ENV_FILE="${env_source}" "${script_dir}/install.sh" --install-only
fi

install -m 0755 "${project_dir}/target/release/term-pigeon" /usr/local/bin/term-pigeon
install -m 0600 "${env_source}" /etc/term-pigeon.env
install -m 0644 "${project_dir}/systemd/term-pigeon.service" /etc/systemd/system/term-pigeon.service

systemctl daemon-reload
systemctl enable term-pigeon.service
systemctl restart term-pigeon.service
systemctl --no-pager --full status term-pigeon.service
