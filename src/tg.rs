use crate::date::moscow_time;
use crate::new::{Dates, GetShifts, Olap, Server};
use crate::olap::{Filter, OlapMap, PeriodType, ReportConfig, ReportType};
use crate::{Cfg, ServerState, shared::read_to_struct};

use std::collections::HashMap;
use std::{error::Error, sync::Arc};

use serde::Deserialize;

use teloxide::dispatching::{HandlerExt, UpdateFilterExt};
use teloxide::payloads::SendMessageSetters;
use teloxide::prelude::{Dispatcher, Requester, ResponseResult};
use teloxide::types::Update;
use teloxide::{Bot, dptree};
use teloxide::{
    types::{CallbackQuery, InlineKeyboardButton, InlineKeyboardMarkup, Message, ParseMode},
    utils::command::BotCommands,
    utils::markdown::escape,
};

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

#[derive(Deserialize)]
struct TgCfg {
    token: String,
    accounts: Vec<String>,
}

#[derive(BotCommands, Clone)]
#[command(rename_rule = "lowercase", description = "Поддерживаемые команды:")]
enum Command {
    #[command(description = "Отобразить список команд")]
    Help,
    #[command(description = "Тестовая команда")]
    Test,
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
}

pub async fn initialise() -> Result<(), Box<dyn Error>> {
    let telegram_config: TgCfg = read_to_struct("/etc/iiko-bot/tg_cfg.toml").await?;
    let (token, accounts) = (telegram_config.token, telegram_config.accounts);
    let allowed = Arc::new(accounts);

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
            servers.clone(),
            olap_store.clone()
        ])
        .build()
        .dispatch()
        .await;

    Ok(())
}

async fn handle_command(
    bot: Bot,
    message: Message,
    command: Command,
    config: Cfg,
    allowed: Arc<Vec<String>>,
    servers: Arc<Mutex<ServerState>>,
    olap_store: SharedOlap,
) -> ResponseResult<()> {
    let username = message
        .from
        .and_then(|u| u.username.clone())
        .unwrap_or_default();

    if let Command::Help = command {
    } else if !allowed.contains(&username) {
        bot.send_message(message.chat.id, "У вас нет доступа к этой команде.")
            .await?;
        return Ok(());
    }

    match command {
        Command::Help => {
            bot.send_message(message.chat.id, Command::descriptions().to_string())
                .await?;
        }

        Command::Test => {
            bot.send_message(message.chat.id, "Доступ к тестовой команде разрешен.")
                .await?;

            let mut options: Vec<InlineKeyboardButton> = Vec::new();

            for map in &servers.lock().await.map {
                options.push(InlineKeyboardButton::callback(map.0, map.0));
            }

            let keyboard = InlineKeyboardMarkup::default().append_row(options);

            bot.send_message(message.chat.id, "Выеби своего бойца")
                .reply_markup(keyboard)
                .await?;
        }

        Command::Today => {
            let (login, pass, server_url, current_server) =
                collect_server_info(servers, config).await;

            let mut server = Server::new(login, pass, server_url.into());

            let shifts = Server::list_shifts_with_offset(&mut server, Dates::Week, 0)
                .await
                .unwrap();
            server.deauth().await.unwrap();

            let offset: usize = 0;
            let shift = Server::latest_shift(shifts, offset).unwrap();

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
                escape(&format_with_dots(shift.sales_card)),
                escape(&format_with_dots(shift.sales_cash)),
                escape(&format_with_dots(shift.pay_orders)),
            );

            bot.send_message(message.chat.id, text)
                .parse_mode(ParseMode::MarkdownV2)
                .await?;
        }

        Command::Yesterday => {
            let (login, pass, server_url, current_server) =
                collect_server_info(servers, config).await;

            let mut server = Server::new(login, pass, server_url.into());

            let shifts = Server::list_shifts_with_offset(&mut server, Dates::Week, 0)
                .await
                .unwrap();
            server.deauth().await.unwrap();

            let offset: usize = 1;
            let shift = Server::latest_shift(shifts, offset).unwrap();

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
                escape(&format_with_dots(shift.sales_card)),
                escape(&format_with_dots(shift.sales_cash)),
                escape(&format_with_dots(shift.pay_orders)),
            );

            bot.send_message(message.chat.id, text)
                .parse_mode(ParseMode::MarkdownV2)
                .await?;
        }

        Command::Week => {
            let (login, pass, server_url, current_server) =
                collect_server_info(servers, config).await;

            let mut server = Server::new(login, pass, server_url.into());

            let shifts = Server::list_shifts_with_offset(&mut server, Dates::Week, 0)
                .await
                .unwrap();
            server.deauth().await.unwrap();

            let sum = Server::sum_shifts(shifts);

            let text = format!(
                "*Сервер*: *{}*\n*Сумма за прошедшие 7 дней*: *{}*",
                current_server,
                escape(&format_with_dots(sum))
            );

            bot.send_message(message.chat.id, text)
                .parse_mode(ParseMode::MarkdownV2)
                .await?;
        }

        Command::Month => {
            let (login, pass, server_url, current_server) =
                collect_server_info(servers, config).await;

            let mut server = Server::new(login, pass, server_url.into());

            let shifts = Server::list_shifts_with_offset(&mut server, Dates::ThisMonth, 0)
                .await
                .unwrap();
            server.deauth().await.unwrap();

            let sum = Server::sum_shifts(shifts);

            let text = format!(
                "*Сервер*: *{}*\n*Сумма за текущий месяц*: *{}*",
                current_server,
                escape(&format_with_dots(sum))
            );

            bot.send_message(message.chat.id, text)
                .parse_mode(ParseMode::MarkdownV2)
                .await?;
        }

        Command::Switch => {
            let mut options: Vec<InlineKeyboardButton> = Vec::new();

            let server = servers.lock().await;

            for map in &server.map {
                options.push(InlineKeyboardButton::callback(map.0, map.0));
            }

            let keyboard = InlineKeyboardMarkup::default().append_row(options);

            let text = format!("Текущий сервер: *{}*", server.current);

            bot.send_message(message.chat.id, text)
                .parse_mode(ParseMode::MarkdownV2)
                .reply_markup(keyboard)
                .await?;
        }

        Command::List => {
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
        }

        Command::Olap => {
            let (login, pass, server_url, current_server) =
                collect_server_info(servers.clone(), config.clone()).await;
            let mut server = Server::new(login, pass, server_url.clone().into());

            let form = ReportConfig {
                report_type: ReportType::Sales,
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

            let rows: Vec<Vec<InlineKeyboardButton>> = olap
                .keys()
                .map(|key| vec![InlineKeyboardButton::callback(key.clone(), key.clone())])
                .collect();
            let keyboard = InlineKeyboardMarkup::new(rows);

            let text = escape(&format!(
                "Режим Olap отчёта. Текущий сервер: {}",
                current_server
            ));

            match bot
                .send_message(message.chat.id, text)
                .parse_mode(ParseMode::MarkdownV2)
                .reply_markup(keyboard)
                .await
            {
                Ok(_) => (),
                Err(e) => eprintln!("{:?}", e),
            };
        }
    }
    Ok(())
}

async fn handle_callback(
    bot: Bot,
    query: CallbackQuery,
    servers: Arc<Mutex<ServerState>>,
    olap_store: SharedOlap,
) -> ResponseResult<()> {
    if let (Some(data), Some(message)) = (query.data.clone(), query.message.clone()) {
        let mut server = servers.lock().await;

        if let Some(url) = server.map.get(&data).cloned() {
            server.current = data.clone();
            bot.send_message(
                message.chat().id,
                format!("Текущий сервер теперь '{}' -> {}", data, url),
            )
            .await?;
        }

        let olap = olap_store.lock().await;
        if let Some(olap_elements) = olap.get(&data) {
            let text = Server::display_olap(&olap_elements);

            bot.send_message(message.chat().id, text)
                .parse_mode(ParseMode::MarkdownV2)
                .await?;
        }
    }
    Ok(())
}
