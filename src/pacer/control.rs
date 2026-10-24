use crate::rtp_::Bitrate;

const PACING_FACTOR: f64 = 1.1;

/// Target padding rate when media is active. This maintains NAT bindings, RTX state,
/// and allows ALR periodic probes to discover higher bandwidth.
const PADDING_TARGET: Bitrate = Bitrate::bps(50_000);

pub(crate) struct PacingResult {
    /// The bitrate at which the pacer may emit padding **when there is no media queued**.
    ///
    /// This value is intentionally an *absolute* target used by the pacer, not a “delta to add
    /// on top of current media”. In practice the **effective padding** over time is the
    /// difference between this target and the media actually sent, because padding is only used
    /// to fill gaps (empty queues), not to continuously top-up while media is flowing.
    pub padding_rate: Bitrate,
    pub pacing_rate: Bitrate,
}

/// Controls the pacing and padding rates.
pub(crate) struct PacerControl {
    /// Total rate the application wants padding to fill up to, capped by the estimate.
    padding_target: Option<Bitrate>,
}

impl PacerControl {
    pub fn new() -> Self {
        Self {
            padding_target: None,
        }
    }

    pub fn set_padding_target(&mut self, target: Option<Bitrate>) {
        self.padding_target = target;
    }

    pub fn calculate(
        &self,
        has_active_media: bool,
        estimate: Bitrate,
        is_overuse: bool,
    ) -> PacingResult {
        // ALR periodic probes handle bandwidth discovery, not continuous padding, unless the
        // application sets a padding target.
        let padding_rate = if is_overuse {
            // No padding during overuse
            Bitrate::ZERO
        } else if has_active_media {
            // Pad to 50 kbps to maintain NAT bindings and RTX state, or to what the application
            // asked for so that the acknowledged rate is up before it sends more media.
            match self.padding_target {
                Some(target) => target.min(estimate).max(PADDING_TARGET),
                None => PADDING_TARGET,
            }
        } else {
            // No padding when no media is being sent
            Bitrate::ZERO
        };

        // Set pacing rate to smooth out media transmission (burst avoidance).
        // Must be at least the current BWE estimate * factor, but also high enough
        // to allow the padding we want to send.
        let min_pacing_rate = estimate * PACING_FACTOR;
        let pacing_rate = min_pacing_rate.max(padding_rate);

        PacingResult {
            padding_rate,
            pacing_rate,
        }
    }
}

#[cfg(test)]
mod test {
    use super::*;

    #[test]
    fn padding_target_raises_padding_up_to_the_estimate() {
        let mut c = PacerControl::new();
        c.set_padding_target(Some(Bitrate::kbps(1_800)));
        assert_eq!(
            c.calculate(true, Bitrate::kbps(2_200), false).padding_rate,
            Bitrate::kbps(1_800)
        );
        // Never above the estimate.
        assert_eq!(
            c.calculate(true, Bitrate::kbps(1_000), false).padding_rate,
            Bitrate::kbps(1_000)
        );
        // Overuse still stops padding.
        assert_eq!(
            c.calculate(true, Bitrate::kbps(2_200), true).padding_rate,
            Bitrate::ZERO
        );
        // Cleared: back to the keep-alive rate.
        c.set_padding_target(None);
        assert_eq!(
            c.calculate(true, Bitrate::kbps(2_200), false).padding_rate,
            PADDING_TARGET
        );
    }

    #[test]
    fn padding_enabled_with_active_media() {
        let c = PacerControl::new();
        let estimate = Bitrate::kbps(1_000);

        let r = c.calculate(true, estimate, false);

        assert_eq!(r.padding_rate, PADDING_TARGET);
    }

    #[test]
    fn no_padding_without_active_media() {
        let c = PacerControl::new();
        let estimate = Bitrate::kbps(1_000);

        let r = c.calculate(false, estimate, false);

        assert_eq!(r.padding_rate, Bitrate::ZERO);
    }

    #[test]
    fn overuse_suppresses_padding() {
        let c = PacerControl::new();
        let estimate = Bitrate::mbps(40);

        let r = c.calculate(true, estimate, true);
        assert_eq!(r.padding_rate, Bitrate::ZERO);
    }
}
