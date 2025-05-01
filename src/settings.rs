use std::collections::{HashSet};
use std::env;
use std::net::IpAddr;
use config::{Config, ConfigError, File};
use serde::Deserialize;

#[derive(Deserialize, Debug, Clone)]
pub struct Settings {
    #[serde(rename = "rate_limiter")]
    pub rate_limiter_settings: RateLimiterSettings,

    #[serde(rename = "api_gateway")]
    pub api_gateway_settings: ApiGatewaySettings,
}

#[derive(Deserialize, Debug, Clone)]
pub struct ApiGatewaySettings {
    pub target_url: String,
    pub proxy_server_addr: String,
}

#[derive(Deserialize, Debug, Clone)]
pub struct RateLimiterSettings {
    pub redis_addr: String,
    pub ip_whitelist: HashSet<IpAddr>,

    #[serde(rename = "limiter")]
    pub limiters_settings: Vec<LimiterSettings>,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum PossibleStrategies {
    IP,
    URL,
    Header,
    Query,
    Body,
}

#[derive(Deserialize, Debug, Clone)]
pub struct LimiterSettings {
    pub strategy: PossibleStrategies,
    pub global_bucket: Option<BucketSettings>,
    pub buckets_per_value: Option<Vec<BuckerPerValue>>,
}

#[derive(Deserialize, Debug, Clone)]
pub struct BuckerPerValue {
    pub value: String,
    pub tokens_count: u32,
    pub add_tokens_every: u32,
}

#[derive(Deserialize, Debug, Clone)]
pub struct BucketSettings {
    pub tokens_count: u32,
    pub add_tokens_every: u32,
}

impl Settings {
    pub fn new() -> Result<Settings, ConfigError> {
        let config_path = env::var("RL_SETTINGS_PATH").unwrap_or_else(|_| "./Settings.toml".to_string());

        let settings = Config::builder()
            .add_source(File::with_name(&config_path))
            .build()?;

        settings.try_deserialize()
    }
}