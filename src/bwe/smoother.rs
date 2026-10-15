use std::collections::VecDeque;
use std::time::{Duration, Instant};

use crate::rtp_::Bitrate;

/// Time window for averaging estimate changes.
const ESTIMATE_WINDOW: Duration = Duration::from_secs(3);

type TimeBitrate = (Instant, Bitrate);

const TOLERANCE: f64 = 0.05;

/// Smooths BWE estimates by averaging over a time window.
pub struct EstimateSmoother {
    estimates: VecDeque<TimeBitrate>,
    maybe_emit: bool,
    emitted: Option<Bitrate>,
}

impl EstimateSmoother {
    pub fn new() -> Self {
        Self {
            estimates: VecDeque::new(),
            maybe_emit: false,
            emitted: None,
        }
    }

    /// Record a new estimate and update the smoothed average.
    pub fn record(&mut self, now: Instant, estimate: Bitrate) {
        // Did value change from previous?
        let do_update = self.estimates.back().map(|b| b.1) != Some(estimate);

        if do_update {
            self.maybe_emit = true;
            self.estimates.push_back((now, estimate));
        }

        // Remove entries older than the window.
        while let Some((time, _)) = self.estimates.front() {
            if now.duration_since(*time) > ESTIMATE_WINDOW {
                // Keep last entry
                if self.estimates.len() == 1 {
                    break;
                }

                self.maybe_emit = true;
                self.estimates.pop_front();
            } else {
                break;
            }
        }
    }

    /// Start the window again from `estimate`, and emit it on the next poll.
    ///
    /// Used when a probe result raises the estimate: averaging it with the estimates from before
    /// the probe would only delay it.
    pub fn restart(&mut self, now: Instant, estimate: Bitrate) {
        self.estimates.clear();
        self.estimates.push_back((now, estimate));
        self.maybe_emit = true;
    }

    /// Poll for an estimate to emit. Returns Some only when there's a new value to emit.
    pub fn poll(&mut self) -> Option<Bitrate> {
        if !self.maybe_emit {
            return None;
        }

        if self.estimates.is_empty() {
            return None;
        }

        let total: f64 = self.estimates.iter().map(|b| b.1.as_f64()).sum();
        let avg = total / self.estimates.len() as f64;
        let rate: Bitrate = avg.into();

        // This forces emitting if we have the first ever value, or a last
        // where the rest of the window is gone (estimates stop coming).
        let force = self.estimates.len() == 1 && self.emitted != Some(rate);

        // Emit if we deviate enough from previously emitted.
        let deviate = if let Some(emitted) = self.emitted {
            !in_tolerance(emitted, rate)
        } else {
            true
        };

        self.maybe_emit = false;

        // Are we not to emit?
        if !force && !deviate {
            return None;
        }

        self.emitted = Some(rate);
        Some(rate)
    }
}

fn in_tolerance(b1: Bitrate, b2: Bitrate) -> bool {
    let min = b1 * (1.0 - TOLERANCE);
    let max = b1 * (1.0 + TOLERANCE);
    b2 > min && b2 < max
}

#[cfg(test)]
mod test {
    use super::*;

    #[test]
    fn restart_drops_the_window_and_emits_the_new_value() {
        let t0 = Instant::now();
        let ms = Duration::from_millis;
        let mut s = EstimateSmoother::new();

        s.record(t0, Bitrate::kbps(400));
        assert_eq!(s.poll(), Some(Bitrate::kbps(400)));

        // A probe result: the window starts again from it, and readers of the smoothed estimate
        // see it at once.
        s.restart(t0 + ms(100), Bitrate::kbps(1_600));
        assert_eq!(s.poll(), Some(Bitrate::kbps(1_600)));

        s.record(t0 + ms(200), Bitrate::kbps(1_600));
        assert_eq!(s.poll(), None);

        // The average no longer includes the pre-probe 400 kbps.
        s.record(t0 + ms(300), Bitrate::kbps(1_000));
        assert_eq!(s.poll(), Some(Bitrate::kbps(1_300)));
    }
}
