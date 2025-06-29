mod date;
mod iiko;
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
