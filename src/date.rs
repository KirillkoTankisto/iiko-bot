use chrono::{Duration, FixedOffset, Utc};

pub fn moscow_time() -> (String, i64) {
    let offset = FixedOffset::east_opt(3 * 3600).unwrap();

    let time_utc = Utc::now();
    let time_moscow = time_utc.with_timezone(&offset);

    (
        time_moscow.format("%Y-%m-%d").to_string(),
        time_moscow
            .format("%d")
            .to_string()
            .parse::<i64>()
            .unwrap_or(0),
    )
}

pub fn moscow_last_(days: i64) -> String {
    let offset = FixedOffset::east_opt(3 * 3600).unwrap();

    let time_utc = Utc::now();
    let time_moscow = time_utc.with_timezone(&offset);

    let past = time_moscow
        .checked_sub_signed(Duration::days(days))
        .unwrap();
    past.format("%Y-%m-%d").to_string()
}
