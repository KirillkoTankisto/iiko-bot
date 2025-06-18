use serde::de::DeserializeOwned;
use std::error::Error;
use tokio::fs;
use toml::from_str;

pub async fn read_to_struct<T: DeserializeOwned, S: AsRef<str>>(
    path: S,
) -> Result<T, Box<dyn Error>> {
    let file = fs::read_to_string(path.as_ref()).await?;

    Ok(from_str(&file)?)
}
