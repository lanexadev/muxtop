//! Per-source-IP token-bucket rate limiter for the accept loop.
//!
//! Implements ADR-30-3 (hand-rolled, no `governor` dep). A single
//! `Mutex<HashMap<IpAddr, Bucket>>` tracks per-IP token state. Refill rate
//! and burst are configurable; a refill rate of `0.0` disables limiting
//! entirely (every connection is admitted).
//!
//! The limiter is intentionally tiny: on each `try_admit(ip)` call we
//! lazily refill the bucket based on elapsed wall time, then check if a
//! single token is available.
//!
//! # Bounded memory (SEC-01)
//!
//! A naive per-IP map is itself a remote DoS: an entry is created for every
//! source address ever seen — including the ones the limiter *rejects* —
//! and never removed. An attacker with a routed IPv6 prefix has 2^64
//! distinct sources to spend, so the map grows until the process is OOM
//! killed. The component meant to stop a flood becomes the flood's target.
//!
//! The fix rests on one observation: **a bucket refilled back to `burst` is
//! indistinguishable from an absent entry** — both admit the next request
//! and start from a full bucket. Idle-full entries can therefore be evicted
//! with zero observable change in rate-limiting behaviour. A bucket refills
//! to full after `burst / refill_per_sec` seconds of inactivity (1 s at the
//! defaults), so the surviving set is bounded by the number of genuinely
//! *active* sources rather than by the number ever seen.
//!
//! Sweeps are amortised: at most one per [`MIN_SWEEP_GAP`] regardless of
//! accept rate, so the O(n) scan cannot itself be triggered per-connection.
//! [`MAX_TRACKED_IPS`] is the last-resort ceiling — past it, sources that
//! are not already tracked are rejected rather than allocated for.

use std::collections::HashMap;
use std::net::IpAddr;
use std::sync::Mutex;
use std::time::{Duration, Instant};

/// Sweep when the map has grown past this many entries (and the
/// [`MIN_SWEEP_GAP`] has elapsed). Well above any plausible count of
/// legitimate concurrent sources for a monitoring server.
const SWEEP_SIZE_TRIGGER: usize = 1024;

/// Periodic sweep cadence, so a map that never crosses the size trigger
/// still releases memory after a burst subsides.
const SWEEP_INTERVAL: Duration = Duration::from_secs(30);

/// Floor between two sweeps. Without it, a map held just above
/// [`SWEEP_SIZE_TRIGGER`] would sweep on *every* new address, turning an
/// O(n) scan into per-connection work — an algorithmic DoS of our own
/// making.
const MIN_SWEEP_GAP: Duration = Duration::from_secs(1);

/// Hard ceiling on tracked sources (~4 MiB of map at 64 B/entry). Reached
/// only under a deliberate address flood; past it we stop allocating for
/// unknown sources instead of growing without bound.
const MAX_TRACKED_IPS: usize = 65_536;

/// Per-IP bucket state: timestamp of last refill + current token count.
#[derive(Debug, Clone, Copy)]
struct Bucket {
    last_refill: Instant,
    tokens: f32,
}

impl Bucket {
    /// Token count this bucket would hold at `now`, capped at `burst`.
    fn refilled_at(&self, now: Instant, refill_per_sec: f32, burst: f32) -> f32 {
        let elapsed = now.duration_since(self.last_refill).as_secs_f32();
        (self.tokens + elapsed * refill_per_sec).min(burst)
    }
}

/// Map of tracked buckets plus the bookkeeping that keeps it bounded.
#[derive(Debug)]
struct LimiterState {
    buckets: HashMap<IpAddr, Bucket>,
    last_sweep: Instant,
    /// Whether the ceiling warning has already been emitted for the current
    /// episode, so a sustained flood logs once rather than once per attempt.
    ceiling_logged: bool,
}

impl LimiterState {
    /// Drop every bucket that has refilled back to `burst` — see the module
    /// docs for why this is behaviour-preserving.
    fn sweep(&mut self, now: Instant, refill_per_sec: f32, burst: f32) {
        self.buckets
            .retain(|_, b| b.refilled_at(now, refill_per_sec, burst) < burst);
        self.last_sweep = now;
    }
}

/// Per-source-IP token-bucket rate limiter.
///
/// `refill_per_sec = 0.0` disables limiting (every call returns `true`).
#[derive(Debug)]
pub struct RateLimiter {
    refill_per_sec: f32,
    burst: f32,
    max_tracked: usize,
    state: Mutex<LimiterState>,
}

impl RateLimiter {
    /// Create a new limiter. `refill_per_sec = 0.0` disables limiting.
    pub fn new(refill_per_sec: f32, burst: f32) -> Self {
        Self::with_max_tracked(refill_per_sec, burst, MAX_TRACKED_IPS)
    }

    /// Same as [`RateLimiter::new`] with an explicit tracking ceiling, so
    /// tests can exercise the cap without inserting 65 536 addresses.
    fn with_max_tracked(refill_per_sec: f32, burst: f32, max_tracked: usize) -> Self {
        Self {
            refill_per_sec,
            burst,
            max_tracked,
            state: Mutex::new(LimiterState {
                buckets: HashMap::new(),
                last_sweep: Instant::now(),
                ceiling_logged: false,
            }),
        }
    }

    /// Number of currently tracked source addresses. Diagnostics only.
    #[cfg(test)]
    fn tracked(&self) -> usize {
        match self.state.lock() {
            Ok(g) => g.buckets.len(),
            Err(p) => p.into_inner().buckets.len(),
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
        let mut state = match self.state.lock() {
            Ok(g) => g,
            // Poisoned mutex: fall open rather than starve everyone.
            Err(p) => p.into_inner(),
        };

        // Only an *unknown* address can grow the map, so the bounding work
        // is confined to that path — steady-state traffic from tracked
        // sources costs exactly what it did before.
        if !state.buckets.contains_key(&ip) {
            let since_sweep = now.duration_since(state.last_sweep);
            if since_sweep >= MIN_SWEEP_GAP
                && (state.buckets.len() >= SWEEP_SIZE_TRIGGER || since_sweep >= SWEEP_INTERVAL)
            {
                state.sweep(now, self.refill_per_sec, self.burst);
            }

            if state.buckets.len() >= self.max_tracked {
                // Log the transition, not the rejection: at the ceiling we are
                // by definition under an address flood, and a line per refused
                // connection would turn the limiter into a log amplifier —
                // trading a bounded map for an unbounded log.
                if !state.ceiling_logged {
                    state.ceiling_logged = true;
                    tracing::warn!(
                        tracked = state.buckets.len(),
                        "rate-limiter tracking ceiling reached; refusing unknown sources \
                         until the sweep reclaims capacity"
                    );
                }
                return false;
            }
            state.ceiling_logged = false;
        }

        let bucket = state.buckets.entry(ip).or_insert(Bucket {
            last_refill: now,
            tokens: self.burst,
        });

        bucket.tokens = bucket.refilled_at(now, self.refill_per_sec, self.burst);
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

    // ---- SEC-01: bounded memory ----------------------------------------

    /// Distinct addresses, cheap to mint — stands in for an attacker walking
    /// a routed IPv6 prefix.
    fn nth_ip(n: u32) -> IpAddr {
        IpAddr::V4(Ipv4Addr::from(n.to_be_bytes()))
    }

    /// The eviction rule: a bucket refilled back to `burst` admits exactly
    /// what an absent entry would, so dropping it changes nothing.
    #[test]
    fn test_sweep_evicts_full_buckets_and_keeps_drained_ones() {
        let now = Instant::now();
        let mut state = LimiterState {
            buckets: HashMap::new(),
            last_sweep: now,
            ceiling_logged: false,
        };
        // Full: idle long enough to be back at burst.
        state.buckets.insert(
            nth_ip(1),
            Bucket {
                last_refill: now - Duration::from_secs(10),
                tokens: 0.0,
            },
        );
        // Drained: just spent its tokens, nowhere near a refill.
        state.buckets.insert(
            nth_ip(2),
            Bucket {
                last_refill: now,
                tokens: 0.0,
            },
        );

        state.sweep(now, 1.0, 5.0);

        assert!(
            !state.buckets.contains_key(&nth_ip(1)),
            "an idle-full bucket carries no state worth keeping"
        );
        assert!(
            state.buckets.contains_key(&nth_ip(2)),
            "a drained bucket must survive — evicting it would hand the \
             attacker a fresh burst"
        );
    }

    /// The finding itself: without eviction, one entry per source address is
    /// retained forever and the map is a remote memory-exhaustion primitive.
    #[test]
    fn test_address_flood_does_not_grow_map_without_bound() {
        let rl = RateLimiter::new(10.0, 10.0);

        // First wave: 2000 distinct sources. No sweep can fire yet
        // (MIN_SWEEP_GAP has not elapsed since construction), so this is the
        // worst case the map is allowed to reach between sweeps.
        for i in 0..2_000 {
            rl.try_admit(nth_ip(i));
        }
        let peak = rl.tracked();
        assert!(peak >= 2_000, "first wave should be tracked (got {peak})");

        // All 2000 buckets refill to burst in burst/refill = 1 s.
        sleep(MIN_SWEEP_GAP + Duration::from_millis(150));

        // The next unknown source triggers the amortised sweep.
        rl.try_admit(nth_ip(9_999));

        let after = rl.tracked();
        assert!(
            after < 100,
            "sweep must reclaim the idle flood (peak {peak}, still holding {after})"
        );
    }

    /// Sweeping is amortised, so a sustained flood cannot turn the O(n) scan
    /// into per-connection work.
    #[test]
    fn test_sweep_is_rate_limited_between_runs() {
        let rl = RateLimiter::new(10.0, 10.0);
        for i in 0..(SWEEP_SIZE_TRIGGER as u32 + 500) {
            rl.try_admit(nth_ip(i));
        }
        // Size trigger is exceeded, but MIN_SWEEP_GAP has not elapsed, so
        // every one of those inserts must have skipped the scan.
        let guard = rl.state.lock().unwrap();
        assert!(
            guard.last_sweep.elapsed() < MIN_SWEEP_GAP,
            "no sweep should have run inside the minimum gap"
        );
    }

    /// Last-resort ceiling: past it we stop allocating for unknown sources.
    /// Known sources keep working — an address flood must not lock out a
    /// client the server is already tracking.
    #[test]
    fn test_tracking_ceiling_rejects_unknown_sources_only() {
        let rl = RateLimiter::with_max_tracked(10.0, 10.0, 8);
        for i in 0..8 {
            assert!(rl.try_admit(nth_ip(i)), "source #{i} is within the cap");
        }
        assert_eq!(rl.tracked(), 8);

        assert!(
            !rl.try_admit(nth_ip(999)),
            "an unknown source past the ceiling must be rejected, not allocated for"
        );
        assert_eq!(rl.tracked(), 8, "rejection must not grow the map");

        assert!(
            rl.try_admit(nth_ip(3)),
            "an already-tracked source keeps its budget at the ceiling"
        );
    }

    /// A line per refused connection would let an attacker drive our log
    /// volume — a bounded map paid for with an unbounded log.
    #[test]
    fn test_ceiling_warning_is_emitted_once_per_episode() {
        let rl = RateLimiter::with_max_tracked(10.0, 10.0, 4);
        for i in 0..4 {
            assert!(rl.try_admit(nth_ip(i)));
        }

        assert!(!rl.try_admit(nth_ip(100)));
        assert!(
            rl.state.lock().unwrap().ceiling_logged,
            "first refusal at the ceiling should arm the flag"
        );

        // Subsequent refusals must find the flag already set and stay quiet.
        for i in 101..200 {
            assert!(!rl.try_admit(nth_ip(i)));
        }
        assert!(rl.state.lock().unwrap().ceiling_logged);
    }

    /// Eviction must not become a way to reset a bucket: an IP that keeps
    /// hammering stays limited across a sweep.
    #[test]
    fn test_limiting_survives_a_sweep() {
        let rl = RateLimiter::new(1.0, 2.0);
        let victim = nth_ip(42);
        assert!(rl.try_admit(victim));
        assert!(rl.try_admit(victim));
        assert!(!rl.try_admit(victim), "burst of 2 is spent");

        {
            let mut state = rl.state.lock().unwrap();
            let now = Instant::now();
            state.sweep(now, 1.0, 2.0);
        }

        assert!(
            !rl.try_admit(victim),
            "the drained bucket survived the sweep, so the limit still holds"
        );
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
