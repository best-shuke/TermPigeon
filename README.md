# TermPigeon

[English](README.en.md) | 简体中文

一个通过 Telegram 远程控制本地编程 Agent 的自托管桥接工具。你可以像聊天一样
让 Agent 检查文件、查看日志、修改项目或管理服务器，并在消息之间延续同一条会话。
当前版本接入 Codex CLI；项目名称和定位不绑定供应商，后续可扩展 Claude Code 等后端。

> 这是独立社区项目，与 Telegram、OpenAI 或 Anthropic 均无隶属关系。当前版本调用
> 本机安装的 Codex CLI，并使用 Telegram Bot API 作为消息入口。

## 功能

- Telegram 私聊白名单，只接受一个 `TERMPIGEON_OWNER_USER_ID`
- 使用 `codex exec --json` 执行任务，保存 thread ID 并通过 `codex exec resume` 续聊
- `/clear` 和 `/new` 新建会话
- 模型、推理强度、Fast 模式与回答详细度的按钮菜单
- Markdown 转 Telegram HTML，支持长消息安全分段与纯文本回退
- 同一时刻只执行一个 Codex 任务，后续消息自动排队
- 每条普通消息会先收到即时回执，再按 Telegram 到达顺序交给 Codex
- 会话状态原子落盘，服务重启后仍能续聊
- `/status` 与 `/services` 查看主机和服务状态
- systemd 常驻、自动重启与最小化服务加固

## 命令

- `/clear` — 清除上下文并开启新对话
- `/new` — 开启新对话，与 `/clear` 相同
- `/model` — 查看或切换模型
- `/effort` — 查看或切换推理强度
- `/fast` — 开关 Fast 模式
- `/verbosity` — 调整回答详细度
- `/settings` — 查看当前设置
- `/defaults` — 恢复默认设置，不清除上下文
- `/services` — 查看应用、定时器和容器
- `/status` — 查看主机与 Codex 会话状态
- `/help` — 查看帮助

## 工作方式

```text
Telegram 私聊
    ↓  用户 ID 与私聊校验
TermPigeon（Rust / teloxide）
    ↓  JSONL 子进程协议
本机 Codex CLI
    ↓
指定工作目录与服务器工具
```

新对话首次消息会调用 `codex exec --json`。程序从事件流中取得 thread ID 和最终
回复；后续消息使用保存的 ID 调用 `codex exec resume`。OpenAI 官方文档将
`codex exec` 标为适合脚本和 CI 的非交互入口，并支持 JSONL 输出和恢复会话：
[Codex developer commands](https://developers.openai.com/codex/cli/reference/)。

## 环境要求

- 使用 systemd 的 Linux 服务器
- 已安装并可用的 Codex CLI
- Rust 1.92 或更高版本（仅编译时需要）
- 从 [@BotFather](https://t.me/BotFather) 创建的 Telegram Bot Token
- 你的 Telegram 数字用户 ID

当前 Codex 发行版如果同时提供 `codex-code-mode-host`，请确保它和 `codex` 来自
同一版本并位于同一目录。

## 快速部署

### 1. 获取源码和配置

```bash
git clone https://github.com/best-shuke/TermPigeon.git
cd TermPigeon
cp .env.example .env
chmod 600 .env
```

编辑 `.env`，至少填写：

```dotenv
TELOXIDE_TOKEN=123456:replace-with-your-token
TERMPIGEON_OWNER_USER_ID=123456789
TERMPIGEON_CODEX_BIN=/usr/local/bin/codex
TERMPIGEON_CODEX_HOME=/var/lib/term-pigeon/codex-home
TERMPIGEON_WORKDIR=/srv
TERMPIGEON_STATE_DIR=/var/lib/term-pigeon/state
TERMPIGEON_DANGEROUSLY_BYPASS_APPROVALS_AND_SANDBOX=false
```

所有路径必须是绝对路径，且 `TERMPIGEON_WORKDIR` 必须已经存在。

### 2. 为服务登录 Codex

默认 systemd 服务以 root 身份运行，因此认证也应写入专用的
`TERMPIGEON_CODEX_HOME`：

```bash
sudo install -d -m 0700 /var/lib/term-pigeon/codex-home
sudo env CODEX_HOME=/var/lib/term-pigeon/codex-home \
  /usr/local/bin/codex login --device-auth
```

也可以按 Codex CLI 支持的方式使用 API Key 登录。不要把 `auth.json`、API Key 或
Bot Token 提交到 Git。

如需自定义模型提供商，可先复制示例配置：

```bash
sudo install -m 0600 codex-config.example.toml \
  /var/lib/term-pigeon/codex-home/config.toml
```

### 3. 编译、安装并启动

```bash
./scripts/install.sh
```

安装脚本会执行锁定依赖的 release 构建，将二进制安装到
`/usr/local/bin/term-pigeon`、环境文件安装到 `/etc/term-pigeon.env`，并启用
`term-pigeon.service`。

检查状态：

```bash
sudo systemctl status term-pigeon.service
sudo journalctl -u term-pigeon.service -f
```

## 安全模式

`TERMPIGEON_DANGEROUSLY_BYPASS_APPROVALS_AND_SANDBOX=false` 是面向公开部署的默认值。
Codex 被限制为 `workspace-write`，不能任意修改工作目录以外的主机文件。
服务启动后还会对 `/etc/term-pigeon.env` 建立不可访问挂载，并且不会把 Telegram Token
或 owner ID 继承给 Codex 子进程。

只有当你的明确目标是通过 Telegram 管理整台服务器时，才设置：

```dotenv
TERMPIGEON_DANGEROUSLY_BYPASS_APPROVALS_AND_SANDBOX=true
```

此模式会向 Codex 传递 `--dangerously-bypass-approvals-and-sandbox`。因为 systemd
服务以 root 运行，**控制 Telegram 账号或泄露 Bot Token 等同于取得服务器 root
权限**。建议至少启用 Telegram 两步验证、使用独立 Bot、保持严格的 owner ID
白名单，并定期轮换 Token。

更多安全说明见 [SECURITY.md](SECURITY.md)。

## 配置项

- `TELOXIDE_TOKEN`：Telegram Bot Token，必填
- `TERMPIGEON_OWNER_USER_ID`：唯一允许访问的 Telegram 用户 ID，必填
- `TERMPIGEON_CODEX_BIN`：Codex 可执行文件的绝对路径，必填
- `TERMPIGEON_CODEX_HOME`：此机器人独享的 Codex 数据与认证目录，必填
- `TERMPIGEON_WORKDIR`：Codex 默认工作目录，必填
- `TERMPIGEON_STATE_DIR`：thread ID 与机器人设置的状态目录，必填
- `TERMPIGEON_CODEX_TIMEOUT_SECONDS`：单次任务超时，默认 `1800`
- `TERMPIGEON_DANGEROUSLY_BYPASS_APPROVALS_AND_SANDBOX`：是否启用整机管理模式，默认 `false`
- `RUST_LOG`：日志过滤器，建议保留 `term_pigeon=info,teloxide=warn`，以记录长轮询冲突与网络错误

## 本地开发

```bash
cargo fmt --check
cargo test --locked
cargo clippy --locked --all-targets -- -D warnings
cargo build --release --locked
```

`.env`、`codex-config.toml`、`target/` 和临时状态都不会进入版本控制。

## 更新部署

拉取新代码后再次运行：

```bash
./scripts/install.sh
```

升级 Codex 时，如果安装包包含 `codex-code-mode-host`，必须和 `codex` 一起升级，
避免二进制协议版本不一致。

## License

[MIT](LICENSE)
