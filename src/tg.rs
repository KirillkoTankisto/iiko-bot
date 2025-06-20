use crate::date::moscow_time;
use crate::new::{Dates, GetShifts, Olap, Server};
use crate::olap::{Filter, OlapMap, PeriodType, ReportConfig, ReportType};
use crate::{Cfg, ServerState, shared::read_to_struct};

use std::collections::HashMap;
use std::{error::Error, sync::Arc};

use serde::{Deserialize, Serialize};

use teloxide::dispatching::{HandlerExt, UpdateFilterExt};
use teloxide::payloads::{SendMessageSetters, SetChatMenuButtonSetters};
use teloxide::prelude::{Dispatcher, Request, Requester, ResponseResult};
use teloxide::types::{BotCommand, MaybeInaccessibleMessage, Update};
use teloxide::{Bot, dptree};
use teloxide::{
    types::{CallbackQuery, InlineKeyboardButton, InlineKeyboardMarkup, Message, ParseMode},
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
    Adduser(String),
    #[command(description = "Удалить пользователя")]
    Deleteuser,
    #[command(description = "Список пользователей")]
    Listusers,
    #[command(description = "Список админов")]
    Listadmins,
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

    let handler = dptree::entry()
        .branch(
            Update::filter_message()
                .filter_command::<Command>()
                .endpoint(handle_command),
        )
        .branch(Update::filter_callback_query().endpoint(handle_callback));

    Dispatcher::builder(bot.clone(), handler)
        .dependencies(dptree::deps![
            main_config.clone(),
            allowed.clone(),
            admins.clone(),
            servers.clone(),
            olap_store.clone()
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

async fn handle_command(
    bot: Bot,
    message: Message,
    command: Command,
    config: Cfg,
    allowed_list: Arc<Mutex<Vec<String>>>,
    admins_list: Arc<Vec<String>>,
    servers: Arc<Mutex<ServerState>>,
    olap_store: SharedOlap,
) -> ResponseResult<()> {
    let username = message
        .clone()
        .from
        .and_then(|u| u.username.clone())
        .unwrap_or_default();

    if !is_allowed(allowed_list.clone(), &username).await
        & !is_admin(admins_list.clone(), &username)
    {
        bot.send_message(message.clone().chat.id, "У вас нет доступа к этой команде.")
            .await?;
        return Ok(());
    }

    let result = match command {
        Command::Start => handle_start(bot, message).await,
        Command::Help => handle_help(bot, message).await,

        Command::Today => handle_today(bot, message, servers, config).await,
        Command::Yesterday => handle_yesterday(bot, message, servers, config).await,
        Command::Week => handle_week(bot, message, servers, config).await,
        Command::Month => handle_month(bot, message, servers, config).await,

        Command::Switch => handle_switch(bot, message, servers).await,
        Command::List => handle_list(bot, message, servers).await,

        Command::Olap => handle_olap(bot, message, servers, config, olap_store).await,

        Command::Adduser(string) if admins_list.contains(&username) => {
            handle_add_user(bot, message, allowed_list, &string).await
        }
        Command::Deleteuser if admins_list.contains(&username) => {
            handle_delete_user(bot, message, allowed_list).await
        }
        Command::Listusers if admins_list.contains(&username) => {
            handle_list_users(bot, message, allowed_list).await
        }
        Command::Listadmins if admins_list.contains(&username) => {
            handle_list_admins(bot, message, admins_list).await
        }

        _ => handle_error(bot, message).await,
    };

    match result {
        Ok(_) => {}
        Err(e) => eprintln!("Ошибка: {e}"),
    }

    Ok(())
}

async fn handle_start(bot: Bot, message: Message) -> Result<(), Box<dyn Error>> {
    let commands: Vec<BotCommand> = Command::bot_commands();

    bot.set_my_commands(commands).await?;

    bot.set_chat_menu_button()
        .chat_id(message.chat.id)
        .menu_button(teloxide::types::MenuButton::Commands)
        .send()
        .await?;

    bot.send_message(message.chat.id, "Я - Iiko бот для отчётов, отправьте команду /help для вывода доступных команд или зайдите в меню")
        .await?;

    bot.send_message(message.chat.id, Command::descriptions().to_string())
        .await?;
    Ok(())
}

async fn handle_help(bot: Bot, message: Message) -> Result<(), Box<dyn Error>> {
    bot.send_message(message.chat.id, Command::descriptions().to_string())
        .await?;
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
) -> Result<(), Box<dyn Error>> {
    let (current_server, server_keys) = {
        let server = servers.lock().await;
        let current_server = server.current.clone();
        let keys = server.map.keys().cloned().collect::<Vec<_>>();
        (current_server, keys)
    };

    let mut keyboard = InlineKeyboardMarkup::default();
    for key in server_keys {
        keyboard = keyboard.append_row(vec![InlineKeyboardButton::callback(key.clone(), key)]);
    }

    let text = format!("Текущий сервер: *{}*", current_server);

    bot.send_message(message.chat.id, text)
        .parse_mode(ParseMode::MarkdownV2)
        .reply_markup(keyboard)
        .await?;

    Ok(())
}

async fn handle_list(
    bot: Bot,
    message: Message,
    servers: Arc<Mutex<ServerState>>,
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

    Ok(())
}

async fn handle_olap(
    bot: Bot,
    message: Message,
    servers: Arc<Mutex<ServerState>>,
    config: Cfg,
    olap_store: SharedOlap,
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
    let form_json = serde_json::to_string_pretty(&form).unwrap();

    let olap = Server::get_olap(form_json, server_url, server.get_token().await.unwrap())
        .await
        .unwrap_or_default();
    server.deauth().await.unwrap();

    *olap_store.lock().await = olap.clone();

    if olap.is_empty() {
        bot.send_message(message.chat.id, "По вашим фильтрам ничего не найдено.")
            .await?;
        return Ok(());
    }

    let buttons: Vec<InlineKeyboardButton> = olap
        .keys()
        .map(|key| InlineKeyboardButton::callback(key.clone(), key.clone()))
        .collect();

    let rows: Vec<Vec<InlineKeyboardButton>> = buttons
        .chunks(2) // create slices of up to 2 items
        .map(|chunk| chunk.to_vec()) // turn each slice into a Vec<Button>
        .collect();

    let keyboard = InlineKeyboardMarkup::new(rows);

    let text = format!("Режим Olap отчёта\\. Текущий сервер: *{}*", current_server);

    match bot
        .send_message(message.chat.id, text)
        .parse_mode(ParseMode::MarkdownV2)
        .reply_markup(keyboard)
        .await
    {
        Ok(_) => (),
        Err(e) => eprintln!("{:?}", e),
    };

    Ok(())
}

async fn handle_add_user(
    bot: Bot,
    message: Message,
    allowed_list: Arc<Mutex<Vec<String>>>,
    username: &String,
) -> Result<(), Box<dyn Error>> {
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

    let mut telegram_config: TgCfg = read_to_struct("/etc/iiko-bot/tg_cfg.toml").await.unwrap();

    telegram_config.accounts.push(stripped.into());

    let mut file = fs::File::create("/etc/iiko-bot/tg_cfg.toml").await.unwrap();

    let config = toml::to_string(&telegram_config).unwrap();

    file.write_all(config.as_bytes()).await.unwrap();

    Ok(())
}

async fn handle_delete_user(
    bot: Bot,
    message: Message,
    allowed_list: Arc<Mutex<Vec<String>>>,
) -> Result<(), Box<dyn Error>> {
    let accounts = allowed_list.lock().await;

    let buttons: Vec<InlineKeyboardButton> = accounts
        .iter()
        .cloned()
        .map(|account| {
            InlineKeyboardButton::callback(format!("@{}", account.clone()), account.clone())
        })
        .collect();

    let rows: Vec<Vec<InlineKeyboardButton>> =
        buttons.chunks(2).map(|chunk| chunk.to_vec()).collect();

    let keyboard = InlineKeyboardMarkup::new(rows);

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

    Ok(())
}

async fn handle_list_users(
    bot: Bot,
    message: Message,
    allowed_list: Arc<Mutex<Vec<String>>>,
) -> Result<(), Box<dyn Error>> {
    let accounts = allowed_list.lock().await;

    let list = accounts.iter().cloned().collect::<Vec<String>>().join("\n");

    let text = format!("Список пользователей:\n{}", list);

    bot.send_message(message.chat.id, text).await?;

    Ok(())
}

async fn handle_list_admins(
    bot: Bot,
    message: Message,
    admins_list: Arc<Vec<String>>,
) -> Result<(), Box<dyn Error>> {
    let list = admins_list
        .iter()
        .cloned()
        .collect::<Vec<String>>()
        .join("\n");

    let text = format!("Список админов:\n{}", list);

    bot.send_message(message.chat.id, text).await?;

    Ok(())
}

async fn handle_error(bot: Bot, message: Message) -> Result<(), Box<dyn Error>> {
    bot.send_message(message.chat.id, "Эта комманда только для админов!")
        .await?;
    Ok(())
}

async fn handle_callback(
    bot: Bot,
    query: CallbackQuery,
    servers: Arc<Mutex<ServerState>>,
    allowed: Arc<Mutex<Vec<String>>>,
    olap_store: SharedOlap,
) -> ResponseResult<()> {
    if let (Some(data), Some(message)) = (query.data, query.message) {
        match data {
            string_switch if servers.lock().await.map.contains_key(&string_switch) => {
                callback_switch(bot, string_switch, message, servers).await?;
            }

            string_olap if olap_store.lock().await.contains_key(&string_olap) => {
                callback_olap(bot, string_olap, message, olap_store).await?
            }

            string_delete_user if allowed.lock().await.contains(&string_delete_user) => {
                callback_delete_user(bot, string_delete_user, message, allowed).await?
            }
            _ => {}
        }
    }
    Ok(())
}

async fn callback_switch(
    bot: Bot,
    data: String,
    message: MaybeInaccessibleMessage,
    servers: Arc<Mutex<ServerState>>,
) -> ResponseResult<()> {
    let mut server = servers.lock().await;

    if let Some(url) = server.map.get(&data).cloned() {
        server.current = data.clone();
        bot.send_message(
            message.chat().id,
            format!("Текущий сервер теперь '{}' -> {}", data, url),
        )
        .await?;
    }

    Ok(())
}

async fn callback_olap(
    bot: Bot,
    data: String,
    message: MaybeInaccessibleMessage,
    olap_store: SharedOlap,
) -> ResponseResult<()> {
    let olap = olap_store.lock().await;

    if let Some(olap_elements) = olap.get(&data) {
        let text = Server::display_olap(&olap_elements);

        bot.send_message(message.chat().id, text)
            .parse_mode(ParseMode::MarkdownV2)
            .await?;
    }

    Ok(())
}

async fn callback_delete_user(
    bot: Bot,
    data: String,
    message: MaybeInaccessibleMessage,
    allowed: Arc<Mutex<Vec<String>>>,
) -> ResponseResult<()> {
    let mut accounts = allowed.lock().await;

    if accounts.contains(&data) {
        accounts.retain(|account| account != &data);

        let mut telegram_config: TgCfg = read_to_struct("/etc/iiko-bot/tg_cfg.toml").await.unwrap();

        telegram_config.accounts.retain(|account| account != &data);

        let mut file = fs::File::create("/etc/iiko-bot/tg_cfg.toml").await.unwrap();

        let config = toml::to_string(&telegram_config).unwrap();

        file.write_all(config.as_bytes()).await.unwrap();

        let text = format!("Пользователь @{} успешно удалён", data);

        bot.send_message(message.chat().id, text).await?;
    }

    Ok(())
}
