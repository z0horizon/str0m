//! Retransmission rate limiting as libwebrtc does it.

use std::collections::VecDeque;
use std::time::{Duration, Instant};

use crate::rtp_::Bitrate;

/// libwebrtc `kRetransmitWindowSizeMs`.
const WINDOW: Duration = Duration::from_millis(500);

/// Share of the estimate retransmissions may use (libwebrtc kProtectionOverheadRateThreshold).
const MAX_SHARE: f64 = 0.5;

/// Caps retransmitted bytes per session at half the current send-side estimate over a 500 ms window,
/// as libwebrtc's `RateLimiter` does for `RTPSender::ReSendPacket`. A NACKed packet that would
/// go over is not retransmitted; the others still are.
#[derive(Debug, Default)]
pub(crate) struct RetransmissionLimiter {
    max_rate: Option<Bitrate>,
    sent: VecDeque<(Instant, usize)>,
    sent_bytes: usize,
}

impl RetransmissionLimiter {
    pub(crate) fn new() -> Self {
        Self::default()
    }

    /// The current estimate, or `None` when there is none (no limit).
    pub(crate) fn set_max_rate(&mut self, max_rate: Option<Bitrate>) {
        self.max_rate = max_rate;
    }

    /// True, and the bytes are counted, if `bytes` more stay within the budget: half the estimate for ordinary resends, the whole estimate for essential ones (libwebrtc's ceiling).
    pub(crate) fn try_use(&mut self, bytes: usize, now: Instant, essential: bool) -> bool {
        while let Some(&(at, size)) = self.sent.front() {
            if now.saturating_duration_since(at) >= WINDOW {
                self.sent.pop_front();
                self.sent_bytes -= size;
            } else {
                break;
            }
        }
        if let Some(max_rate) = self.max_rate {
            let share = if essential { 1.0 } else { MAX_SHARE };
            let budget = (max_rate.as_f64() * share * WINDOW.as_secs_f64() / 8.0) as usize;
            if self.sent_bytes + bytes > budget {
                return false;
            }
        }
        self.sent.push_back((now, bytes));
        self.sent_bytes += bytes;
        true
    }
}

#[cfg(test)]
mod test {
    use super::*;
    use std::time::{Duration, Instant};

    #[test]
    fn essential_packets_may_use_the_whole_estimate() {
        let now = Instant::now();
        let mut l = RetransmissionLimiter::new();
        // 1 Mbps over 500 ms: 31_250 bytes for ordinary resends, 62_500 in all for essential ones.
        l.set_max_rate(Some(Bitrate::bps(1_000_000)));
        assert!(l.try_use(31_000, now, false));
        assert!(!l.try_use(1_000, now, false));
        assert!(l.try_use(31_000, now, true));
        assert!(!l.try_use(1_000, now, true));
    }

    #[test]
    fn the_limit_is_half_the_estimate() {
        let now = Instant::now();
        let mut l = RetransmissionLimiter::new();
        // Estimate 1 Mbps: 0.5 x 1 Mbps over 500 ms = 31_250 bytes.
        l.set_max_rate(Some(Bitrate::bps(1_000_000)));
        assert!(l.try_use(31_000, now, false));
        assert!(!l.try_use(500, now, false));
    }

    #[test]
    fn allows_up_to_the_max_rate_over_the_window() {
        let now = Instant::now();
        let mut l = RetransmissionLimiter::new();
        // 1 Mbps over 500 ms = 62_500 bytes.
        l.set_max_rate(Some(Bitrate::bps(1_000_000)));
        assert!(l.try_use(30_000, now, false));
        assert!(!l.try_use(2_000, now + Duration::from_millis(100), false));
        assert!(l.try_use(1_000, now + Duration::from_millis(100), false));
    }

    #[test]
    fn budget_returns_once_the_window_moves_on() {
        let now = Instant::now();
        let mut l = RetransmissionLimiter::new();
        l.set_max_rate(Some(Bitrate::bps(1_000_000)));
        assert!(l.try_use(31_250, now, false));
        assert!(!l.try_use(1, now + Duration::from_millis(499), false));
        assert!(l.try_use(31_250, now + Duration::from_millis(501), false));
    }

    #[test]
    fn no_max_rate_means_no_limit() {
        let now = Instant::now();
        let mut l = RetransmissionLimiter::new();
        assert!(l.try_use(10_000_000, now, false));
    }
}
