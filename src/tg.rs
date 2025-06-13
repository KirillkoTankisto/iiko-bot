use crate::previous_shift;
use crate::shared::read_to_struct;
use crate::{
    Cfg, ServerState, auth, current_shift, list_shifts_month, list_shifts_week, logout, sum_shifts,
};
use serde::Deserialize;
use std::{error::Error, sync::Arc};
use teloxide::types::{InputPollOption, ParseMode};
use teloxide::utils::markdown::escape;
use teloxide::{prelude::*, utils::command::BotCommands};
use tokio::sync::Mutex;

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
    #[command(description = "Переключиться на сервер")]
    Switch { alias: String },
    #[command(description = "Вывести список доступных серверов")]
    List,
}

pub async fn initialise() -> Result<(), Box<dyn Error>> {
    let telegram_config: TgCfg = read_to_struct("/etc/iiko-bot/tg_cfg.toml").await?;
    let (token, accounts) = (telegram_config.token, telegram_config.accounts);
    let allowed = Arc::new(accounts);

    let main_config: Cfg = read_to_struct("/etc/iiko-bot/cfg.toml").await?;
    let servers = main_config.servers;
    let first = servers.keys().next().expect("Список серверов пуст").clone();

    let state = ServerState {
        map: servers,
        current: first,
    };

    let servers = Arc::new(Mutex::new(state));

    let bot = Bot::new(token);

    Command::repl(bot, move |bot, msg, cmd| {
        let allowed = allowed.clone();
        let servers = servers.clone();
        async move { answer(bot, msg, cmd, allowed, servers).await }
    })
    .await;

    Ok(())
}

async fn answer(
    bot: Bot,
    message: Message,
    command: Command,
    allowed_users: Arc<Vec<String>>,
    servers: Arc<Mutex<ServerState>>,
) -> ResponseResult<()> {
    // Получаем username отправителя
    let username = message
        .from
        .and_then(|u| u.username.clone())
        .unwrap_or_default();

    // Проверка доступа ко всем командам, кроме /help
    if let Command::Help = command {
    } else if !allowed_users.contains(&username) {
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

            let options = vec![InputPollOption::from("Артём")];

            bot.send_poll(message.chat.id, "Какого цвета оранжевый?", options).await?;
        }

        Command::Today => {
            let main_config: Cfg = read_to_struct("/etc/iiko-bot/cfg.toml").await.unwrap();
            let (login, pass) = (main_config.login, main_config.pass);

            let servers = servers.lock().await;
            let server = servers.map.get(&servers.current).unwrap();
            let token = auth(login, pass, server).await.unwrap();
            let shifts = list_shifts_week(&token, server).await.unwrap();
            logout(&token, server).await.unwrap();
            let shift = current_shift(&shifts).unwrap();
            let text = format!(
                "*Сервер*: *{}*\n\
                 *Текущая смена*:\n\
                 Номер смены: *{}*\n\
                 Статус: *{}*\n\
                 Оплачено картой: *{}*\n\
                 Оплачено наличкой: *{}*\n\
                 Итог: *{}*",
                servers.current,
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
            let main_config: Cfg = read_to_struct("/etc/iiko-bot/cfg.toml").await.unwrap();
            let (login, pass) = (main_config.login, main_config.pass);

            let servers = servers.lock().await;
            let server = servers.map.get(&servers.current).unwrap();
            let token = auth(login, pass, server).await.unwrap();
            let shifts = list_shifts_week(&token, &server).await.unwrap();
            logout(&token, &server).await.unwrap();
            let shift = previous_shift(&shifts, 1).unwrap();
            let text = format!(
                "*Сервер*: *{}*\n\
                *Предыдущая смена*:\n\
                 Номер смены: *{}*\n\
                 Статус: *{}*\n\
                 Оплачено картой: *{}*\n\
                 Оплачено наличкой: *{}*\n\
                 Итог: *{}*",
                servers.current,
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
            let main_config: Cfg = read_to_struct("/etc/iiko-bot/cfg.toml").await.unwrap();
            let (login, pass) = (main_config.login, main_config.pass);

            let servers = servers.lock().await;
            let server = servers.map.get(&servers.current).unwrap();

            let token = auth(login, pass, server).await.unwrap();
            let shifts = list_shifts_week(&token, &server).await.unwrap();

            logout(&token, &server).await.unwrap();

            let sum = sum_shifts(shifts);
            let text = format!(
                "*Сервер*: *{}*\n*Сумма за прошедшие 7 дней*: *{}*",
                servers.current,
                escape(&format_with_dots(sum))
            );
            bot.send_message(message.chat.id, text)
                .parse_mode(ParseMode::MarkdownV2)
                .await?;
        }

        Command::Month => {
            let main_config: Cfg = read_to_struct("/etc/iiko-bot/cfg.toml").await.unwrap();
            let (login, pass) = (main_config.login, main_config.pass);

            let servers = servers.lock().await;
            let server = servers.map.get(&servers.current).unwrap();

            let token = auth(login, pass, server).await.unwrap();
            let shifts = list_shifts_month(&token, &server).await.unwrap();

            logout(&token, &server).await.unwrap();

            let sum = sum_shifts(shifts);
            let text = format!(
                "*Сервер*: *{}*\n*Сумма за текущий месяц*: *{}*",
                servers.current,
                escape(&format_with_dots(sum))
            );
            bot.send_message(message.chat.id, text)
                .parse_mode(ParseMode::MarkdownV2)
                .await?;
        }

        Command::Switch { alias } => {
            let mut server = servers.lock().await;

            if let Some(url) = server.map.get(&alias).cloned() {
                server.current = alias.clone();
                bot.send_message(
                    message.chat.id,
                    format!("Текущий сервер теперь '{}' -> {}", alias, url),
                )
                .await?;
            } else {
                let text = server
                    .map
                    .iter()
                    .map(|(alias, url)| format!("{} -> {}", alias, url))
                    .collect::<Vec<_>>()
                    .join("\n");

                let error = format!("Неизвестный псевдоним *{}*\n*Доступные*:\n", alias);

                bot.send_message(message.chat.id, format!("{}{}", error, escape(&text)))
                    .parse_mode(ParseMode::MarkdownV2)
                    .await?;
            }
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
    }

    Ok(())
}
