mod date;
mod make_url;
mod tg;
mod shared;

use date::{moscow_last_, moscow_time};
use reqwest::{Client, StatusCode};
use serde::Deserialize;
use sha1::{Digest, Sha1};
use std::{
    collections::HashMap,
    error::Error,
    fmt::Display,
    time::{self, Duration, Instant},
};
use tg::*;

#[tokio::main]
async fn main() -> Result<(), Box<dyn Error>> {
    initialise().await?;
    Ok(())
}

struct Token {
    token: String,
    time: Instant,
    lifetime: Duration,
}

impl Token {
    fn is_expired(&self) -> bool {
        self.time.elapsed() >= self.lifetime
    }
}

async fn auth(login: String, pass: String, server: &String) -> Result<Token, Box<dyn Error>> {
    let url = make_url::default(server, &["auth"]);

    let response = Client::new()
        .get(&url)
        .query(&[("login", login), ("pass", sha1sum(pass))])
        .send()
        .await?;

    let status = response.status();

    let token = match status {
        StatusCode::OK => response.text().await?,
        StatusCode::FORBIDDEN => return Err(format!("Доступ к пути {url} запрещён").into()),
        StatusCode::BAD_REQUEST => {
            return Err(format!("Неверный запрос: {}\n{:#?}", url, response.text().await?).into());
        }
        other => return Err(format!("{}", other).into()),
    };

    let token = Token {
        token,
        time: time::Instant::now(),
        lifetime: Duration::from_secs(3600),
    };

    Ok(token)
}

async fn logout(token: &Token, server: &String) -> Result<(), Box<dyn Error>> {
    if token.is_expired() {
        return Err(format!("Токен {} уже истёк", token.token).into());
    }

    let url = make_url::default(server, &["logout"]);

    Client::new()
        .get(url)
        .query(&[("key", token.token.clone())])
        .send()
        .await?
        .text()
        .await?;

    Ok(())
}

#[derive(Deserialize)]
struct Cfg {
    login: String,
    pass: String,
    servers: HashMap<String, String>,
}

struct ServerState {
    map: HashMap<String, String>,
    current: String,
}

fn sha1sum(pass: String) -> String {
    format!("{:x}", Sha1::digest(pass.as_bytes()))
}

#[allow(dead_code)]
#[derive(Deserialize, Clone)]
#[serde(rename_all = "camelCase")]
struct Shift {
    id: String,
    session_number: usize,
    fiscal_number: usize,
    cash_reg_number: usize,
    cash_reg_serial: String,
    open_date: String,
    close_date: Option<String>,
    accept_date: Option<String>,
    manager_id: String,
    responsible_user_id: Option<String>,
    session_start_cash: usize,
    pay_orders: usize,
    sum_writeoff_orders: usize,
    sales_cash: usize,
    sales_credit: usize,
    sales_card: usize,
    pay_in: usize,
    pay_out: usize,
    pay_income: i32,
    cash_remain: Option<usize>,
    cash_diff: i32,
    session_status: SessionStatus,
    conception_id: Option<String>,
}

#[derive(Deserialize, Clone)]
#[serde(rename_all = "UPPERCASE")]
enum SessionStatus {
    OPEN,
    CLOSED,
    ACCEPTED,
    UNACCEPTED,
    HASWARNINGS,
}

impl Display for SessionStatus {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::OPEN => write!(f, "Открыта"),
            _ => write!(f, "Закрыта"),
        }
    }
}

type Shifts = Vec<Shift>;

async fn list_shifts_week(token: &Token, server: &String) -> Result<Shifts, Box<dyn Error>> {
    let url = make_url::default(server, &["v2", "cashshifts", "list"]);

    let response = Client::new()
        .get(url)
        .query(&[
            ("openDateFrom", moscow_last_(6)),
            ("openDateTo", moscow_time().0),
            ("status", "ANY".to_string()),
            ("key", token.token.clone()),
        ])
        .send()
        .await?
        .text()
        .await?;

    let parsed: Shifts = serde_json::from_str(&response)?;

    Ok(parsed)
}

async fn list_shifts_month(token: &Token, server: &String) -> Result<Shifts, Box<dyn Error>> {
    let url = make_url::default(server, &["v2", "cashshifts", "list"]);

    let response = Client::new()
        .get(url)
        .query(&[
            ("openDateFrom", moscow_last_(moscow_time().1 - 1)),
            ("openDateTo", moscow_time().0),
            ("status", "ANY".to_string()),
            ("key", token.token.clone()),
        ])
        .send()
        .await?
        .text()
        .await?;

    let parsed: Shifts = serde_json::from_str(&response)?;

    Ok(parsed)
}

fn current_shift(shifts: &Shifts) -> Result<Shift, Box<dyn Error>> {
    shifts
        .last()
        .cloned()
        .ok_or("Невозможно получить текущую смену".into())
}

fn previous_shift(shifts: &Shifts, offset: usize) -> Result<Shift, Box<dyn Error>> {
    shifts
        .into_iter()
        .cloned()
        .nth(shifts.len() - offset - 1)
        .ok_or_else(|| format!("Нет смены со сдвигом {}", offset).into())
}

fn sum_shifts(shifts: Shifts) -> usize {
    shifts.iter().map(|shift| shift.pay_orders).sum()
}
