//! Application-driven probing (`Bwe::request_probe`) and padding (`Bwe::set_padding_target`).
//!
//! The scenario: after a bandwidth dip, a sender keeps running a low layer while it wants the
//! top layer. Its media rate is too low for the delay-based estimate to grow (an increase is
//! capped at 1.5x the acknowledged rate), so the estimate stays around 560 kbps on a 3 Mbps link
//! until a built-in probe gets it out, seconds later. An application that knows it is short can
//! probe, or pad, at once.

use std::time::Duration;

use netem::{DataSize, NetemConfig};
use str0m::bwe::{Bitrate, BweKind};
use str0m::{Event, RtcError};
use tracing::info;

use crate::common::{
    BweTestContext, TestRtc, connect_with_bwe, get_last_bwe_estimate, init_crypto_default, init_log,
};

const LINK: Bitrate = Bitrate::mbps(3);
const DIP: Bitrate = Bitrate::kbps(500);
const LOW_LAYER: Bitrate = Bitrate::kbps(450);
/// The estimate the next layer needs.
const RECOVERED: Bitrate = Bitrate::mbps(2);

fn link(rate: Bitrate) -> NetemConfig {
    NetemConfig::new().link(rate, DataSize::kbytes(60)).seed(42)
}

fn set_link(l: &mut TestRtc, r: &mut TestRtc, rate: Bitrate) {
    l.set_netem(link(rate));
    r.set_netem(link(rate));
}

struct Recovery {
    /// From the link being restored to the estimate reaching `RECOVERED`.
    elapsed: Option<Duration>,
    /// Estimate when the application acted.
    estimate_at_action: Bitrate,
    /// Probe targets from the application's action on.
    probes: Vec<Bitrate>,
    /// Estimate events from the link being restored on, with the time since then.
    estimates: Vec<(Duration, Estimate)>,
}

#[derive(Debug, Clone, Copy, PartialEq)]
enum Estimate {
    Twcc(Bitrate),
    ProbeResult(Bitrate),
}

/// Runs a sender at the low layer through a dip, lets `action` act 300 ms after the link is
/// restored, and measures how long the estimate takes to reach `RECOVERED`.
fn recover(action: impl FnOnce(&mut TestRtc, Bitrate)) -> Result<Recovery, RtcError> {
    // The application wants the top layer throughout, as an SFU does.
    let (mut l, mut r) = connect_with_bwe(Bitrate::kbps(450), LINK);
    // Negotiated, so that the pacer pads (see `BweTestContext::negotiated`).
    let mut ctx = BweTestContext::negotiated(&mut l, &mut r);
    ctx.set_media_send_rate(LOW_LAYER);

    set_link(&mut l, &mut r, LINK);
    ctx.run_for_duration(&mut l, &mut r, Duration::from_secs(3))?;
    set_link(&mut l, &mut r, DIP);
    ctx.run_for_duration(&mut l, &mut r, Duration::from_secs(10))?;

    set_link(&mut l, &mut r, LINK);
    let restored = l.duration();
    let restored_at = l.last;
    let restored_offset = l.events.len();
    ctx.run_for_duration(&mut l, &mut r, Duration::from_millis(300))?;

    let estimate_at_action = get_last_bwe_estimate(&l).expect("an estimate");
    assert!(
        estimate_at_action < Bitrate::mbps(1),
        "the dip must leave the estimate low: {estimate_at_action}"
    );
    let offset = l.events.len();
    action(&mut l, estimate_at_action);

    let mut elapsed = None;
    while l.duration() - restored < Duration::from_secs(20) {
        ctx.run_for_duration(&mut l, &mut r, Duration::from_millis(100))?;
        let since = l.duration() - restored;
        if elapsed.is_none() && get_last_bwe_estimate(&l).is_some_and(|e| e >= RECOVERED) {
            elapsed = Some(since);
        }
        // Keep running a while after recovering, so the events that follow can be checked.
        if elapsed.is_some() && since >= Duration::from_secs(4) {
            break;
        }
    }

    let probes = l.events[offset..]
        .iter()
        .filter_map(|(_, e)| match e {
            Event::Probe(p) => Some(p.target_bitrate()),
            _ => None,
        })
        .collect();

    let estimates = l.events[restored_offset..]
        .iter()
        .filter_map(|(at, e)| {
            let since = at.saturating_duration_since(restored_at);
            match e {
                Event::EgressBitrateEstimate(BweKind::Twcc { estimate, .. }) => {
                    Some((since, Estimate::Twcc(*estimate)))
                }
                Event::EgressBitrateEstimate(BweKind::ProbeResult { estimate }) => {
                    Some((since, Estimate::ProbeResult(*estimate)))
                }
                _ => None,
            }
        })
        .collect();

    Ok(Recovery {
        elapsed,
        estimate_at_action,
        probes,
        estimates,
    })
}

#[test]
fn requested_probe_recovers_the_estimate_sooner() -> Result<(), RtcError> {
    init_log();
    init_crypto_default();

    let without = recover(|_, _| {})?;
    let with = recover(|l, estimate| l.bwe().request_probe(estimate * 2.0))?;
    info!(
        "without {:?}, with {:?} (probes {:?})",
        without.elapsed, with.elapsed, with.probes
    );

    let without = without.elapsed.expect("str0m recovers on its own, later");
    let requested = with.estimate_at_action * 2.0;
    assert_eq!(
        with.probes.first(),
        Some(&requested),
        "the requested probe is sent first"
    );
    let with = with.elapsed.expect("the request recovers the estimate");
    assert!(
        with + Duration::from_secs(1) <= without,
        "a request recovers at least 1 s sooner: {with:?} vs {without:?}"
    );
    Ok(())
}

#[test]
fn padding_target_recovers_the_estimate_sooner() -> Result<(), RtcError> {
    init_log();
    init_crypto_default();

    let without = recover(|_, _| {})?;
    let with = recover(|l, _| l.bwe().set_padding_target(Some(RECOVERED)))?;
    info!("without {:?}, with {:?}", without.elapsed, with.elapsed);

    let without = without.elapsed.expect("str0m recovers on its own, later");
    let with = with.elapsed.expect("padding recovers the estimate");
    assert!(
        with + Duration::from_secs(1) <= without,
        "padding recovers at least 1 s sooner: {with:?} vs {without:?}"
    );
    Ok(())
}

#[test]
fn requested_probe_emits_probe_result_and_lifts_twcc() -> Result<(), RtcError> {
    init_log();
    init_crypto_default();

    let run = recover(|l, estimate| l.bwe().request_probe(estimate * 2.0))?;
    println!("{:?}", run.estimates);

    let (at, result) = run
        .estimates
        .iter()
        .find_map(|(at, e)| match e {
            Estimate::ProbeResult(b) => Some((*at, *b)),
            _ => None,
        })
        .expect("a ProbeResult after the request");
    assert!(
        result > run.estimate_at_action,
        "a probe result above the estimate at the request: {result} vs {}",
        run.estimate_at_action
    );

    // The smoother starts again from the probe result instead of averaging the dip back in.
    let next_twcc = run
        .estimates
        .iter()
        .filter(|(t, _)| *t >= at)
        .find_map(|(_, e)| match e {
            Estimate::Twcc(b) => Some(*b),
            _ => None,
        })
        .expect("a Twcc estimate after the probe result");
    assert!(
        next_twcc >= result * 0.9,
        "the next Twcc estimate continues from the probe result: {next_twcc} vs {result}"
    );
    Ok(())
}

#[test]
fn no_probe_result_without_a_probe() -> Result<(), RtcError> {
    init_log();
    init_crypto_default();

    let run = recover(|_, _| {})?;
    let early: Vec<_> = run
        .estimates
        .iter()
        .filter(|(t, e)| *t < Duration::from_secs(1) && matches!(e, Estimate::ProbeResult(_)))
        .collect();
    assert!(early.is_empty(), "probe results with no probe: {early:?}");
    Ok(())
}
