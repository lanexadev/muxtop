//! Per-source-IP token-bucket rate limiter for the accept loop.
//!
//! Implements ADR-30-3 (hand-rolled, no `governor` dep). A single
//! `Mutex<HashMap<IpAddr, Bucket>>` tracks per-IP token state. Refill rate
//! and burst are configurable; a refill rate of `0.0` disables limiting
//! entirely (every connection is admitted).
//!
//! The limiter is intentionally tiny (~30 lines of real logic): on each
//! `try_admit(ip)` call we lazily refill the bucket based on elapsed wall
//! time, then check if a single token is available.

use std::collections::HashMap;
use std::net::IpAddr;
use std::sync::Mutex;
use std::time::Instant;

/// Per-IP bucket state: timestamp of last refill + current token count.
#[derive(Debug, Clone, Copy)]
struct Bucket {
    last_refill: Instant,
    tokens: f32,
}

/// Per-source-IP token-bucket rate limiter.
///
/// `refill_per_sec = 0.0` disables limiting (every call returns `true`).
#[derive(Debug)]
pub struct RateLimiter {
    refill_per_sec: f32,
    burst: f32,
    buckets: Mutex<HashMap<IpAddr, Bucket>>,
}

impl RateLimiter {
    /// Create a new limiter. `refill_per_sec = 0.0` disables limiting.
    pub fn new(refill_per_sec: f32, burst: f32) -> Self {
        Self {
            refill_per_sec,
            burst,
            buckets: Mutex::new(HashMap::new()),
        }
    }

    /// Returns `true` if a connection from `ip` should be admitted (and
    /// debits one token from its bucket); `false` if the bucket is empty.
    ///
    /// When the limiter is disabled (`refill_per_sec == 0.0`) this is a
    /// no-op that always returns `true` and never touches the bucket map.
    pub fn try_admit(&self, ip: IpAddr) -> bool {
        if self.refill_per_sec == 0.0 {
            return true;
        }

        let now = Instant::now();
        let mut buckets = match self.buckets.lock() {
            Ok(g) => g,
            // Poisoned mutex: fall open rather than starve everyone.
            Err(p) => p.into_inner(),
        };

        let bucket = buckets.entry(ip).or_insert(Bucket {
            last_refill: now,
            tokens: self.burst,
        });

        let elapsed = now.duration_since(bucket.last_refill).as_secs_f32();
        bucket.tokens = (bucket.tokens + elapsed * self.refill_per_sec).min(self.burst);
        bucket.last_refill = now;

        if bucket.tokens >= 1.0 {
            bucket.tokens -= 1.0;
            true
        } else {
            false
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::net::Ipv4Addr;
    use std::thread::sleep;
    use std::time::Duration;

    fn ip() -> IpAddr {
        IpAddr::V4(Ipv4Addr::new(127, 0, 0, 1))
    }

    #[test]
    fn test_disabled_admits_unconditionally() {
        let rl = RateLimiter::new(0.0, 10.0);
        for _ in 0..1_000 {
            assert!(rl.try_admit(ip()));
        }
    }

    #[test]
    fn test_burst_then_reject() {
        let rl = RateLimiter::new(10.0, 10.0);
        // First 10 (the burst) succeed.
        for i in 0..10 {
            assert!(rl.try_admit(ip()), "admit #{i} of burst should pass");
        }
        // 11th must fail (no time elapsed → no refill).
        assert!(!rl.try_admit(ip()), "11th attempt must be rate-limited");
    }

    #[test]
    fn test_refill_after_time() {
        let rl = RateLimiter::new(20.0, 5.0);
        // Drain the burst.
        for _ in 0..5 {
            assert!(rl.try_admit(ip()));
        }
        assert!(!rl.try_admit(ip()), "drained bucket must reject");

        // Wait long enough to refill ≥ 2 tokens (20/s × 0.15s = 3 tokens).
        sleep(Duration::from_millis(150));
        assert!(rl.try_admit(ip()), "after refill, should admit again");
        assert!(
            rl.try_admit(ip()),
            "second refilled token should also admit"
        );
    }

    #[test]
    fn test_per_ip_isolation() {
        let rl = RateLimiter::new(1.0, 1.0);
        let a = IpAddr::V4(Ipv4Addr::new(10, 0, 0, 1));
        let b = IpAddr::V4(Ipv4Addr::new(10, 0, 0, 2));

        assert!(rl.try_admit(a));
        assert!(!rl.try_admit(a), "A is now drained");
        // B has its own bucket; should still have a token.
        assert!(rl.try_admit(b));
    }

    #[test]
    fn test_burst_cap_after_long_idle() {
        // Even after a long idle period, the bucket cannot exceed `burst`
        // tokens — i.e. you don't get to "save up" credit indefinitely.
        let rl = RateLimiter::new(2.0, 5.0);
        sleep(Duration::from_millis(50));
        // First call refills (5 + 50ms*2/s ≈ 5.1, capped at 5) and debits 1
        // → 4 left. Subsequent rapid calls do tiny refills, so we can drain
        // ~5 quickly (the burst cap) and then must wait.
        let mut admitted = 0;
        for _ in 0..20 {
            if rl.try_admit(ip()) {
                admitted += 1;
            }
        }
        // We expect roughly the burst cap (5) plus ≤ 1-2 stragglers from
        // micro-elapsed refills during the loop. The hard upper bound is
        // burst + 1 (worst case: a few microseconds elapsed = a few extra
        // tokens at refill_per_sec=2.0 → still under burst+1 in practice).
        assert!(
            admitted <= 6,
            "burst cap must hold (got {admitted} admits in tight loop)"
        );
    }
}
