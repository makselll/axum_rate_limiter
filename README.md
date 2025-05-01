# Rate Limiter

A flexible and powerful rate limiting solution that can be configured to limit requests based on various strategies including IP, URL, Headers, Query parameters, and Request body content.

## Features

- Multiple rate limiting strategies
- Redis-based token bucket implementation
- IP whitelisting
- Configurable token bucket parameters
- Support for global and per-value rate limits
- Easy configuration through TOML file
- Flexible configuration path setup via environment variables

## Configuration

The rate limiter is configured through a `Settings.toml` file. You can specify the path to your configuration file using the `RL_SETTINGS_PATH` environment variable. If not specified, the rate limiter will look for the Settings.toml file in the current directory.

```bash
# Example of setting custom configuration path
export RL_SETTINGS_PATH=/path/to/your/custom/Settings.toml
```

Here's a detailed breakdown of the configuration options:

### API Gateway Configuration

```toml
[api_gateway]
target_url = "python-server:5000"      # The target service URL to proxy requests to
proxy_server_addr = "0.0.0.0:3000"     # The address where the rate limiter proxy will listen
```

### Rate Limiter Base Configuration

```toml
[rate_limiter]
redis_addr = "redis:6379"              # Redis server address for token bucket storage
ip_whitelist = ["127.0.0.1", "198.0.0.1"]  # List of IPs that bypass rate limiting
```

### Rate Limiting Strategies

The rate limiter supports multiple strategies that can be configured simultaneously. Each strategy is defined under `[[rate_limiter.limiter]]` section.

#### Available Strategies

1. **URL-based Rate Limiting**
```toml
[[rate_limiter.limiter]]
strategy = "url"
global_bucket = { tokens_count = 10, add_tokens_every = 120 }  # Global limit for all URLs
buckets_per_value = [
    { value = "/hello", tokens_count = 1, add_tokens_every = 10 },  # Specific limit for /hello
    { value = "/", tokens_count = 5, add_tokens_every = 10 }        # Specific limit for /
]
```

2. **IP-based Rate Limiting**
```toml
[[rate_limiter.limiter]]
strategy = "ip"
global_bucket = { tokens_count = 10, add_tokens_every = 120 }  # Limit requests per IP
```

3. **Header-based Rate Limiting**
```toml
[[rate_limiter.limiter]]
strategy = "header"
global_bucket = { tokens_count = 3, add_tokens_every = 120 }
buckets_per_value = [
    { value = "X-Telegram-User-Token", tokens_count = 1, add_tokens_every = 100 },
]
```

4. **Query Parameter Rate Limiting**
```toml
[[rate_limiter.limiter]]
strategy = "query"
buckets_per_value = [
    { value = "user_id", tokens_count = 1, add_tokens_every = 30 },
]
```

5. **Request Body Rate Limiting**
```toml
[[rate_limiter.limiter]]
strategy = "body"
buckets_per_value = [
    { value = "user_id", tokens_count = 1, add_tokens_every = 30 },
]
```

### Configuration Parameters Explained

- `strategy`: The type of rate limiting to apply (`url`, `ip`, `header`, `query`, or `body`)
- `global_bucket`: Global rate limit settings that apply to all values for the strategy
  - `tokens_count`: Number of tokens (requests) allowed
  - `add_tokens_every`: Time in seconds after which tokens are replenished
- `buckets_per_value`: Specific rate limits for individual values
  - `value`: The specific value to apply the limit to (e.g., URL path, header name, query parameter)
  - `tokens_count`: Number of tokens (requests) allowed for this specific value
  - `add_tokens_every`: Time in seconds after which tokens are replenished for this value

## Usage Examples

### Example 1: Basic URL Rate Limiting

```toml
[[rate_limiter.limiter]]
strategy = "url"
buckets_per_value = [
    { value = "/api/users", tokens_count = 5, add_tokens_every = 60 },  # 5 requests per minute
]
```

### Example 2: Combined IP and Header Rate Limiting

```toml
[[rate_limiter.limiter]]
strategy = "ip"
global_bucket = { tokens_count = 100, add_tokens_every = 3600 }  # 100 requests per hour per IP

[[rate_limiter.limiter]]
strategy = "header"
buckets_per_value = [
    { value = "API-Key", tokens_count = 1000, add_tokens_every = 3600 },  # 1000 requests per hour per API key
]
```

## Error Responses

When rate limits are exceeded, the service will return:
- HTTP Status Code: 429 (Too Many Requests)
- A message indicating the rate limit has been exceeded

## Notes

- The rate limiter uses a token bucket algorithm implemented with Redis
- Whitelisted IPs bypass all rate limiting rules
- Each strategy can have both global and specific limits (Except `query` and `body`)
- Token buckets are replenished gradually over time
- Configuration changes require service restart to take effect
