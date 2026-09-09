mod codex;
mod config;
mod server;
mod state;
mod telegram_format;

use std::{
    sync::Arc,
    sync::atomic::{AtomicBool, Ordering},
    time::{Duration, Instant},
};

use anyhow::{Context, Result, bail};
use codex::CodexRunner;
use config::Config;
use state::{
    CodexSettings, ModelChoice, ReasoningEffort, ResponseVerbosity, SessionState, StateStore,
};
use teloxide::{
    prelude::*,
    types::{
        BotCommand, CallbackQuery, ChatAction, ChatKind, InlineKeyboardButton,
        InlineKeyboardMarkup, MessageId, ParseMode,
    },
};
use tokio::sync::{Mutex, OwnedMutexGuard, mpsc};

const CONVERSATION_QUEUE_CAPACITY: usize = 64;

const HELP: &str = r#"# TermPigeon · 服务器 Agent 助手

直接发送普通消息即可连续对话。从一次 `/clear` 或 `/new` 到下一次清理之间的所有普通消息，都属于同一个上下文。

## 对话

- `/clear` — 清除当前上下文并开启全新对话
- `/new` — 开启全新对话（与 `/clear` 相同）

## Codex 设置

- `/model` — 用按钮选择模型
- `/effort` — 用按钮选择推理强度
- `/fast` — 用按钮开关快速模式
- `/verbosity` — 用按钮选择回答详细度
- `/settings` — 查看当前设置
- `/defaults` — 恢复默认设置，不清除上下文

## 服务器

- `/services` — 查看已部署服务
- `/status` — 查看服务器状态
- `/help` — 查看本帮助"#;

#[derive(Clone)]
struct App {
    bot: Bot,
    config: Arc<Config>,
    runner: Arc<CodexRunner>,
    store: Arc<StateStore>,
    session: Arc<Mutex<SessionState>>,
    conversation_lock: Arc<Mutex<()>>,
    conversation_tx: mpsc::Sender<ConversationJob>,
}

#[derive(Debug)]
struct ConversationJob {
    chat_id: ChatId,
    message_id: MessageId,
    kind: ConversationJobKind,
}

#[derive(Debug)]
enum ConversationJobKind {
    Chat(String),
    Reset,
}

#[tokio::main]
async fn main() -> Result<()> {
    dotenvy::dotenv().ok();
    env_logger::Builder::from_env(
        env_logger::Env::default().default_filter_or("term_pigeon=info,teloxide=warn"),
    )
    .init();

    let config = Arc::new(Config::from_env()?);
    tokio::fs::create_dir_all(&config.codex_home)
        .await
        .with_context(|| format!("create Codex home {}", config.codex_home.display()))?;
    let store = Arc::new(StateStore::new(config.state_dir.clone()).await?);
    let session = store.load().await?;
    let bot = Bot::new(config.bot_token.clone());
    let runner = Arc::new(CodexRunner::new(
        config.codex_bin.clone(),
        config.codex_home.clone(),
        config.workdir.clone(),
        config.codex_timeout,
        config.dangerously_bypass_approvals_and_sandbox,
    ));
    let (conversation_tx, conversation_rx) = mpsc::channel(CONVERSATION_QUEUE_CAPACITY);
    let app = App {
        bot: bot.clone(),
        config,
        runner,
        store,
        session: Arc::new(Mutex::new(session)),
        conversation_lock: Arc::new(Mutex::new(())),
        conversation_tx,
    };

    register_commands(&bot).await?;
    log::info!("TermPigeon started; conversation queue capacity={CONVERSATION_QUEUE_CAPACITY}");

    let handler = teloxide::dptree::entry()
        .branch(Update::filter_message().endpoint(message_handler))
        .branch(Update::filter_callback_query().endpoint(callback_handler));

    let conversation_app = app.clone();
    let mut dispatcher = Dispatcher::builder(bot, handler)
        .dependencies(teloxide::dptree::deps![app])
        .enable_ctrlc_handler()
        .build();
    let mut conversation_worker =
        tokio::spawn(run_conversation_worker(conversation_app, conversation_rx));

    tokio::select! {
        _ = dispatcher.dispatch() => {
            conversation_worker.abort();
        }
        result = &mut conversation_worker => {
            match result {
                Ok(()) => bail!("conversation worker stopped unexpectedly"),
                Err(error) => bail!("conversation worker failed: {error}"),
            }
        }
    }
    Ok(())
}

async fn run_conversation_worker(app: App, mut receiver: mpsc::Receiver<ConversationJob>) {
    while let Some(job) = receiver.recv().await {
        let result = match job.kind {
            ConversationJobKind::Chat(text) => {
                app.process_chat(job.chat_id, job.message_id, &text).await
            }
            ConversationJobKind::Reset => app.process_reset(job.chat_id, job.message_id).await,
        };
        if let Err(error) = result {
            log::error!(
                "conversation job failed; message_id={}: {error:#}",
                job.message_id.0
            );
            let _ = app
                .bot
                .send_message(
                    job.chat_id,
                    "❌ 处理失败，请稍后重试。详细原因已写入服务日志。",
                )
                .await;
        }
    }
}

async fn message_handler(app: App, message: Message) -> ResponseResult<()> {
    if let Err(error) = app.handle_message(message).await {
        log::error!("message handling failed: {error:#}");
    }
    respond(())
}

async fn callback_handler(app: App, query: CallbackQuery) -> ResponseResult<()> {
    if let Err(error) = app.handle_callback(query).await {
        log::error!("callback handling failed: {error:#}");
    }
    respond(())
}

impl App {
    async fn handle_message(&self, message: Message) -> Result<()> {
        let sender_id = message.from.as_ref().map(|user| user.id.0);
        let is_private = matches!(message.chat.kind, ChatKind::Private(_));
        if sender_id != Some(self.config.owner_user_id) || !is_private {
            log::warn!(
                "rejected Telegram message from user {:?} in chat {}",
                sender_id,
                message.chat.id
            );
            self.bot
                .send_message(message.chat.id, "未授权：这个机器人仅供服务器所有者使用。")
                .await?;
            return Ok(());
        }

        let Some(text) = message
            .text()
            .map(str::trim)
            .filter(|text| !text.is_empty())
        else {
            self.bot
                .send_message(message.chat.id, "目前只支持文本消息。")
                .await?;
            return Ok(());
        };

        let command = parse_command(text);
        log::info!(
            "received Telegram message; message_id={}, chars={}, kind={}",
            message.id.0,
            text.chars().count(),
            command
                .as_ref()
                .map(|(command, _)| command.as_str())
                .unwrap_or("text")
        );

        if let Some((command, argument)) = command {
            match command.as_str() {
                "start" | "help" => {
                    send_long(&self.bot, message.chat.id, HELP).await?;
                }
                "new" | "clear" => {
                    self.enqueue_reset(message.chat.id, message.id).await?;
                }
                "model" => self.configure_model(message.chat.id, &argument).await?,
                "effort" | "reasoning" | "think" => {
                    self.configure_effort(message.chat.id, &argument).await?
                }
                "fast" => self.configure_fast(message.chat.id, &argument).await?,
                "verbosity" => self.configure_verbosity(message.chat.id, &argument).await?,
                "settings" => self.show_settings(message.chat.id).await?,
                "defaults" => self.restore_defaults(message.chat.id).await?,
                "services" => {
                    send_long(&self.bot, message.chat.id, &server::deployed_services()).await?;
                }
                "status" => {
                    let session = self.session.lock().await.clone();
                    let response = format!(
                        "{}\n\n{}",
                        server::server_status(session.thread_id.is_some()),
                        settings_summary(&session)
                    );
                    send_long(&self.bot, message.chat.id, &response).await?;
                }
                _ => {
                    send_long(
                        &self.bot,
                        message.chat.id,
                        &format!("❌ **未知命令**\n\n{HELP}"),
                    )
                    .await?;
                }
            }
        } else {
            self.enqueue_chat(message.chat.id, message.id, text).await?;
        }
        Ok(())
    }

    async fn handle_callback(&self, query: CallbackQuery) -> Result<()> {
        let Some(message) = query.message.as_ref() else {
            self.bot
                .answer_callback_query(query.id)
                .text("这个选择菜单已经失效，请重新发送命令。")
                .show_alert(true)
                .await?;
            return Ok(());
        };
        let is_private = matches!(message.chat().kind, ChatKind::Private(_));
        if query.from.id.0 != self.config.owner_user_id || !is_private {
            log::warn!(
                "rejected Telegram callback from user {} in chat {}",
                query.from.id.0,
                message.chat().id
            );
            self.bot
                .answer_callback_query(query.id)
                .text("未授权：这个机器人仅供服务器所有者使用。")
                .show_alert(true)
                .await?;
            return Ok(());
        }

        let Some(selection) = query.data.as_deref().and_then(parse_setting_callback) else {
            self.bot
                .answer_callback_query(query.id)
                .text("这个选择已经失效，请重新发送命令。")
                .show_alert(true)
                .await?;
            return Ok(());
        };
        let chat_id = message.chat().id;
        let message_id = message.id();

        // Acknowledge immediately so Telegram removes the loading indicator even
        // when a running Codex task means the setting change must wait briefly.
        self.bot.answer_callback_query(query.id).await?;
        let notice = self.apply_setting(chat_id, selection).await?;
        self.edit_setting_menu(chat_id, message_id, selection.menu(), &notice)
            .await?;
        Ok(())
    }

    async fn enqueue_reset(&self, chat_id: ChatId, message_id: MessageId) -> Result<()> {
        let permit = match self.conversation_tx.clone().try_reserve_owned() {
            Ok(permit) => permit,
            Err(_) => {
                self.bot
                    .send_message(chat_id, "⚠️ 当前队列已满，请稍后重新发送 /new 或 /clear。")
                    .await?;
                return Ok(());
            }
        };
        self.bot
            .send_message(chat_id, "⏳ 已收到，正在开启全新对话…")
            .await?;
        permit.send(ConversationJob {
            chat_id,
            message_id,
            kind: ConversationJobKind::Reset,
        });
        Ok(())
    }

    async fn process_reset(&self, chat_id: ChatId, message_id: MessageId) -> Result<()> {
        let guard = self
            .lock_conversation(chat_id, "当前任务仍在执行；完成后会自动开启新对话。")
            .await?;
        let mut session = self.session.lock().await;
        session.thread_id = None;
        self.store.save(&session).await?;
        drop(session);
        drop(guard);
        log::info!("conversation reset; message_id={}", message_id.0);
        self.bot
            .send_message(
                chat_id,
                "已开启全新对话。旧上下文已清除；从你的下一条普通消息开始，后续消息都会继续使用同一个新上下文，直到再次发送 /clear 或 /new。",
            )
            .await?;
        Ok(())
    }

    async fn enqueue_chat(&self, chat_id: ChatId, message_id: MessageId, text: &str) -> Result<()> {
        let permit = match self.conversation_tx.clone().try_reserve_owned() {
            Ok(permit) => permit,
            Err(_) => {
                self.bot
                    .send_message(chat_id, "⚠️ 当前任务队列已满，请稍后重新发送。")
                    .await?;
                return Ok(());
            }
        };
        self.bot
            .send_message(chat_id, "⏳ 已收到，消息已进入 Codex 处理队列。")
            .await?;
        permit.send(ConversationJob {
            chat_id,
            message_id,
            kind: ConversationJobKind::Chat(text.to_owned()),
        });
        Ok(())
    }

    async fn process_chat(&self, chat_id: ChatId, message_id: MessageId, text: &str) -> Result<()> {
        let conversation_guard = self
            .lock_conversation(chat_id, "上一项任务仍在执行；这条消息已排队。")
            .await?;
        let started = Instant::now();
        let typing_done = Arc::new(AtomicBool::new(false));
        let typing_task = spawn_typing(self.bot.clone(), chat_id, typing_done.clone());
        let (thread_id, settings) = {
            let session = self.session.lock().await;
            (session.thread_id.clone(), session.settings.clone())
        };
        log::info!(
            "starting Codex job; message_id={}, resumed={}",
            message_id.0,
            thread_id.is_some()
        );
        let result = self.runner.run(thread_id.as_deref(), text, &settings).await;
        typing_done.store(true, Ordering::Relaxed);
        typing_task.abort();

        match result {
            Ok(result) => {
                if let Some(thread_id) = result.thread_id {
                    let mut session = self.session.lock().await;
                    session.thread_id = Some(thread_id);
                    self.store.save(&session).await?;
                }
                send_long(&self.bot, chat_id, &result.response).await?;
                log::info!(
                    "Codex job completed; message_id={}, elapsed_ms={}",
                    message_id.0,
                    started.elapsed().as_millis()
                );
            }
            Err(error) => {
                log::error!(
                    "Codex request failed; message_id={}, elapsed_ms={}: {error:#}",
                    message_id.0,
                    started.elapsed().as_millis()
                );
                send_long(
                    &self.bot,
                    chat_id,
                    &format!(
                        "## ❌ Codex 执行失败\n\n```text\n{error:#}\n```\n\n当前会话仍然保留，可以重试或使用 `/clear`。"
                    ),
                )
                .await?;
            }
        }
        drop(conversation_guard);
        Ok(())
    }

    async fn configure_model(&self, chat_id: ChatId, argument: &str) -> Result<()> {
        if argument.is_empty() {
            self.send_setting_menu(chat_id, SettingMenu::Model, None)
                .await?;
            return Ok(());
        }
        let Some(model) = ModelChoice::from_input(argument) else {
            self.send_setting_menu(
                chat_id,
                SettingMenu::Model,
                Some("无效的模型选项，请点击选择。"),
            )
            .await?;
            return Ok(());
        };
        let response = self
            .apply_setting(chat_id, SettingSelection::Model(model))
            .await?;
        self.bot.send_message(chat_id, response).await?;
        Ok(())
    }

    async fn configure_effort(&self, chat_id: ChatId, argument: &str) -> Result<()> {
        if argument.is_empty() {
            self.send_setting_menu(chat_id, SettingMenu::Effort, None)
                .await?;
            return Ok(());
        }
        let Some(effort) = ReasoningEffort::from_input(argument) else {
            self.send_setting_menu(
                chat_id,
                SettingMenu::Effort,
                Some("无效的推理强度，请点击选择。"),
            )
            .await?;
            return Ok(());
        };
        let response = self
            .apply_setting(chat_id, SettingSelection::Effort(effort))
            .await?;
        self.bot.send_message(chat_id, response).await?;
        Ok(())
    }

    async fn configure_fast(&self, chat_id: ChatId, argument: &str) -> Result<()> {
        let value = match argument.trim().to_ascii_lowercase().as_str() {
            "on" | "true" | "1" | "开" | "开启" => Some(true),
            "off" | "false" | "0" | "关" | "关闭" => Some(false),
            _ => None,
        };
        let Some(enabled) = value else {
            self.send_setting_menu(chat_id, SettingMenu::Fast, None)
                .await?;
            return Ok(());
        };
        let response = self
            .apply_setting(chat_id, SettingSelection::Fast(enabled))
            .await?;
        self.bot.send_message(chat_id, response).await?;
        Ok(())
    }

    async fn configure_verbosity(&self, chat_id: ChatId, argument: &str) -> Result<()> {
        if argument.is_empty() {
            self.send_setting_menu(chat_id, SettingMenu::Verbosity, None)
                .await?;
            return Ok(());
        }
        let Some(verbosity) = ResponseVerbosity::from_input(argument) else {
            self.send_setting_menu(
                chat_id,
                SettingMenu::Verbosity,
                Some("无效的回答详细度，请点击选择。"),
            )
            .await?;
            return Ok(());
        };
        let response = self
            .apply_setting(chat_id, SettingSelection::Verbosity(verbosity))
            .await?;
        self.bot.send_message(chat_id, response).await?;
        Ok(())
    }

    async fn apply_setting(&self, chat_id: ChatId, selection: SettingSelection) -> Result<String> {
        let waiting_message = match selection.menu() {
            SettingMenu::Model => "当前任务结束后再切换模型。",
            SettingMenu::Effort => "当前任务结束后再切换推理强度。",
            SettingMenu::Fast => "当前任务结束后再切换 Fast 模式。",
            SettingMenu::Verbosity => "当前任务结束后再切换回答详细度。",
        };
        let guard = self.lock_conversation(chat_id, waiting_message).await?;
        let mut session = self.session.lock().await;

        let (response, changed) = match selection {
            SettingSelection::Model(model) => {
                let changed = session.settings.model != model;
                session.settings.model = model;
                let mut adjustments = Vec::new();
                if !model.supports_effort(session.settings.reasoning_effort) {
                    session.settings.reasoning_effort = ReasoningEffort::Medium;
                    adjustments.push("推理强度已调整为 Medium");
                }
                if !model.supports_fast() && session.settings.fast_mode {
                    session.settings.fast_mode = false;
                    adjustments.push("Fast 模式已关闭");
                }
                let mut response = if changed {
                    format!("模型已切换为 {}。", model.label())
                } else {
                    format!("当前已经是 {}。", model.label())
                };
                if !adjustments.is_empty() {
                    response.push('\n');
                    response.push_str(&adjustments.join("；"));
                    response.push('。');
                }
                (response, changed || !adjustments.is_empty())
            }
            SettingSelection::Effort(effort) => {
                if !session.settings.model.supports_effort(effort) {
                    (
                        format!(
                            "{} 不支持 {}，设置未更改。",
                            session.settings.model.label(),
                            effort.label()
                        ),
                        false,
                    )
                } else {
                    let changed = session.settings.reasoning_effort != effort;
                    session.settings.reasoning_effort = effort;
                    let response = if changed {
                        format!("推理强度已切换为 {}。", effort.label())
                    } else {
                        format!("当前推理强度已经是 {}。", effort.label())
                    };
                    (response, changed)
                }
            }
            SettingSelection::Fast(enabled) => {
                if enabled && !session.settings.model.supports_fast() {
                    (
                        format!(
                            "{} 当前不支持 Fast 模式，设置未更改。",
                            session.settings.model.label()
                        ),
                        false,
                    )
                } else {
                    let changed = session.settings.fast_mode != enabled;
                    session.settings.fast_mode = enabled;
                    let response = match (enabled, changed) {
                        (true, true) => "Fast 模式已开启。响应会更快，但额度或费用消耗也会增加。",
                        (false, true) => "Fast 模式已关闭。",
                        (true, false) => "Fast 模式当前已经开启。",
                        (false, false) => "Fast 模式当前已经关闭。",
                    };
                    (response.to_owned(), changed)
                }
            }
            SettingSelection::Verbosity(verbosity) => {
                let changed = session.settings.verbosity != verbosity;
                session.settings.verbosity = verbosity;
                let response = if changed {
                    format!("回答详细度已切换为 {}。", verbosity.label())
                } else {
                    format!("当前回答详细度已经是 {}。", verbosity.label())
                };
                (response, changed)
            }
        };

        if changed {
            self.store.save(&session).await?;
        }
        drop(session);
        drop(guard);
        Ok(format!("{response} 当前对话上下文继续保留。"))
    }

    async fn send_setting_menu(
        &self,
        chat_id: ChatId,
        menu: SettingMenu,
        notice: Option<&str>,
    ) -> Result<()> {
        let session = self.session.lock().await.clone();
        self.bot
            .send_message(chat_id, setting_menu_text(menu, &session, notice))
            .reply_markup(setting_keyboard(menu, &session))
            .await?;
        Ok(())
    }

    async fn edit_setting_menu(
        &self,
        chat_id: ChatId,
        message_id: MessageId,
        menu: SettingMenu,
        notice: &str,
    ) -> Result<()> {
        let session = self.session.lock().await.clone();
        let result = self
            .bot
            .edit_message_text(
                chat_id,
                message_id,
                setting_menu_text(menu, &session, Some(notice)),
            )
            .reply_markup(setting_keyboard(menu, &session))
            .await;
        match result {
            Ok(_) => Ok(()),
            Err(error) if error.to_string().contains("message is not modified") => Ok(()),
            Err(error) => Err(error.into()),
        }
    }

    async fn show_settings(&self, chat_id: ChatId) -> Result<()> {
        let session = self.session.lock().await.clone();
        send_long(&self.bot, chat_id, &settings_summary(&session)).await?;
        Ok(())
    }

    async fn restore_defaults(&self, chat_id: ChatId) -> Result<()> {
        let guard = self
            .lock_conversation(chat_id, "当前任务结束后再恢复默认设置。")
            .await?;
        let mut session = self.session.lock().await;
        session.settings = CodexSettings::default();
        self.store.save(&session).await?;
        drop(session);
        drop(guard);
        self.bot
            .send_message(
                chat_id,
                "已恢复默认设置：GPT-5.6 Sol、XHigh、Fast 关闭、回答详细度 Medium。当前对话上下文继续保留；需要清空请发送 /clear 或 /new。",
            )
            .await?;
        Ok(())
    }

    async fn lock_conversation(
        &self,
        chat_id: ChatId,
        waiting_message: &str,
    ) -> Result<OwnedMutexGuard<()>> {
        match self.conversation_lock.clone().try_lock_owned() {
            Ok(guard) => Ok(guard),
            Err(_) => {
                self.bot.send_message(chat_id, waiting_message).await?;
                Ok(self.conversation_lock.clone().lock_owned().await)
            }
        }
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum SettingMenu {
    Model,
    Effort,
    Fast,
    Verbosity,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum SettingSelection {
    Model(ModelChoice),
    Effort(ReasoningEffort),
    Fast(bool),
    Verbosity(ResponseVerbosity),
}

impl SettingSelection {
    fn menu(self) -> SettingMenu {
        match self {
            Self::Model(_) => SettingMenu::Model,
            Self::Effort(_) => SettingMenu::Effort,
            Self::Fast(_) => SettingMenu::Fast,
            Self::Verbosity(_) => SettingMenu::Verbosity,
        }
    }
}

fn parse_setting_callback(data: &str) -> Option<SettingSelection> {
    let mut parts = data.split(':');
    if parts.next()? != "cfg" {
        return None;
    }
    let kind = parts.next()?;
    let value = parts.next()?;
    if parts.next().is_some() {
        return None;
    }
    match kind {
        "model" => ModelChoice::from_input(value).map(SettingSelection::Model),
        "effort" => ReasoningEffort::from_input(value).map(SettingSelection::Effort),
        "fast" => match value {
            "on" => Some(SettingSelection::Fast(true)),
            "off" => Some(SettingSelection::Fast(false)),
            _ => None,
        },
        "verbosity" => ResponseVerbosity::from_input(value).map(SettingSelection::Verbosity),
        _ => None,
    }
}

fn setting_menu_text(menu: SettingMenu, session: &SessionState, notice: Option<&str>) -> String {
    let prompt = match menu {
        SettingMenu::Model => format!("当前模型：{}\n请选择模型：", session.settings.model.label()),
        SettingMenu::Effort => format!(
            "当前模型：{}\n当前推理强度：{}\n请选择推理强度：",
            session.settings.model.label(),
            session.settings.reasoning_effort.label()
        ),
        SettingMenu::Fast => {
            let availability = if session.settings.model.supports_fast() {
                "请选择 Fast 模式："
            } else {
                "当前模型不支持开启 Fast；可以选择关闭。"
            };
            format!(
                "当前模型：{}\n当前 Fast 模式：{}\n{availability}\n\n提示：开启 Fast 会提高响应速度，也会增加额度或费用消耗。",
                session.settings.model.label(),
                on_off(session.settings.fast_mode)
            )
        }
        SettingMenu::Verbosity => format!(
            "当前回答详细度：{}\n请选择回答详细度：",
            session.settings.verbosity.label()
        ),
    };
    match notice {
        Some(notice) => format!("{notice}\n\n{prompt}"),
        None => prompt,
    }
}

fn setting_keyboard(menu: SettingMenu, session: &SessionState) -> InlineKeyboardMarkup {
    let buttons: Vec<InlineKeyboardButton> = match menu {
        SettingMenu::Model => ModelChoice::ALL
            .into_iter()
            .map(|model| {
                InlineKeyboardButton::callback(
                    checked_label(short_model_label(model), session.settings.model == model),
                    format!("cfg:model:{}", model_alias(model)),
                )
            })
            .collect(),
        SettingMenu::Effort => session
            .settings
            .model
            .supported_efforts()
            .into_iter()
            .map(|effort| {
                InlineKeyboardButton::callback(
                    checked_label(effort.label(), session.settings.reasoning_effort == effort),
                    format!("cfg:effort:{}", effort.id()),
                )
            })
            .collect(),
        SettingMenu::Fast => [false, true]
            .into_iter()
            .filter(|enabled| !*enabled || session.settings.model.supports_fast())
            .map(|enabled| {
                InlineKeyboardButton::callback(
                    checked_label(
                        if enabled { "开启" } else { "关闭" },
                        session.settings.fast_mode == enabled,
                    ),
                    format!("cfg:fast:{}", if enabled { "on" } else { "off" }),
                )
            })
            .collect(),
        SettingMenu::Verbosity => [
            ResponseVerbosity::Low,
            ResponseVerbosity::Medium,
            ResponseVerbosity::High,
        ]
        .into_iter()
        .map(|verbosity| {
            InlineKeyboardButton::callback(
                checked_label(verbosity.label(), session.settings.verbosity == verbosity),
                format!("cfg:verbosity:{}", verbosity.id()),
            )
        })
        .collect(),
    };
    let rows = buttons
        .chunks(2)
        .map(<[InlineKeyboardButton]>::to_vec)
        .collect::<Vec<_>>();
    InlineKeyboardMarkup::new(rows)
}

fn checked_label(label: &str, selected: bool) -> String {
    if selected {
        format!("✓ {label}")
    } else {
        label.to_owned()
    }
}

fn short_model_label(model: ModelChoice) -> &'static str {
    match model {
        ModelChoice::Sol => "Sol",
        ModelChoice::Terra => "Terra",
        ModelChoice::Luna => "Luna",
        ModelChoice::Gpt55 => "GPT-5.5",
        ModelChoice::Gpt52 => "GPT-5.2",
    }
}

async fn register_commands(bot: &Bot) -> Result<()> {
    bot.set_my_commands(vec![
        BotCommand::new("clear", "清除上下文并开启新对话"),
        BotCommand::new("new", "开启全新对话"),
        BotCommand::new("model", "查看或切换模型"),
        BotCommand::new("effort", "查看或切换推理强度"),
        BotCommand::new("fast", "查看或切换快速模式"),
        BotCommand::new("verbosity", "查看或切换回答详细度"),
        BotCommand::new("settings", "查看当前 Codex 设置"),
        BotCommand::new("defaults", "恢复默认 Codex 设置"),
        BotCommand::new("services", "查看已部署服务"),
        BotCommand::new("status", "查看服务器状态"),
        BotCommand::new("help", "查看帮助"),
    ])
    .await
    .context("register Telegram commands")?;
    Ok(())
}

fn parse_command(text: &str) -> Option<(String, String)> {
    let mut parts = text.split_whitespace();
    let first = parts.next()?;
    let command = first.strip_prefix('/')?;
    let command = command.split('@').next()?.to_ascii_lowercase();
    let argument = parts.collect::<Vec<_>>().join(" ");
    Some((command, argument))
}

fn model_alias(model: ModelChoice) -> &'static str {
    match model {
        ModelChoice::Sol => "sol",
        ModelChoice::Terra => "terra",
        ModelChoice::Luna => "luna",
        ModelChoice::Gpt55 => "5.5",
        ModelChoice::Gpt52 => "5.2",
    }
}

fn settings_summary(session: &SessionState) -> String {
    format!(
        "## ⚙️ Codex 设置\n\n- **模型**：{}\n- **推理强度**：{}\n- **Fast 模式**：{}\n- **回答详细度**：{}\n- **对话上下文**：{}\n\n> 只有 `/clear`、`/new` 会清空上下文；切换模型或强度不会清空。",
        session.settings.model.label(),
        session.settings.reasoning_effort.label(),
        on_off(session.settings.fast_mode),
        session.settings.verbosity.label(),
        if session.thread_id.is_some() {
            "已建立"
        } else {
            "等待第一条消息"
        }
    )
}

fn on_off(enabled: bool) -> &'static str {
    if enabled { "开启" } else { "关闭" }
}

fn spawn_typing(bot: Bot, chat_id: ChatId, done: Arc<AtomicBool>) -> tokio::task::JoinHandle<()> {
    tokio::spawn(async move {
        while !done.load(Ordering::Relaxed) {
            let _ = bot.send_chat_action(chat_id, ChatAction::Typing).await;
            tokio::time::sleep(Duration::from_secs(4)).await;
        }
    })
}

async fn send_long(bot: &Bot, chat_id: ChatId, text: &str) -> Result<()> {
    for chunk in telegram_format::format_chunks(text, 3800) {
        let result = bot
            .send_message(chat_id, &chunk.html)
            .parse_mode(ParseMode::Html)
            .await;
        match result {
            Ok(_) => {}
            Err(error) if is_formatting_error(&error.to_string()) => {
                log::warn!("Telegram rich-text rendering failed; using plain text: {error}");
                bot.send_message(chat_id, chunk.plain).await?;
            }
            Err(error) => return Err(error.into()),
        }
        tokio::time::sleep(Duration::from_millis(150)).await;
    }
    Ok(())
}

fn is_formatting_error(error: &str) -> bool {
    let error = error.to_ascii_lowercase();
    error.contains("parse entities")
        || error.contains("can't find end tag")
        || error.contains("unsupported start tag")
        || error.contains("message is too long")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_commands_with_bot_suffix() {
        assert_eq!(
            parse_command("/model@or_codex_bot terra"),
            Some(("model".to_owned(), "terra".to_owned()))
        );
        assert_eq!(
            parse_command("/clear"),
            Some(("clear".to_owned(), String::new()))
        );
        assert_eq!(parse_command("hello"), None);
    }

    #[test]
    fn parses_only_valid_setting_callbacks() {
        assert_eq!(
            parse_setting_callback("cfg:model:terra"),
            Some(SettingSelection::Model(ModelChoice::Terra))
        );
        assert_eq!(
            parse_setting_callback("cfg:effort:xhigh"),
            Some(SettingSelection::Effort(ReasoningEffort::Xhigh))
        );
        assert_eq!(
            parse_setting_callback("cfg:fast:on"),
            Some(SettingSelection::Fast(true))
        );
        assert_eq!(parse_setting_callback("cfg:fast:maybe"), None);
        assert_eq!(parse_setting_callback("cfg:model:sol:extra"), None);
        assert_eq!(parse_setting_callback("other:model:sol"), None);
    }

    #[test]
    fn effort_keyboard_respects_model_capabilities() {
        let mut session = SessionState::default();
        session.settings.model = ModelChoice::Gpt55;
        let keyboard = setting_keyboard(SettingMenu::Effort, &session);
        let labels = keyboard
            .inline_keyboard
            .iter()
            .flatten()
            .map(|button| button.text.as_str())
            .collect::<Vec<_>>();
        assert_eq!(labels.len(), 4);
        assert!(labels.iter().any(|label| label.contains("XHigh")));
        assert!(!labels.iter().any(|label| label.contains("Max")));
        assert!(!labels.iter().any(|label| label.contains("Ultra")));
    }
}
