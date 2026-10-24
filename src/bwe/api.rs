//! Bandwidth estimation.

use crate::{Rtc, rtp_::Mid};

pub use crate::rtp_::Bitrate;

#[derive(Debug, PartialEq)]
#[non_exhaustive]
/// Bandwidth estimation kind.
pub enum BweKind {
    /// Transport wide congestion control.
    ///
    /// Emits the current estimate with `can_probe: true` when probing becomes
    /// available, initially using the configured starting bitrate. Estimate
    /// updates follow while probing is available. On loss of probing capability,
    /// emits the last reported estimate once with `can_probe: false`, then
    /// suppresses updates until probing becomes available again.
    Twcc {
        /// Estimated available bitrate. Initially this is the configured starting bitrate.
        /// When probing becomes unavailable, this retains the last estimate.
        estimate: Bitrate,
        /// Whether the local BWE can currently probe for capacity.
        ///
        /// Requires BWE to be enabled, SRTP keys, and a sending media section
        /// supporting TWCC feedback and the transport sequence extension.
        /// This does not guarantee that feedback is arriving.
        can_probe: bool,
    },
    /// A probe result raised the TWCC estimate.
    ///
    /// Emitted at once, unsmoothed, when a probe cluster measures more than the estimate had
    /// been, and followed by a [`BweKind::Twcc`] with the same estimate: the smoothing starts
    /// again from this value rather than averaging in the estimate from before the probe. An
    /// application that only reads `Twcc` sees the same estimate.
    ProbeResult {
        /// The estimate the probe result set.
        estimate: Bitrate,
    },
    /// REMB (Receiver Estimated Maximum Bitrate)
    Remb {
        /// Bitrate reported by the remote receiver.
        estimate: Bitrate,
        /// Media section associated with the report.
        mid: Mid,
    },
}

/// Access to the Bandwidth Estimate subsystem.
pub struct Bwe<'a>(pub(crate) &'a mut Rtc);

impl<'a> Bwe<'a> {
    /// Configure the desired bitrate.
    ///
    /// Configure the bandwidth estimation system with the desired bitrate.
    /// **Note:** This only has an effect if BWE has been enabled via
    /// [`RtcConfig::enable_bwe`][crate::RtcConfig::enable_bwe].
    ///
    /// * `desired_bitrate` The bitrate you would like to eventually send at. The BWE system will try
    ///   to reach this bitrate by probing with padding packets. You should allocate your media bitrate
    ///   based on the estimated the BWE system produces via
    ///   [`Event::EgressBitrateEstimate`][crate::Event::EgressBitrateEstimate]. This rate might not
    ///   be reached if the network link cannot sustain the desired bitrate.
    ///
    /// ## Example
    ///
    /// Say you have three simulcast video tracks each with a high layer configured at 1.5Mbit/s.
    /// You should then set the desired bitrate to 4.5Mbit/s (or slightly higher). If the network
    /// link can sustain 4.5Mbit/s there will eventually be an
    /// [`Event::EgressBitrateEstimate`][crate::Event::EgressBitrateEstimate] with this estimate.
    pub fn set_desired_bitrate(&mut self, desired_bitrate: Bitrate) {
        self.0.session.set_bwe_desired_bitrate(desired_bitrate);
    }

    /// Ask for a probe cluster at `target` (capped at twice the desired bitrate).
    ///
    /// For an application that knows it is short of bandwidth for its next quality step and
    /// does not want to wait for the built-in triggers (ALR every 5 s, or a stagnant estimate
    /// after 15 s).
    ///
    /// **Note:** This only has an effect if BWE has been enabled via
    /// [`RtcConfig::enable_bwe`][crate::RtcConfig::enable_bwe] and padding can be sent. The
    /// request is dropped when a built-in probe would not run either, while another probe is
    /// pending or awaits its result, and when the capped target is not above the estimate.
    pub fn request_probe(&mut self, target: Bitrate) {
        let now = self.0.last_now;
        self.0.session.request_bwe_probe(target, now);
    }

    /// Let padding fill the total send rate up to `target` (capped by the current estimate).
    ///
    /// The default is about 50 kbps. Before stepping up to a higher layer, an application can
    /// pad up to what it will send after the step. `None` returns to the default.
    ///
    /// **Note:** This only has an effect if BWE has been enabled via
    /// [`RtcConfig::enable_bwe`][crate::RtcConfig::enable_bwe] and a negotiated sending media is
    /// active. Padding still stops during overuse, and the target is kept across [`Bwe::reset`].
    pub fn set_padding_target(&mut self, target: Option<Bitrate>) {
        self.0.session.set_bwe_padding_target(target);
    }

    /// Reset the BWE with a new init_bitrate
    ///
    /// This discards the current estimator state and starts estimation again
    /// from `init_bitrate`. Normal transitions from audio-only to video sending
    /// do not require a reset: with TWCC negotiated, probing can discover
    /// capacity before video starts. The desired bitrate controls the capacity
    /// the estimator attempts to discover.
    pub fn reset(&mut self, init_bitrate: Bitrate) {
        self.0.session.reset_bwe(init_bitrate);
    }
}
