use serde::de::DeserializeOwned;
use sha1::{Digest, Sha1};
use std::error::Error;
use tokio::fs;
use toml::from_str;

pub async fn read_to_struct<T: DeserializeOwned, S: AsRef<str>>(
    path: S,
) -> Result<T, Box<dyn Error>> {
    let file = fs::read_to_string(path.as_ref()).await?;

    Ok(from_str(&file)?)
}

pub fn sha1sum<S: AsRef<str>>(pass: S) -> String {
    format!("{:x}", Sha1::digest(pass.as_ref().as_bytes()))
}

use std::usize;

const HTTPS: &'static str = "https://";
const MIDDLE: &'static str = "/resto/api";

pub fn make_url(server: &String, path: &[&str]) -> String {
    let mut string = String::with_capacity(
        HTTPS.len()
            + server.len()
            + MIDDLE.len()
            + path.len()
            + path.iter().map(|element| element.len()).sum::<usize>(),
    );

    string.push_str(HTTPS);

    string.push_str(server);

    string.push_str(MIDDLE);

    for element in path {
        string.push('/');
        string.push_str(&element);
    }

    println!("{string}");

    string
}
