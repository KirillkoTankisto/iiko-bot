use crate::date::moscow_time;
use crate::iiko::{Dates, GetShifts, Olap, Server};
use crate::olap::{Filter, OlapMap, PeriodType, ReportConfig, ReportType};
use crate::{Cfg, ServerState, shared::read_to_struct};

use std::collections::HashMap;
use std::vec;
use std::{error::Error, sync::Arc};

use serde::{Deserialize, Serialize};

use teloxide::dispatching::dialogue::InMemStorage;
use teloxide::dispatching::{HandlerExt, UpdateFilterExt};
use teloxide::payloads::{SendMessageSetters, SetChatMenuButtonSetters};
use teloxide::prelude::{Dialogue, Dispatcher, Request, Requester, ResponseResult};
use teloxide::types::{BotCommand, KeyboardButton, KeyboardMarkup, Update};
use teloxide::{Bot, dptree};
use teloxide::{
    types::{Message, ParseMode},
    utils::command::BotCommands,
    utils::markdown::escape,
};

use tokio::fs;
use tokio::io::AsyncWriteExt;
use tokio::sync::Mutex;

type SharedOlap = Arc<Mutex<OlapMap>>;

fn format_with_dots(number: usize) -> String {
    let number_string = number.to_string();
    let length = number_string.len();
    let mut result = String::with_capacity(length + (length - 1) / 3);

    for (size, character) in number_string.chars().enumerate() {
        let rem = length - size;

        if size > 0 && rem % 3 == 0 {
            result.push('.');
        }

        result.push(character);
    }

    result
}

async fn collect_server_info(
    servers: Arc<Mutex<ServerState>>,
    config: Cfg,
) -> (String, String, String, String) {
    let (login, pass) = (config.login, config.pass);

    let servers = servers.lock().await;
    let server_url = servers.map.get(&servers.current).unwrap().to_owned();

    (login, pass, server_url, servers.current.clone())
}

#[derive(Deserialize, Serialize)]
struct TgCfg {
    token: String,
    accounts: Vec<String>,
    admins: Vec<String>,
}

#[derive(BotCommands, Clone)]
#[command(rename_rule = "lowercase", description = "Поддерживаемые команды:")]
enum Command {
    #[command(description = "Запустить бота")]
    Start,
    #[command(description = "Отобразить список команд")]
    Help,
    #[command(description = "Сегодняшняя выручка")]
    Today,
    #[command(description = "Вчерашняя выручка")]
    Yesterday,
    #[command(description = "Выручка за 7 дней")]
    Week,
    #[command(description = "Выручка за данный месяц")]
    Month,
    #[command(description = "Переключиться на другой сервер")]
    Switch,
    #[command(description = "Вывести список доступных серверов")]
    List,
    #[command(description = "Режим Olap отчёта")]
    Olap,
    #[command(description = "Добавить пользователя")]
    Adduser,
    #[command(description = "Удалить пользователя")]
    Deleteuser,
    #[command(description = "Список пользователей")]
    Listusers,
    #[command(description = "Список админов")]
    Listadmins,
}

#[derive(Clone, Default)]
enum State {
    #[default]
    None,
    Switch,
    Olap,
    AddUser,
    DeleteUser,
    Dialogue,
    Report,
    Admin,
}

#[derive(Clone)]
struct DependenciesForDispatcher {
    config: Cfg,
    allowed_list: Arc<Mutex<Vec<String>>>,
    admins_list: Arc<Vec<String>>,
    servers: Arc<Mutex<ServerState>>,
    olap_store: SharedOlap,
}

pub async fn initialise() -> Result<(), Box<dyn Error>> {
    let telegram_config: TgCfg = read_to_struct("/etc/iiko-bot/tg_cfg.toml").await?;
    let (token, accounts, admins) = (
        telegram_config.token,
        telegram_config.accounts,
        telegram_config.admins,
    );

    let allowed = Arc::new(Mutex::new(accounts));
    let admins = Arc::new(admins);

    let main_config: Cfg = read_to_struct("/etc/iiko-bot/cfg.toml").await?;
    let servers = main_config.servers.clone();
    let first = servers.keys().next().expect("Список серверов пуст").clone();

    let state = ServerState {
        map: servers,
        current: first,
    };

    let olap_store: SharedOlap = Arc::new(Mutex::new(HashMap::new()));

    let servers = Arc::new(Mutex::new(state));

    let bot = Bot::new(token);

    let handler = Update::filter_message()
        .enter_dialogue::<Message, InMemStorage<State>, State>()
        .endpoint(handle_states);

    let deps = DependenciesForDispatcher {
        config: main_config.clone(),
        allowed_list: allowed.clone(),
        admins_list: admins.clone(),
        servers: servers.clone(),
        olap_store: olap_store.clone(),
    };

    Dispatcher::builder(bot.clone(), handler)
        .dependencies(dptree::deps![
            deps.clone(),
            InMemStorage::<State>::new(),
            State::None
        ])
        .build()
        .dispatch()
        .await;

    Ok(())
}

async fn is_allowed(allowed_list: Arc<Mutex<Vec<String>>>, username: &String) -> bool {
    allowed_list.lock().await.contains(username)
}

fn is_admin(admins_list: Arc<Vec<String>>, username: &String) -> bool {
    admins_list.contains(username)
}

async fn handle_start(
    bot: Bot,
    message: Message,
    dialogue: MyDialogue,
    allowed_list: Arc<Mutex<Vec<String>>>,
    admins_list: Arc<Vec<String>>,
) -> Result<(), Box<dyn Error>> {
    let username = &message
        .from
        .ok_or("Не удалось определить отправителя")?
        .username
        .ok_or("Не удалось получить ник")?;

    if !is_allowed(allowed_list, &username).await && !is_admin(admins_list, &username) {
        bot.send_message(message.chat.id, "Вы не в списке пользователей")
            .await?;
        return Ok(());
    }

    let commands: Vec<BotCommand> = Command::bot_commands();

    bot.set_my_commands(commands).await?;

    bot.set_chat_menu_button()
        .chat_id(message.chat.id)
        .menu_button(teloxide::types::MenuButton::Commands)
        .send()
        .await?;

    let buttons: Vec<KeyboardButton> = vec![
        KeyboardButton::new("Отчёты"),
        KeyboardButton::new("Сменить сервер"),
    ];

    let buttons2: Vec<KeyboardButton> = vec![
        KeyboardButton::new("Список серверов"),
        KeyboardButton::new("Администрирование"),
    ];

    let keyboard = KeyboardMarkup::default()
        .append_row(buttons)
        .append_row(buttons2)
        .one_time_keyboard();

    bot.send_message(message.chat.id, "Выберите опцию")
        .reply_markup(keyboard)
        .await?;

    dialogue.update(State::Dialogue).await?;

    Ok(())
}

async fn handle_dialogue(
    bot: Bot,
    message: Message,
    dialogue: MyDialogue,
    servers: Arc<Mutex<ServerState>>,
    allowed_list: Arc<Mutex<Vec<String>>>,
    admins_list: Arc<Vec<String>>,
) -> Result<(), Box<dyn Error>> {
    if let Some(text) = message.text() {
        let result = match text {
            "Отчёты" => list_reports(bot, message, dialogue).await,
            "Сменить сервер" => handle_switch(bot, message, servers, dialogue).await,
            "Список серверов" => {
                handle_list(bot, message, dialogue, servers, allowed_list, admins_list).await
            }
            "Администрирование" => {
                handle_admin(bot, message, dialogue, allowed_list, admins_list).await
            }
            _ => handle_start(bot, message, dialogue, allowed_list, admins_list).await,
        };

        match result {
            Ok(_) => {}
            Err(e) => eprintln!("Ошибка: {e}"),
        }
    }

    Ok(())
}

async fn handle_admin(
    bot: Bot,
    message: Message,
    dialogue: MyDialogue,
    allowed_list: Arc<Mutex<Vec<String>>>,
    admins_list: Arc<Vec<String>>,
) -> Result<(), Box<dyn Error>> {
    let username = message
        .from
        .clone()
        .ok_or("Не удалось определить отправителя")?
        .username
        .ok_or("Не удалось получить ник")?;

    if !is_admin(admins_list.clone(), &username) {
        bot.send_message(message.chat.id, "Вы не находитесь в списке админов")
            .await?;
        handle_start(bot, message, dialogue, allowed_list, admins_list).await?;
        return Ok(());
    };

    let buttons: Vec<KeyboardButton> = vec![
        KeyboardButton::new("Добавить пользователя"),
        KeyboardButton::new("Удалить пользователя"),
    ];

    let buttons2: Vec<KeyboardButton> = vec![
        KeyboardButton::new("Список пользователей"),
        KeyboardButton::new("Список админов"),
    ];

    let buttons3: Vec<KeyboardButton> = vec![KeyboardButton::new("Назад")];

    let keyboard = KeyboardMarkup::default()
        .append_row(buttons)
        .append_row(buttons2)
        .append_row(buttons3)
        .one_time_keyboard();

    bot.send_message(message.chat.id, "Выберите опцию")
        .reply_markup(keyboard)
        .await?;

    dialogue.update(State::Admin).await?;

    Ok(())
}

async fn callback_admin(
    bot: Bot,
    message: Message,
    dialogue: MyDialogue,
    allowed_list: Arc<Mutex<Vec<String>>>,
    admins_list: Arc<Vec<String>>,
) -> Result<(), Box<dyn Error>> {
    if let Some(text) = message.text() {
        match text {
            "Добавить пользователя" => {
                handle_add_user(bot, message, dialogue).await?
            }

            "Удалить пользователя" => {
                handle_delete_user(bot, message, allowed_list, dialogue).await?
            }

            "Список пользователей" => {
                handle_list_users(bot, message, dialogue, allowed_list, admins_list).await?
            }

            "Список админов" => {
                handle_list_admins(bot, message, dialogue, allowed_list, admins_list).await?
            }

            "Назад" => handle_start(bot, message, dialogue, allowed_list, admins_list).await?,

            _ => {}
        };
    }

    Ok(())
}

async fn list_reports(
    bot: Bot,
    message: Message,
    dialogue: MyDialogue,
) -> Result<(), Box<dyn Error>> {
    let buttons: Vec<KeyboardButton> = vec![
        KeyboardButton::new("За сегодня"),
        KeyboardButton::new("За вчера"),
    ];

    let buttons2: Vec<KeyboardButton> = vec![
        KeyboardButton::new("За 7 дней"),
        KeyboardButton::new("За текущий месяц"),
    ];

    let buttons3: Vec<KeyboardButton> = vec![KeyboardButton::new("Olap отчёт")];

    let buttons4: Vec<KeyboardButton> = vec![KeyboardButton::new("Назад")];

    let keyboard = KeyboardMarkup::default()
        .append_row(buttons)
        .append_row(buttons2)
        .append_row(buttons3)
        .append_row(buttons4)
        .one_time_keyboard();

    bot.send_message(message.chat.id, "Выберите опцию")
        .reply_markup(keyboard)
        .await?;

    dialogue.update(State::Report).await?;

    Ok(())
}

async fn handle_reports(
    bot: Bot,
    message: Message,
    dialogue: MyDialogue,
    deps: DependenciesForDispatcher,
) -> Result<(), Box<dyn Error>> {
    let (bot_cloned, message_cloned, dialogue_cloned) =
        (bot.clone(), message.clone(), dialogue.clone());

    if let Some(text) = message.text() {
        match text {
            "За сегодня" => {
                handle_today(bot, message, deps.servers, deps.config).await?;
                handle_start(
                    bot_cloned,
                    message_cloned,
                    dialogue_cloned,
                    deps.allowed_list,
                    deps.admins_list,
                )
                .await?;
            }

            "За вчера" => {
                handle_yesterday(bot, message, deps.servers, deps.config).await?;
                handle_start(
                    bot_cloned,
                    message_cloned,
                    dialogue_cloned,
                    deps.allowed_list,
                    deps.admins_list,
                )
                .await?;
            }
            "За 7 дней" => {
                handle_week(bot, message, deps.servers, deps.config).await?;
                handle_start(
                    bot_cloned,
                    message_cloned,
                    dialogue_cloned,
                    deps.allowed_list,
                    deps.admins_list,
                )
                .await?;
            }

            "За текущий месяц" => {
                handle_month(bot, message, deps.servers, deps.config).await?;
                handle_start(
                    bot_cloned,
                    message_cloned,
                    dialogue_cloned,
                    deps.allowed_list,
                    deps.admins_list,
                )
                .await?;
            }
            "Olap отчёт" => {
                handle_olap(
                    bot,
                    message,
                    deps.servers,
                    deps.config,
                    deps.olap_store,
                    dialogue,
                )
                .await?
            }

            "Назад" => {
                handle_start(bot, message, dialogue, deps.allowed_list, deps.admins_list).await?
            }
            _ => {}
        };
    }

    Ok(())
}

async fn handle_states(
    bot: Bot,
    message: Message,
    dialogue: MyDialogue,
    deps: DependenciesForDispatcher,
) -> ResponseResult<()> {
    let state = dialogue.get().await.unwrap().unwrap();

    let result = match state {
        State::AddUser => {
            handle_add_user_dialogue(bot, message, deps.allowed_list, dialogue, deps.admins_list)
                .await
        }
        State::DeleteUser => {
            callback_delete_user(bot, message, dialogue, deps.allowed_list, deps.admins_list).await
        }

        State::Olap => {
            callback_olap(
                bot,
                message,
                deps.olap_store,
                dialogue,
                deps.allowed_list,
                deps.admins_list,
            )
            .await
        }

        State::Switch => {
            callback_switch(
                bot,
                message,
                deps.servers,
                dialogue,
                deps.allowed_list,
                deps.admins_list,
            )
            .await
        }

        State::Dialogue => {
            handle_dialogue(
                bot,
                message,
                dialogue,
                deps.servers,
                deps.allowed_list,
                deps.admins_list,
            )
            .await
        }

        State::Report => handle_reports(bot, message, dialogue, deps.clone()).await,

        State::Admin => {
            callback_admin(bot, message, dialogue, deps.allowed_list, deps.admins_list).await
        }
        State::None => {
            handle_start(bot, message, dialogue, deps.allowed_list, deps.admins_list).await
        }
    };

    if let Err(e) = result {
        eprintln!("Ошибка: {e}")
    }

    Ok(())
}

async fn handle_today(
    bot: Bot,
    message: Message,
    servers: Arc<Mutex<ServerState>>,
    config: Cfg,
) -> Result<(), Box<dyn Error>> {
    let (login, pass, server_url, current_server) = collect_server_info(servers, config).await;

    let mut server = Server::new(login, pass, server_url.into());

    let shifts = Server::list_shifts_with_offset(&mut server, Dates::Week, 0).await?;

    server.deauth().await?;

    let offset: usize = 0;

    let shift = Server::latest_shift(shifts, offset)?;

    let text = format!(
        "*Сервер*: *{}*\n\
                 *Текущая смена*:\n\
                 Номер смены: *{}*\n\
                 Статус: *{}*\n\
                 Оплачено картой: *{}*\n\
                 Оплачено наличкой: *{}*\n\
                 Итог: *{}*",
        current_server,
        escape(&format_with_dots(shift.session_number)),
        shift.session_status.to_string(),
        escape(&format_with_dots(shift.sales_card as usize)),
        escape(&format_with_dots(shift.sales_cash)),
        escape(&format_with_dots(shift.pay_orders as usize)),
    );

    bot.send_message(message.chat.id, text)
        .parse_mode(ParseMode::MarkdownV2)
        .await?;

    Ok(())
}

async fn handle_yesterday(
    bot: Bot,
    message: Message,
    servers: Arc<Mutex<ServerState>>,
    config: Cfg,
) -> Result<(), Box<dyn Error>> {
    let (login, pass, server_url, current_server) = collect_server_info(servers, config).await;

    let mut server = Server::new(login, pass, server_url.into());

    let shifts = Server::list_shifts_with_offset(&mut server, Dates::Week, 0).await?;
    server.deauth().await?;

    let offset: usize = 1;
    let shift = Server::latest_shift(shifts, offset)?;

    let text = format!(
        "*Сервер*: *{}*\n\
                 *Предыдущая смена*:\n\
                 Номер смены: *{}*\n\
                 Статус: *{}*\n\
                 Оплачено картой: *{}*\n\
                 Оплачено наличкой: *{}*\n\
                 Итог: *{}*",
        current_server,
        escape(&format_with_dots(shift.session_number)),
        shift.session_status.to_string(),
        escape(&format_with_dots(shift.sales_card as usize)),
        escape(&format_with_dots(shift.sales_cash)),
        escape(&format_with_dots(shift.pay_orders as usize)),
    );

    bot.send_message(message.chat.id, text)
        .parse_mode(ParseMode::MarkdownV2)
        .await?;

    Ok(())
}

async fn handle_week(
    bot: Bot,
    message: Message,
    servers: Arc<Mutex<ServerState>>,
    config: Cfg,
) -> Result<(), Box<dyn Error>> {
    let (login, pass, server_url, current_server) = collect_server_info(servers, config).await;

    let mut server = Server::new(login, pass, server_url.into());

    let shifts = Server::list_shifts_with_offset(&mut server, Dates::Week, 0).await?;
    server.deauth().await?;

    let sum = Server::sum_shifts(shifts);

    let text = format!(
        "*Сервер*: *{}*\n*Сумма за прошедшие 7 дней*: *{}*",
        current_server,
        escape(&format_with_dots(sum as usize))
    );

    bot.send_message(message.chat.id, text)
        .parse_mode(ParseMode::MarkdownV2)
        .await?;

    Ok(())
}

async fn handle_month(
    bot: Bot,
    message: Message,
    servers: Arc<Mutex<ServerState>>,
    config: Cfg,
) -> Result<(), Box<dyn Error>> {
    let (login, pass, server_url, current_server) = collect_server_info(servers, config).await;

    let mut server = Server::new(login, pass, server_url.into());

    let shifts = Server::list_shifts_with_offset(&mut server, Dates::ThisMonth, 0).await?;
    server.deauth().await?;

    let sum = Server::sum_shifts(shifts);

    let text = format!(
        "*Сервер*: *{}*\n*Сумма за текущий месяц*: *{}*",
        current_server,
        escape(&format_with_dots(sum as usize))
    );

    bot.send_message(message.chat.id, text)
        .parse_mode(ParseMode::MarkdownV2)
        .await?;

    Ok(())
}

async fn handle_switch(
    bot: Bot,
    message: Message,
    servers: Arc<Mutex<ServerState>>,
    dialogue: MyDialogue,
) -> Result<(), Box<dyn Error>> {
    let (current_server, server_keys) = {
        let server = servers.lock().await;
        let current_server = server.current.clone();
        let keys = server.map.keys().cloned().collect::<Vec<_>>();
        (current_server, keys)
    };

    let mut keyboard = KeyboardMarkup::default().one_time_keyboard();
    for key in server_keys {
        keyboard = keyboard.append_row(vec![KeyboardButton::new(key.clone())]);
    }

    let text = format!("Текущий сервер: *{}*", current_server);

    bot.send_message(message.chat.id, text)
        .parse_mode(ParseMode::MarkdownV2)
        .reply_markup(keyboard)
        .await?;

    dialogue.update(State::Switch).await?;

    Ok(())
}

async fn handle_list(
    bot: Bot,
    message: Message,
    dialogue: MyDialogue,
    servers: Arc<Mutex<ServerState>>,
    allowed_list: Arc<Mutex<Vec<String>>>,
    admins_list: Arc<Vec<String>>,
) -> Result<(), Box<dyn Error>> {
    let text = servers
        .lock()
        .await
        .map
        .iter()
        .map(|server| format!("{} -> {}", server.0, server.1))
        .collect::<Vec<String>>()
        .join("\n");

    let text = format!(
        "*Список серверов*:\n{}\n*Выбранный сервер*: *{}*",
        escape(&text),
        servers.lock().await.current
    );

    bot.send_message(message.chat.id, text)
        .parse_mode(ParseMode::MarkdownV2)
        .await?;

    handle_start(bot, message, dialogue, allowed_list, admins_list).await?;

    Ok(())
}

async fn handle_olap(
    bot: Bot,
    message: Message,
    servers: Arc<Mutex<ServerState>>,
    config: Cfg,
    olap_store: SharedOlap,
    dialogue: MyDialogue,
) -> Result<(), Box<dyn Error>> {
    let (login, pass, server_url, current_server) =
        collect_server_info(servers.clone(), config.clone()).await;
    let mut server = Server::new(login, pass, server_url.clone().into());

    let form = ReportConfig {
        report_type: ReportType::SALES,
        group_by_row_fields: vec!["DishCategory".into()],
        group_by_col_fields: vec!["DishName".into()],
        aggregate_fields: vec!["GuestNum".into(), "DishDiscountSumInt".into()],
        filters: {
            let mut m = HashMap::new();
            m.insert(
                "OpenDate.Typed".into(),
                Filter::DateRange {
                    periodType: PeriodType::CURRENT_MONTH,
                    to: moscow_time().0,
                },
            );
            m.insert(
                "DeletedWithWriteoff".into(),
                Filter::IncludeValues {
                    values: vec!["NOT_DELETED".into()],
                },
            );
            m.insert(
                "OrderDeleted".into(),
                Filter::IncludeValues {
                    values: vec!["NOT_DELETED".into()],
                },
            );
            m
        },
    };

    let form_json = serde_json::to_string_pretty(&form)?;

    let token = server.get_token().await?;

    let olap = Server::get_olap(form_json, server_url, token).await?;

    server.deauth().await?;

    *olap_store.lock().await = olap.clone();

    if olap.is_empty() {
        bot.send_message(message.chat.id, "По вашим фильтрам ничего не найдено.")
            .await?;
        return Ok(());
    }

    let buttons: Vec<KeyboardButton> = olap.keys().map(|key| KeyboardButton::new(key)).collect();

    let rows: Vec<Vec<KeyboardButton>> = buttons
        .chunks(2) // create slices of up to 2 items
        .map(|chunk| chunk.to_vec()) // turn each slice into a Vec<Button>
        .collect();

    let keyboard = KeyboardMarkup::new(rows).one_time_keyboard();

    let text = format!("Режим Olap отчёта\\. Текущий сервер: *{}*", current_server);

    bot.send_message(message.chat.id, text)
        .parse_mode(ParseMode::MarkdownV2)
        .reply_markup(keyboard)
        .await?;

    dialogue.update(State::Olap).await?;

    Ok(())
}

/*
    Дальше идут команды для админов
*/

// /adduser, здесь несколько функций

async fn handle_add_user(
    bot: Bot,
    message: Message,
    dialogue: MyDialogue,
) -> Result<(), Box<dyn Error>> {
    bot.send_message(message.chat.id, "Введите имя пользователя")
        .await?;

    dialogue.update(State::AddUser).await?;

    Ok(())
}

type MyDialogue = Dialogue<State, InMemStorage<State>>;

async fn handle_add_user_dialogue(
    bot: Bot,
    message: Message,
    allowed_list: Arc<Mutex<Vec<String>>>,
    dialogue: MyDialogue,
    admins_list: Arc<Vec<String>>,
) -> Result<(), Box<dyn Error>> {
    let username = message
        .text()
        .ok_or("Ну удалось получить текст сообщения")?;

    if username.is_empty() {
        bot.send_message(message.chat.id, "Вы не ввели имя пользователя.")
            .await?;
        return Ok(());
    }

    let stripped = username.strip_prefix('@').unwrap_or(&username);

    {
        let mut accounts = allowed_list.lock().await;
        if !accounts.contains(&stripped.to_string()) {
            accounts.push(stripped.to_string());
        }
    }

    let mut telegram_config: TgCfg = read_to_struct("/etc/iiko-bot/tg_cfg.toml").await?;

    telegram_config.accounts.push(stripped.into());

    let mut file = fs::File::create("/etc/iiko-bot/tg_cfg.toml").await?;

    let config = toml::to_string(&telegram_config)?;

    file.write_all(config.as_bytes()).await?;

    dialogue.update(State::None).await?;

    let text = format!("Пользователь @{} успешно добавлен", stripped);

    bot.send_message(message.chat.id, text).await?;

    handle_start(bot, message, dialogue, allowed_list, admins_list).await?;

    Ok(())
}

// Конец /adduser

async fn handle_delete_user(
    bot: Bot,
    message: Message,
    allowed_list: Arc<Mutex<Vec<String>>>,
    dialogue: MyDialogue,
) -> Result<(), Box<dyn Error>> {
    let accounts = allowed_list.lock().await;

    let buttons: Vec<KeyboardButton> = accounts
        .iter()
        .cloned()
        .map(|account| KeyboardButton::new(account))
        .collect();

    let rows: Vec<Vec<KeyboardButton>> = buttons.chunks(2).map(|chunk| chunk.to_vec()).collect();

    let keyboard = KeyboardMarkup::new(rows).one_time_keyboard();

    let text = format!("Выберите аккаунт для удаления");

    match bot
        .send_message(message.chat.id, text)
        .parse_mode(ParseMode::MarkdownV2)
        .reply_markup(keyboard)
        .await
    {
        Ok(_) => (),
        Err(e) => eprintln!("{:?}", e),
    };

    dialogue.update(State::DeleteUser).await?;

    Ok(())
}

async fn handle_list_users(
    bot: Bot,
    message: Message,
    dialogue: MyDialogue,
    allowed_list: Arc<Mutex<Vec<String>>>,
    admins_list: Arc<Vec<String>>,
) -> Result<(), Box<dyn Error>> {
    let accounts = allowed_list.lock().await;

    let list = accounts.iter().cloned().collect::<Vec<String>>().join("\n");

    drop(accounts);

    let text = format!("Список пользователей:\n{}", list);

    bot.send_message(message.chat.id, text).await?;

    handle_start(
        bot,
        message,
        dialogue,
        Arc::clone(&allowed_list),
        admins_list,
    )
    .await?;

    Ok(())
}

async fn handle_list_admins(
    bot: Bot,
    message: Message,
    dialogue: MyDialogue,
    allowed_list: Arc<Mutex<Vec<String>>>,
    admins_list: Arc<Vec<String>>,
) -> Result<(), Box<dyn Error>> {
    let list = admins_list
        .iter()
        .cloned()
        .collect::<Vec<String>>()
        .join("\n");

    let text = format!("Список админов:\n{}", list);

    bot.send_message(message.chat.id, text).await?;

    handle_start(
        bot,
        message,
        dialogue,
        Arc::clone(&allowed_list),
        admins_list,
    )
    .await?;

    Ok(())
}

async fn callback_switch(
    bot: Bot,
    message: Message,
    servers: Arc<Mutex<ServerState>>,
    dialogue: MyDialogue,
    allowed_list: Arc<Mutex<Vec<String>>>,
    admins_list: Arc<Vec<String>>,
) -> Result<(), Box<dyn Error>> {
    let data = message
        .text()
        .ok_or("Невозможно получить текст сообщения")?;

    let mut server = servers.lock().await;

    if let Some(url) = server.map.get(data).cloned() {
        server.current = data.to_string();
        bot.send_message(
            message.chat.id,
            format!("Текущий сервер теперь '{}' -> {}", data, url),
        )
        .await?;
    }

    dialogue.update(State::None).await?;

    handle_start(bot, message, dialogue, allowed_list, admins_list).await?;

    Ok(())
}

async fn callback_olap(
    bot: Bot,
    message: Message,
    olap_store: SharedOlap,
    dialogue: MyDialogue,
    allowed_list: Arc<Mutex<Vec<String>>>,
    admins_list: Arc<Vec<String>>,
) -> Result<(), Box<dyn Error>> {
    let data = message
        .text()
        .ok_or("Невозможно получить текст сообщения")?;

    let olap = olap_store.lock().await;

    if let Some(olap_elements) = olap.get(data) {
        let text = Server::display_olap(&olap_elements);

        bot.send_message(message.chat.id, text)
            .parse_mode(ParseMode::MarkdownV2)
            .await?;
    }

    dialogue.update(State::None).await?;

    handle_start(bot, message, dialogue, allowed_list, admins_list).await?;

    Ok(())
}

async fn callback_delete_user(
    bot: Bot,
    message: Message,
    dialogue: MyDialogue,
    allowed: Arc<Mutex<Vec<String>>>,
    admins_list: Arc<Vec<String>>,
) -> Result<(), Box<dyn Error>> {
    let data = message
        .text()
        .ok_or("Невозможно получить текст сообщения")?
        .to_string();

    let removed = {
        let mut accounts = allowed.lock().await;
        if accounts.contains(&data) {
            accounts.retain(|account| account != &data);
            true
        } else {
            false
        }
    };

    if removed {
        let mut telegram_config: TgCfg = read_to_struct("/etc/iiko-bot/tg_cfg.toml").await?;
        telegram_config.accounts.retain(|account| account != &data);

        let mut file = fs::File::create("/etc/iiko-bot/tg_cfg.toml").await?;
        let config = toml::to_string(&telegram_config)?;
        file.write_all(config.as_bytes()).await?;

        let text = format!("Пользователь @{} успешно удалён", data);
        bot.send_message(message.chat.id, text).await?;
    }

    dialogue.update(State::None).await?;

    let allowed_clone = Arc::clone(&allowed);

    if let Err(e) = handle_start(bot, message, dialogue, allowed_clone, admins_list).await {
        eprintln!("Ошибка: {e}");
    }

    Ok(())
}
