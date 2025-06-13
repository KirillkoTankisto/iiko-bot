use toml::from_str;
use serde::de::DeserializeOwned;
use tokio::fs;
use std::error::Error;

pub async fn read_to_struct<T: DeserializeOwned, S: AsRef<str>>(path: S) -> Result<T, Box<dyn Error>> {
    let file = fs::read_to_string(path.as_ref()).await?;

    Ok(from_str(&file)?)
}
