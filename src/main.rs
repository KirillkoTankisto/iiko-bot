mod date;
mod iiko;
mod make_url;
mod olap;
mod shared;
mod tg;

use crate::tg::initialise;
use std::error::Error;

#[tokio::main]
async fn main() -> Result<(), Box<dyn Error>> {
    initialise().await?;
    Ok(())
}
