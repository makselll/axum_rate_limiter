use std::collections::{HashMap, HashSet};
use std::net::{IpAddr, SocketAddr};
use std::sync::{Arc};
use axum::body::{to_bytes, Body, Bytes};
use axum::extract::{ConnectInfo, State};
use axum::http::{HeaderValue, Request, StatusCode};
use axum::http::request::Parts;
use axum::middleware::Next;
use axum::response::{IntoResponse, Response};
use axum_macros::debug_middleware;
use deadpool_redis::{Config, Pool};
use crate::settings::{BucketSettings, RateLimiterSettings};
use crate::strategy::{LimitForRequest, Strategy};

#[debug_middleware]
pub async fn middleware(
    State(rate_limiter_manager): State<Arc<RateLimiterManager>>,
    ConnectInfo(addr): ConnectInfo<SocketAddr>,
    request: Request<Body>,
    next: Next,
) -> Response<Body> {
    // Check whitelist
    if rate_limiter_manager.ip_whitelist.contains(&addr.ip()) {
        println!("IP {} is whitelisted", addr.ip());
        return next.run(request).await;
    }

    // Split the request into parts and body because Request<Body> is not Send
    let (parts, body) = request.into_parts();
    let body_bytes = match to_bytes(body, usize::MAX).await {
        Ok(bytes) => bytes,
        Err(_) => return (StatusCode::INTERNAL_SERVER_ERROR, "Internal server error").into_response(),
    };
    
    let safe_request = SafeRequest::new(parts, body_bytes);
    let mut lowest_limit: Option<LimitForRequest> = None;
    
    let rate_limiter_groups = vec!(
        &rate_limiter_manager.user_rate_limiters, // start to check the user
        &rate_limiter_manager.request_rate_limiters // check the request
    );
    
    for rate_limiters_group in rate_limiter_groups {
        for rate_limiter in rate_limiters_group.iter() {
            let limit = rate_limiter.check(&safe_request, addr).await;
            match limit {
                None => continue,
                Some(limit) => {
                    match &lowest_limit {
                        Some(current) if current > &limit => lowest_limit = Some(limit),
                        None => lowest_limit = Some(limit),
                        _ => {}
                    }
                }
            }
        }

        if let Some(limit) = &lowest_limit {
            if limit.is_limit_exceeded {
                println!("Rate limit exceeded for {}", addr.ip());
                return (StatusCode::TOO_MANY_REQUESTS, "Rate limit exceeded").into_response();
            }
        }
    }
    
    let mut response = next.run(Request::from_parts(safe_request.parts, Body::from(safe_request.body))).await;
    
    if let Some(limit) = &lowest_limit {
        let headers = response.headers_mut();   
        headers.insert("X-RateLimit-Limit", HeaderValue::from(limit.total_limit));
        headers.insert("X-RateLimit-Remaining", HeaderValue::from(limit.requests_to_exceed_limit));
    }
    
    response
}


#[derive(Clone, Debug)]
pub struct RateLimiterManager {
    ip_whitelist: HashSet<IpAddr>,
    user_rate_limiters: Vec<Arc<RateLimiter>>,
    request_rate_limiters: Vec<Arc<RateLimiter>>,
}

impl RateLimiterManager {
    pub fn new(rate_limiter_settings: RateLimiterSettings) -> Result<Self, std::io::Error> {
        let mut user_rate_limiters = Vec::new();
        let mut request_rate_limiters = Vec::new();


        let cfg = Config::from_url(format!("redis://{}", rate_limiter_settings.redis_addr.as_str()));
        let pool = cfg.create_pool(Some(deadpool_redis::Runtime::Tokio1)).map_err(|e| std::io::Error::new(std::io::ErrorKind::InvalidData, e))?;

        for settings in rate_limiter_settings.limiters_settings.iter() {
            let strategy = Strategy::from_possible_strategy(&settings.strategy);
            let global_bucket= settings.global_bucket.as_ref().map(Bucket::from);
            
            let buckets_per_value = settings.buckets_per_value.as_ref().map(
                |buckets| buckets.iter().map(
                    |b| (b.value.clone(), Bucket::new(b.tokens_count, b.add_tokens_every))
                ).collect());
            
            if buckets_per_value.is_none() && global_bucket.is_none() {
                return Err(std::io::Error::new(std::io::ErrorKind::InvalidData,"No bucket defined for rate limiter"))
            }
            
            match strategy {
                Strategy::IP(_) => user_rate_limiters.push(Arc::new(RateLimiter::new(strategy, pool.clone(), global_bucket, buckets_per_value))),
                Strategy::Header(_) => user_rate_limiters.push(Arc::new(RateLimiter::new(strategy, pool.clone(), global_bucket, buckets_per_value))),
                Strategy::Url(_) => request_rate_limiters.push(Arc::new(RateLimiter::new(strategy, pool.clone(), global_bucket, buckets_per_value))),
                Strategy::Query(_) => request_rate_limiters.push(Arc::new(RateLimiter::new(strategy, pool.clone(), global_bucket, buckets_per_value))),
                Strategy::Body(_) => request_rate_limiters.push(Arc::new(RateLimiter::new(strategy, pool.clone(), global_bucket, buckets_per_value))),
            }
        }
        
        Ok(Self {
            user_rate_limiters,
            request_rate_limiters,
            ip_whitelist: rate_limiter_settings.ip_whitelist.clone(),
        })
    }
}


#[derive(Clone, Debug)]
pub struct Bucket {
    pub tokens_count: u32,
    pub add_tokens_every: u32,
}

impl Bucket {
    pub fn new(tokens_count: u32, add_tokens_every: u32) -> Self {
        Self {
            tokens_count,
            add_tokens_every,
        }
    }
}

impl From<&BucketSettings> for Bucket {
    fn from(settings: &BucketSettings) -> Self {
        Self {
            tokens_count: settings.tokens_count,
            add_tokens_every: settings.add_tokens_every,
        }
    }   
}
#[derive(Clone, Debug)]
struct RateLimiter {
    strategy: Strategy,
    redis_pool: Pool,
    global_bucket: Option<Bucket>,
    buckets_per_value: Option<HashMap<String, Bucket>>,
}


impl RateLimiter {
    pub fn new(strategy: Strategy, redis_pool: Pool, global_bucket: Option<Bucket>, buckets_per_value: Option<HashMap<String, Bucket>>) -> Self {
        Self {
            strategy,
            redis_pool,
            global_bucket,
            buckets_per_value,
        }
    }
    
    pub async fn check(&self, request: &SafeRequest, addr: SocketAddr) -> Option<LimitForRequest> {
        let redis_conn = match self.redis_pool.get().await {
            Ok(redis_conn) => redis_conn,
            Err(_) => return None,
        };
        self.strategy.check_limit(redis_conn, self.global_bucket.as_ref(), self.buckets_per_value.as_ref(), request, addr).await

    }
}


pub struct SafeRequest {
    pub parts: Parts,
    pub body: Bytes,
}

impl SafeRequest {
    pub fn new(parts: Parts, body: Bytes) -> Self {
        Self {
            parts,
            body,
        }
    }
}