use std::{
    collections::HashMap,
    error::Error,
    time::{Duration, Instant},
};

use reqwest::Client;
use serde_json::from_str;

use crate::{
    Shift, Shifts,
    date::{moscow_last_, moscow_time},
    make_url,
    olap::{OLAPList, OlapElement, OlapMap, wrap_text},
    sha1sum,
};

pub struct Server {
    login: String,
    pass: String,
    url: String,
    token: Option<NewToken>,
}

#[allow(dead_code)]
pub enum Dates {
    Week,
    ThisMonth,
    Custom,
}

impl Server {
    pub fn new<S: Into<String>>(login: S, pass: S, url: S) -> Self {
        Self {
            login: login.into(),
            pass: pass.into(),
            url: url.into(),
            token: None,
        }
    }

    async fn auth(&mut self) -> Result<(), Box<dyn Error>> {
        if !self.is_authenticated() {
            let url = make_url::default(&self.url, &["auth"]);

            let response = Client::new()
                .get(&url)
                .query(&[("login", &self.login), ("pass", &sha1sum(&self.pass))])
                .timeout(Duration::from_secs(2))
                .send()
                .await?;

            let token = response.text().await?;

            let token = NewToken {
                id: token,
                creation_time: Instant::now(),
                lifetime: Duration::from_secs(3600),
            };

            self.token = Some(token);

            Ok(())
        } else {
            Ok(())
        }
    }

    pub async fn deauth(&mut self) -> Result<(), Box<dyn Error>> {
        if self.is_authenticated() {
            let url = make_url::default(&self.url, &["logout"]);

            Client::new()
                .get(url)
                .query(&[("key", self.token.clone().unwrap().id.clone())])
                .timeout(Duration::from_secs(2))
                .send()
                .await?
                .text()
                .await?;
        }

        self.token = None;
        Ok(())
    }

    fn is_authenticated(&self) -> bool {
        let token = self.token.as_ref();

        if token.is_none() {
            return false;
        }

        if token.unwrap().is_expired() {
            return false;
        }

        true
    }

    pub async fn get_token(&mut self) -> Result<String, Box<dyn Error>> {
        self.auth().await?;

        Ok(self.token.clone().unwrap().id)
    }
}

impl GetShifts for Server {
    async fn list_shifts_with_offset<Num: Into<i64>>(
        server: &mut Self,
        date: Dates,
        offset: Num,
    ) -> Result<Shifts, Box<dyn Error>> {
        server.auth().await?;

        let url = make_url::default(&server.url, &["v2", "cashshifts", "list"]);

        let date_from = match date {
            Dates::Week => moscow_last_(6),
            Dates::ThisMonth => moscow_last_(moscow_time().1 - 1),
            Dates::Custom => moscow_last_(offset.into()),
        };

        let response = Client::new()
            .get(url)
            .query(&[
                ("openDateFrom", date_from),
                ("openDateTo", moscow_time().0),
                ("status", "ANY".to_string()),
                ("key", server.token.clone().unwrap().id),
            ])
            .timeout(Duration::from_secs(2))
            .send()
            .await?
            .text()
            .await?;

        let parsed: Shifts = serde_json::from_str(&response)?;

        Ok(parsed)
    }

    fn latest_shift<Num>(shifts: Shifts, offset: Num) -> Result<Shift, Box<dyn Error>>
    where
        Num: Into<usize>,
    {
        let offset = offset.into();
        let len = shifts.len();

        if offset >= len {
            return Err(format!("Нет смены со сдвигом {}", offset).into());
        }

        let idx = len - offset - 1;

        shifts
            .into_iter()
            .nth(idx)
            .ok_or_else(|| format!("Нет смены со сдвигом {}", offset).into())
    }

    fn sum_shifts(shifts: Shifts) -> f64 {
        shifts.iter().map(|shift| shift.pay_orders).sum()
    }
}

pub trait GetShifts {
    async fn list_shifts_with_offset<Num: Into<i64>>(
        server: &mut Server,
        date: Dates,
        offset: Num,
    ) -> Result<Shifts, Box<dyn Error>>;

    fn latest_shift<Num: Into<usize>>(shifts: Shifts, offset: Num)
    -> Result<Shift, Box<dyn Error>>;

    fn sum_shifts(shifts: Shifts) -> f64;
}

#[derive(Clone)]
struct NewToken {
    id: String,
    creation_time: Instant,
    lifetime: Duration,
}

impl NewToken {
    fn is_expired(&self) -> bool {
        self.creation_time.elapsed() >= self.lifetime
    }
}

impl Olap for Server {
    async fn get_olap(
        form: String,
        server_url: String,
        key: String,
    ) -> Result<OlapMap, Box<dyn Error>> {
        let url = make_url::default(&server_url, &["v2", "reports", "olap"]);

        let response = Client::new()
            .post(url)
            .header("Content-Type", "application/json")
            .query(&[("key", &key)])
            .body(form)
            .send()
            .await?
            .text()
            .await?;

        let parsed: OLAPList = from_str(&response)?;

        let mut olap_map: OlapMap = HashMap::new();

        for element in parsed.data {
            let key = element.DishCategory.unwrap_or_else(|| "Другие".into());
            let olap = OlapElement {
                DishDiscountSumInt: element.DishDiscountSumInt,
                DishName: element.DishName,
                GuestNum: element.GuestNum,
            };
            olap_map
                .entry(key)
                .and_modify(|v| v.push(olap.clone()))
                .or_insert_with(|| vec![olap]);
        }

        Ok(olap_map)
    }
    fn display_olap(elements: &[OlapElement]) -> String {
        let headers = ["Название", "Сумма", "Заказы"];

        let mut sorted: Vec<&OlapElement> = elements.iter().collect();
        sorted.sort_by(|a, b| b.GuestNum.cmp(&a.GuestNum));
        let displayed = sorted.into_iter().take(20).collect::<Vec<_>>();

        let mut widths = headers
            .iter()
            .map(|h| h.chars().count())
            .collect::<Vec<usize>>();

        for element in &displayed {
            widths[0] = widths[0].max(element.DishName.chars().count().min(15));
            widths[1] = widths[1].max(element.DishDiscountSumInt.to_string().len());
            widths[2] = widths[2].max(element.GuestNum.to_string().len());
        }

        let draw_border = |left: char, middle: char, separator: char, right: char| {
            let mut string = String::new();
            string.push(left);
            for (i, &w) in widths.iter().enumerate() {
                string.push_str(&middle.to_string().repeat(w + 2));
                string.push(if i + 1 == widths.len() {
                    right
                } else {
                    separator
                });
            }
            string.push('\n');
            string
        };

        let mut table = String::new();

        table.push_str("```\n");
        table.push_str(&draw_border('┌', '─', '┬', '┐'));
        table.push('│');

        for (i, &h) in headers.iter().enumerate() {
            let total = widths[i] + 2;
            let pad_left = (total - h.chars().count()) / 2;
            let pad_right = total - h.chars().count() - pad_left;
            table.push_str(&" ".repeat(pad_left));
            table.push_str(h);
            table.push_str(&" ".repeat(pad_right));
            table.push('│');
        }

        table.push('\n');
        table.push_str(&draw_border('├', '─', '┼', '┤'));

        for (idx, element) in displayed.iter().enumerate() {
            let name_lines = wrap_text(&element.DishName, widths[0]);
            for (line_idx, line) in name_lines.into_iter().enumerate() {
                table.push('│');

                let pad_right = widths[0] + 2 - 1 - line.chars().count();
                table.push(' ');
                table.push_str(&line);
                table.push_str(&" ".repeat(pad_right));
                table.push('│');

                let fields = if line_idx == 0 {
                    vec![
                        element.DishDiscountSumInt.to_string(),
                        element.GuestNum.to_string(),
                    ]
                } else {
                    vec![String::new(), String::new()]
                };
                for (size, cell) in fields.iter().enumerate() {
                    let total = widths[size + 1] + 2;
                    let pad_right = total - 1 - cell.chars().count();

                    table.push(' ');
                    table.push_str(cell);
                    table.push_str(&" ".repeat(pad_right));
                    table.push('│');
                }

                table.push('\n');
            }
            if idx + 1 != displayed.len() {
                table.push_str(&draw_border('├', '─', '┼', '┤'));
            }
        }

        table.push_str(&draw_border('└', '─', '┴', '┘'));
        table.push_str("```\n");

        table
    }
}

pub trait Olap {
    async fn get_olap(form: String, url: String, key: String) -> Result<OlapMap, Box<dyn Error>>;

    fn display_olap(elements: &[OlapElement]) -> String;
}
