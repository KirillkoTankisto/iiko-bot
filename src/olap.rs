use std::collections::HashMap;

use serde::{Deserialize, Serialize};

#[derive(Deserialize, Debug)]
#[allow(non_snake_case)]
pub struct OLAP {
    pub DishCategory: Option<String>,
    pub DishDiscountSumInt: f64,
    pub DishName: String,
    pub GuestNum: u32,
}

#[derive(Deserialize, Debug)]
pub struct OLAPList {
    pub data: Vec<OLAP>,
}

#[derive(Clone, Debug)]
#[allow(non_snake_case)]
pub struct OlapElement {
    pub DishDiscountSumInt: f64,
    pub DishName: String,
    pub GuestNum: u32,
}

pub type OlapMap = HashMap<String, Vec<OlapElement>>;

pub fn wrap_text(text: &str, width: usize) -> Vec<String> {
    let mut lines = Vec::new();
    let mut current = String::new();
    for word in text.split_whitespace() {
        if !current.is_empty() && current.chars().count() + 1 + word.chars().count() > width {
            lines.push(current.clone());
            current.clear();
        }
        if !current.is_empty() {
            current.push(' ');
        }
        current.push_str(word);
    }
    if !current.is_empty() {
        lines.push(current);
    }
    lines
}

#[derive(Serialize, Deserialize, Debug)]
#[serde(rename_all = "UPPERCASE")]
pub enum ReportType {
    Sales,
}

#[derive(Serialize, Deserialize, Debug)]
#[serde(rename_all = "PascalCase")]
pub enum FilterType {
    DateRange,
    IncludeValues,
}

#[derive(Serialize, Deserialize, Debug)]
#[allow(non_camel_case_types)]
pub enum PeriodType {
    CURRENT_MONTH,
}

#[derive(Serialize, Deserialize, Debug)]
#[serde(tag = "filterType")]
#[allow(non_snake_case)]
pub enum Filter {
    DateRange {
        periodType: PeriodType,
        to: String,
    },
    IncludeValues {
        values: Vec<String>,
    },
}

#[derive(Serialize, Deserialize, Debug)]
pub struct ReportConfig {
    #[serde(rename = "reportType")]
    pub report_type: ReportType,

    #[serde(rename = "groupByRowFields")]
    pub group_by_row_fields: Vec<String>,

    #[serde(rename = "groupByColFields")]
    pub group_by_col_fields: Vec<String>,

    #[serde(rename = "aggregateFields")]
    pub aggregate_fields: Vec<String>,

    pub filters: HashMap<String, Filter>,
}