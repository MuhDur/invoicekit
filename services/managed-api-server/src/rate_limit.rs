// SPDX-License-Identifier: Apache-2.0
// Copyright 2026 The InvoiceKit Authors
//! T-134 per-tenant token-bucket rate limiter.
//!
//! Token buckets are keyed by `(tenant_id, route_key)`. Each bucket
//! has a `capacity` (burst size) and a `refill_per_second`
//! (steady-state rate). Every call to [`TokenBucketRateLimiter::take`]
//! returns either `Ok(remaining)` with the new bucket level OR
//! `Err(RateLimited { retry_after_ms })` so the HTTP layer can stamp
//! a 429 + `Retry-After` header in the same shape Stripe and GitHub
//! return.
//!
//! No `notify`/`futures` machinery — the limiter is a stateful
//! `Mutex<HashMap<...>>` consulted synchronously per request. That
//! costs one mutex acquire per call but keeps the code dependency-
//! free, deterministic in tests, and easy to swap for a Redis-backed
//! `TokenBucketLuaScript` implementation when production scale
//! requires it.

use std::collections::HashMap;
use std::sync::Mutex;
use std::time::{Duration, Instant};

use invoicekit_managed_api::TenantId;

/// Per-(tenant, route) policy tuple.
#[derive(Clone, Copy, Debug)]
pub struct RateLimitPolicy {
    /// Max tokens in the bucket (burst size).
    pub capacity: u32,
    /// Tokens added per second (steady-state allowed rate).
    pub refill_per_second: f64,
}

impl Default for RateLimitPolicy {
    /// Permissive default: 30-token burst + 6 req/sec sustained.
    /// Picked so a typical SDK that issues a handful of requests
    /// per user action never trips the limiter, but a tight retry
    /// loop fires the 429 within a second.
    fn default() -> Self {
        Self {
            capacity: 30,
            refill_per_second: 6.0,
        }
    }
}

/// Reason a `take` call was rejected.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct RateLimited {
    /// Milliseconds the caller should wait before retrying. The
    /// HTTP layer stamps this on the `Retry-After` header.
    pub retry_after_ms: u32,
}

/// Per-tenant rate limiter.
pub struct TokenBucketRateLimiter {
    default_policy: RateLimitPolicy,
    per_route_policy: HashMap<String, RateLimitPolicy>,
    buckets: Mutex<HashMap<(TenantId, String), BucketState>>,
}

impl std::fmt::Debug for TokenBucketRateLimiter {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("TokenBucketRateLimiter")
            .field("default_policy", &self.default_policy)
            .field("per_route_overrides", &self.per_route_policy.len())
            .finish_non_exhaustive()
    }
}

impl TokenBucketRateLimiter {
    /// New limiter with the default policy applied to every route.
    #[must_use]
    pub fn with_default_policy() -> Self {
        Self {
            default_policy: RateLimitPolicy::default(),
            per_route_policy: HashMap::new(),
            buckets: Mutex::new(HashMap::new()),
        }
    }

    /// New limiter with a custom default policy.
    #[must_use]
    pub fn with_policy(default_policy: RateLimitPolicy) -> Self {
        Self {
            default_policy,
            per_route_policy: HashMap::new(),
            buckets: Mutex::new(HashMap::new()),
        }
    }

    /// Override the policy for a specific route. Routes are matched
    /// by exact key (the handler passes its own canonical key like
    /// `"GET /v1/audit/events"`).
    #[must_use]
    pub fn with_route_override(
        mut self,
        route: impl Into<String>,
        policy: RateLimitPolicy,
    ) -> Self {
        self.per_route_policy.insert(route.into(), policy);
        self
    }

    /// Take one token for `(tenant, route)`. Returns the remaining
    /// token count when accepted; returns [`RateLimited`] otherwise.
    ///
    /// # Errors
    ///
    /// Returns `Err(RateLimited)` when the bucket is empty; the
    /// caller maps this to HTTP 429 + Retry-After.
    pub fn take(&self, tenant: &TenantId, route: &str) -> Result<f64, RateLimited> {
        self.take_at(tenant, route, Instant::now())
    }

    /// Like [`Self::take`] but lets tests pin the clock.
    ///
    /// # Errors
    ///
    /// Same as [`Self::take`].
    pub fn take_at(
        &self,
        tenant: &TenantId,
        route: &str,
        now: Instant,
    ) -> Result<f64, RateLimited> {
        let policy = self
            .per_route_policy
            .get(route)
            .copied()
            .unwrap_or(self.default_policy);
        let mut buckets = self.buckets.lock().expect("rate-limit lock poisoned");
        let key = (tenant.clone(), route.to_owned());
        let bucket = buckets.entry(key).or_insert(BucketState {
            tokens: f64::from(policy.capacity),
            last_refill: now,
        });
        // Refill since last touch.
        let elapsed = now
            .checked_duration_since(bucket.last_refill)
            .unwrap_or(Duration::ZERO);
        let added = elapsed.as_secs_f64() * policy.refill_per_second;
        bucket.tokens = (bucket.tokens + added).min(f64::from(policy.capacity));
        bucket.last_refill = now;

        if bucket.tokens >= 1.0 {
            bucket.tokens -= 1.0;
            Ok(bucket.tokens)
        } else {
            let missing = 1.0 - bucket.tokens;
            let wait_secs = missing / policy.refill_per_second;
            let retry_after_ms = (wait_secs * 1000.0).ceil() as u32;
            Err(RateLimited {
                retry_after_ms: retry_after_ms.max(1),
            })
        }
    }
}

#[derive(Debug)]
struct BucketState {
    tokens: f64,
    last_refill: Instant,
}

#[cfg(test)]
mod tests {
    use super::*;

    fn tenant(s: &str) -> TenantId {
        TenantId::new(s).unwrap()
    }

    #[test]
    fn fresh_bucket_starts_with_full_capacity() {
        let limiter = TokenBucketRateLimiter::with_policy(RateLimitPolicy {
            capacity: 5,
            refill_per_second: 1.0,
        });
        let remaining = limiter.take(&tenant("tenant_a"), "GET /v1/test").unwrap();
        assert!((4.0 - remaining).abs() < 1e-6, "got {remaining}");
    }

    #[test]
    fn bucket_drains_to_zero_then_rate_limits() {
        let limiter = TokenBucketRateLimiter::with_policy(RateLimitPolicy {
            capacity: 3,
            refill_per_second: 1.0,
        });
        let route = "GET /v1/test";
        let t = tenant("tenant_a");
        for _ in 0..3 {
            limiter.take(&t, route).unwrap();
        }
        let err = limiter.take(&t, route).unwrap_err();
        assert!(err.retry_after_ms >= 1, "got {err:?}");
    }

    #[test]
    fn bucket_refills_over_time() {
        let limiter = TokenBucketRateLimiter::with_policy(RateLimitPolicy {
            capacity: 2,
            refill_per_second: 10.0,
        });
        let route = "GET /v1/test";
        let t = tenant("tenant_a");
        let t0 = Instant::now();
        limiter.take_at(&t, route, t0).unwrap();
        limiter.take_at(&t, route, t0).unwrap();
        assert!(limiter.take_at(&t, route, t0).is_err(), "should be empty");
        // 1 second later → 10 tokens added, capped at capacity 2.
        let t1 = t0 + Duration::from_secs(1);
        let remaining = limiter.take_at(&t, route, t1).unwrap();
        assert!((1.0 - remaining).abs() < 1e-6);
    }

    #[test]
    fn per_tenant_isolation_holds() {
        let limiter = TokenBucketRateLimiter::with_policy(RateLimitPolicy {
            capacity: 1,
            refill_per_second: 0.001,
        });
        let route = "GET /v1/test";
        limiter.take(&tenant("tenant_a"), route).unwrap();
        // tenant_b has its own bucket, must not be drained.
        limiter.take(&tenant("tenant_b"), route).unwrap();
        // tenant_a is now empty.
        assert!(limiter.take(&tenant("tenant_a"), route).is_err());
    }

    #[test]
    fn per_route_override_takes_precedence() {
        let limiter = TokenBucketRateLimiter::with_policy(RateLimitPolicy {
            capacity: 10,
            refill_per_second: 5.0,
        })
        .with_route_override(
            "POST /v1/expensive",
            RateLimitPolicy {
                capacity: 1,
                refill_per_second: 0.001,
            },
        );
        let t = tenant("tenant_a");
        // Default route: 10-token burst available.
        for _ in 0..10 {
            limiter.take(&t, "GET /v1/test").unwrap();
        }
        // Expensive route: 1-token burst; second call rate-limited.
        limiter.take(&t, "POST /v1/expensive").unwrap();
        assert!(limiter.take(&t, "POST /v1/expensive").is_err());
    }

    #[test]
    fn rate_limited_retry_after_is_at_least_one_millisecond() {
        let limiter = TokenBucketRateLimiter::with_policy(RateLimitPolicy {
            capacity: 1,
            refill_per_second: 1_000_000.0, // Refill so fast the math could round to 0.
        });
        let t = tenant("tenant_a");
        let route = "GET /v1/test";
        let t0 = Instant::now();
        limiter.take_at(&t, route, t0).unwrap();
        let err = limiter.take_at(&t, route, t0).unwrap_err();
        assert!(err.retry_after_ms >= 1, "got {err:?}");
    }
}
