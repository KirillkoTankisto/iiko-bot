mod date;
mod iiko;
mod make_url;
mod olap;
mod shared;
mod tg;

use serde::Deserialize;
use sha1::{Digest, Sha1};
use std::{collections::HashMap, error::Error, fmt::Display};

use crate::tg::initialise;

#[tokio::main]
async fn main() -> Result<(), Box<dyn Error>> {
    initialise().await?;
    Ok(())
}

#[derive(Deserialize, Clone)]
struct Cfg {
    login: String,
    pass: String,
    servers: HashMap<String, String>,
}

struct ServerState {
    map: HashMap<String, String>,
    current: String,
}

fn sha1sum<S: AsRef<str>>(pass: S) -> String {
    format!("{:x}", Sha1::digest(pass.as_ref().as_bytes()))
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
    pay_orders: f64,
    sum_writeoff_orders: usize,
    sales_cash: usize,
    sales_credit: usize,
    sales_card: f64,
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
