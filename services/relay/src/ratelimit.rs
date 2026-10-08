//! Limitação de taxa em memória (token bucket), sem serviços externos.

use std::collections::HashMap;
use std::net::IpAddr;
use std::sync::Mutex;
use std::time::Instant;

#[derive(Debug, Clone)]
pub struct TokenBucket {
    capacity: f64,
    tokens: f64,
    refill_per_sec: f64,
    last: Instant,
}

impl TokenBucket {
    pub fn new(capacity: f64, refill_per_sec: f64, now: Instant) -> Self {
        Self {
            capacity,
            tokens: capacity,
            refill_per_sec,
            last: now,
        }
    }

    fn refill(&mut self, now: Instant) {
        let elapsed = now.saturating_duration_since(self.last).as_secs_f64();
        self.tokens = (self.tokens + elapsed * self.refill_per_sec).min(self.capacity);
        self.last = now;
    }

    /// Consome `amount` se houver saldo.
    pub fn try_take(&mut self, amount: f64, now: Instant) -> bool {
        self.refill(now);
        if self.tokens >= amount {
            self.tokens -= amount;
            true
        } else {
            false
        }
    }

    fn is_full(&mut self, now: Instant) -> bool {
        self.refill(now);
        self.tokens >= self.capacity
    }
}

/// Um bucket por endereço IP.
pub struct IpRateLimiter {
    buckets: Mutex<HashMap<IpAddr, TokenBucket>>,
    capacity: f64,
    refill_per_sec: f64,
}

/// Limite de IPs rastreados; acima disso, buckets cheios são descartados.
const MAX_TRACKED_IPS: usize = 100_000;

impl IpRateLimiter {
    pub fn new(capacity: f64, refill_per_sec: f64) -> Self {
        Self {
            buckets: Mutex::new(HashMap::new()),
            capacity,
            refill_per_sec,
        }
    }

    pub fn check(&self, ip: IpAddr) -> bool {
        let now = Instant::now();
        let Ok(mut buckets) = self.buckets.lock() else {
            return false;
        };
        if buckets.len() >= MAX_TRACKED_IPS && !buckets.contains_key(&ip) {
            buckets.retain(|_, b| !b.is_full(now));
            if buckets.len() >= MAX_TRACKED_IPS {
                return false;
            }
        }
        buckets
            .entry(ip)
            .or_insert_with(|| TokenBucket::new(self.capacity, self.refill_per_sec, now))
            .try_take(1.0, now)
    }

    /// Remove buckets que já se recompuseram (IPs inativos).
    pub fn prune(&self) {
        let now = Instant::now();
        if let Ok(mut buckets) = self.buckets.lock() {
            buckets.retain(|_, b| !b.is_full(now));
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::Duration;

    #[test]
    fn bucket_refills_over_time() {
        let t0 = Instant::now();
        let mut b = TokenBucket::new(2.0, 1.0, t0);
        assert!(b.try_take(1.0, t0));
        assert!(b.try_take(1.0, t0));
        assert!(!b.try_take(1.0, t0));
        assert!(!b.try_take(1.0, t0 + Duration::from_millis(500)));
        assert!(b.try_take(1.0, t0 + Duration::from_millis(1100)));
        // Nunca passa da capacidade.
        let later = t0 + Duration::from_secs(100);
        assert!(b.try_take(2.0, later));
        assert!(!b.try_take(1.0, later));
    }

    #[test]
    fn limiter_is_per_ip() {
        let l = IpRateLimiter::new(1.0, 0.0);
        let a: IpAddr = [10, 0, 0, 1].into();
        let b: IpAddr = [10, 0, 0, 2].into();
        assert!(l.check(a));
        assert!(!l.check(a));
        assert!(l.check(b));
    }
}
