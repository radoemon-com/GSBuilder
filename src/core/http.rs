use crate::core::config::Config;
use crate::core::error::AppResult;
use reqwest::blocking::Client;
use std::time::Duration;

pub fn http_client(config: &Config) -> AppResult<Client> {
    Ok(Client::builder()
        .user_agent(&config.network.user_agent)
        .timeout(Duration::from_secs(config.network.timeout_seconds))
        .build()?)
}
