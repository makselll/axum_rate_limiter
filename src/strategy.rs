use std::collections::HashMap;
use std::hash::{DefaultHasher, Hash, Hasher};
use std::net::SocketAddr;
use axum::async_trait;
use deadpool_redis::{redis, Connection};
use serde_json::Value;
use url::{form_urlencoded};
use crate::limiter::{Bucket, SafeRequest};
use crate::settings::PossibleStrategies;


#[derive(Clone, Debug)]
pub struct LimitForRequest {
    pub total_limit: u32,
    pub requests_to_exceed_limit: i32,
    pub is_limit_exceeded: bool,
}

impl LimitForRequest {
    pub fn new(total_limit: u32, requests_to_exceed_limit: i32, is_limit_exceeded: bool) -> Self {
        Self {
            total_limit,
            requests_to_exceed_limit,
            is_limit_exceeded,
        }
    }
}

impl PartialEq for LimitForRequest {
    fn eq(&self, other: &Self) -> bool {
        self.requests_to_exceed_limit == other.requests_to_exceed_limit
    }
}

impl Eq for LimitForRequest {}

impl Ord for LimitForRequest {
    fn cmp(&self, other: &Self) -> std::cmp::Ordering {
        self.requests_to_exceed_limit.cmp(&other.requests_to_exceed_limit)
    }
}

impl PartialOrd for LimitForRequest {
    fn partial_cmp(&self, other: &Self) -> Option<std::cmp::Ordering> {
        Some(self.requests_to_exceed_limit.cmp(&other.requests_to_exceed_limit))
    }
}


#[derive(Debug)]
pub struct LimitRedisKey {
    key: String,
    bucket: Bucket,
}

impl LimitRedisKey {
    fn new(key: String, bucket: Bucket) -> Self {
        Self {
            key,
            bucket
        }
    }
}

#[async_trait]
pub trait RateLimiterChecker {
    async fn check_limit(&self, mut redis_connection: Connection, global_bucket: Option<&Bucket>, buckets_per_value: Option<&HashMap<String, Bucket>>, request: &SafeRequest, addr: SocketAddr) -> Option<LimitForRequest> {
        let limit_redis_key = match self.get_redis_key(request, addr, global_bucket, buckets_per_value) {
            Some(key) => key,
            None => return None, // skip this check because we can't define what value we should check
        };
        
        redis::cmd("SET")
            .arg(&limit_redis_key.key)
            .arg(limit_redis_key.bucket.tokens_count)
            .arg("EX")
            .arg(limit_redis_key.bucket.add_tokens_every)
            .arg("NX")
            .query_async::<()>(&mut redis_connection)
            .await
            .unwrap_or(()); // Ignore error

        // Decrement key
        let count: i32 = redis::cmd("DECR")
            .arg(&limit_redis_key.key)
            .query_async(&mut redis_connection)
            .await
            .unwrap_or(-1); // Set to 0 if the key doesn't exist

        Some(LimitForRequest::new(limit_redis_key.bucket.tokens_count, count,count < 0))
    }

    fn hash_key(&self, s: String) -> u64 {
        let mut hasher = DefaultHasher::new();
        s.hash(&mut hasher);
        hasher.finish()
    }

    fn get_redis_key(&self, request: &SafeRequest, addr: SocketAddr, global_bucket: Option<&Bucket>, buckets_per_value: Option<&HashMap<String, Bucket>>) -> Option<LimitRedisKey>;
}


#[derive(Clone, Debug)]
pub struct IPRateLimiterStrategy;
#[derive(Clone, Debug)]
pub struct UrlRateLimiterStrategy;

#[derive(Clone, Debug)]
pub struct HeaderRateLimiterStrategy;

#[derive(Clone, Debug)]
pub struct RequestQueryRateLimiterStrategy;

#[derive(Clone, Debug)]
pub struct RequestBodyRateLimiterStrategy;


#[async_trait]
impl RateLimiterChecker for IPRateLimiterStrategy {
    fn get_redis_key(&self, _request: &SafeRequest, addr: SocketAddr, global_bucket: Option<&Bucket>, buckets_per_value: Option<&HashMap<String, Bucket>>) -> Option<LimitRedisKey> {
        let ip = addr.ip();

        let bucket = match buckets_per_value {
            Some(bucket) => bucket.get(&ip.to_string()).or(global_bucket),
            None => global_bucket
        };

        Some(LimitRedisKey::new(format!("rate_limiter:ip:{}", self.hash_key(ip.to_string())), bucket?.to_owned()))
    }
}


#[async_trait]
impl RateLimiterChecker for UrlRateLimiterStrategy {
    fn get_redis_key(&self, request: &SafeRequest, _addr: SocketAddr, global_bucket: Option<&Bucket>, buckets_per_value: Option<&HashMap<String, Bucket>>) -> Option<LimitRedisKey> {
        let uri = request.parts.uri.path();

        let bucket = match buckets_per_value {
            Some(bucket) => bucket.get(&uri.to_string()).or(global_bucket),
            None => global_bucket
        };

        Some(LimitRedisKey::new(format!("rate_limiter:url:{}", self.hash_key(uri.to_string())), bucket?.to_owned()))
    }
}


impl RateLimiterChecker for HeaderRateLimiterStrategy {
    fn get_redis_key(&self, request: &SafeRequest, _addr: SocketAddr, global_bucket: Option<&Bucket>, buckets_per_value: Option<&HashMap<String, Bucket>>) -> Option<LimitRedisKey> {
        let mut found_header: Option<String> = None;
        let mut found_bucket: Option<Bucket> = None;

        if let Some(buckets) = buckets_per_value {
            for (k, v) in buckets {
                match request.parts.headers.get(k.to_lowercase()) {
                    Some(value) => {
                        found_header = Some(format!("{}:{}", k, value.to_str().unwrap()));
                        found_bucket = Some(v.to_owned())
                    },
                    None => continue,
                };
            };
        };

        if found_header.is_none() && global_bucket.is_some() {
            if let Some(value) = request.parts.headers.get("authorization") {
                found_header = Some(value.to_str().unwrap().to_string());
                found_bucket = global_bucket.cloned();
            };
        }

        Some(LimitRedisKey::new(format!("rate_limiter:header:{}", self.hash_key(found_header?)), found_bucket?))
    }
}


impl RateLimiterChecker for RequestQueryRateLimiterStrategy {
    fn get_redis_key(&self, request: &SafeRequest, _addr: SocketAddr, _global_bucket: Option<&Bucket>, buckets_per_value: Option<&HashMap<String, Bucket>>) -> Option<LimitRedisKey> {
        let mut found_param: Option<String> = None;
        let mut found_bucket: Option<Bucket> = None;

        if let Some(query) = request.parts.uri.query() {
            for (k, v) in form_urlencoded::parse(query.as_bytes()) {
                if let Some(bucket) = buckets_per_value?.get(&k.to_string()) {
                    found_param = Some(format!("{}:{}", k, v));
                    found_bucket = Some(bucket.to_owned());
                    break;
                }
            }
        }

        Some(LimitRedisKey::new(format!("rate_limiter:query:{}", self.hash_key(found_param?)), found_bucket?))
    }
}


impl RateLimiterChecker for RequestBodyRateLimiterStrategy {
    fn get_redis_key(&self, request: &SafeRequest, _addr: SocketAddr, _global_bucket: Option<&Bucket>, buckets_per_value: Option<&HashMap<String, Bucket>>) -> Option<LimitRedisKey> {
        let mut found_param: Option<String> = None;
        let mut found_bucket: Option<Bucket> = None;

        let json_body: HashMap<String, Value> = serde_json::from_slice(&request.body).unwrap_or_default();
        for (k, v) in json_body {
            if let Some(bucket) = buckets_per_value?.get(&k.to_string()) {
                found_param = Some(format!("{}:{}", k, v));
                found_bucket = Some(bucket.to_owned());
                break;
            }
        }
        
        Some(LimitRedisKey::new(format!("rate_limiter:json:{}", self.hash_key(found_param?)), found_bucket?))
    }
}


#[derive(Clone, Debug)]
pub enum Strategy {
    IP(IPRateLimiterStrategy),
    Url(UrlRateLimiterStrategy),
    Header(HeaderRateLimiterStrategy),
    Query(RequestQueryRateLimiterStrategy),
    Body(RequestBodyRateLimiterStrategy),
}

impl Strategy {
    pub fn from_possible_strategy(strategy: &PossibleStrategies) -> Self {
        match strategy {
            PossibleStrategies::IP => Strategy::IP(IPRateLimiterStrategy),
            PossibleStrategies::URL => Strategy::Url(UrlRateLimiterStrategy),
            PossibleStrategies::Header => Strategy::Header(HeaderRateLimiterStrategy),
            PossibleStrategies::Query => Strategy::Query(RequestQueryRateLimiterStrategy),
            PossibleStrategies::Body => Strategy::Body(RequestBodyRateLimiterStrategy),
        }
    }

    pub async fn check_limit(
        &self,
        redis_connection: Connection,
        global_bucket: Option<&Bucket>,
        buckets_per_value: Option<&HashMap<String, Bucket>>,
        request: &SafeRequest,
        addr: SocketAddr,
    ) -> Option<LimitForRequest> {
        match self {
            Strategy::IP(strategy) => strategy.check_limit(redis_connection, global_bucket, buckets_per_value, request, addr).await,
            Strategy::Url(strategy) => strategy.check_limit(redis_connection, global_bucket, buckets_per_value,request, addr).await,
            Strategy::Header(strategy) => strategy.check_limit(redis_connection, global_bucket, buckets_per_value,request, addr).await,
            Strategy::Query(strategy) => strategy.check_limit(redis_connection, global_bucket, buckets_per_value,request, addr).await,
            Strategy::Body(strategy) => strategy.check_limit(redis_connection, global_bucket, buckets_per_value,request, addr).await,

        }
    }

}
