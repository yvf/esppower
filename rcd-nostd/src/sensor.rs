//! Contactless EMF power-presence sensor (esp-hal ADC backend).
//!
//! Hardware: a 3-stage BC547 NPN transistor cascade (high-gain amplifier) with a
//! short spiral copper antenna on the base of Q1 and a 1–10 MΩ bleeder to ground.
//! The amplified output is fed through a 10 kΩ series resistor into GPIO4.
//!
//! GPIO4 is read as an **analog ADC1 input** (channel 3), NOT a digital pin. The
//! cascade output is an emitter-follower chain clamped by the final transistor's
//! base-emitter junction to ≈ 0.8 V, so it never crosses the logic-HIGH threshold —
//! a digital read could never see a transition. Power presence is therefore inferred
//! from the **peak-to-peak swing** of the 50 Hz signal over a sampling window
//! (≈750 mV pp when live, ≈300 mV pp ambient). See `220AC_CT_detector.md`.

use embassy_time::{Duration, Timer};
use esp_hal::analog::adc::{Adc, AdcConfig, AdcPin, Attenuation};
use esp_hal::peripherals::{ADC1, GPIO4};
use esp_hal::Blocking;
use log::debug;

use crate::config::{EMF_DETECTION_THRESHOLD, EMF_SAMPLE_COUNT, EMF_SAMPLE_INTERVAL_US};

/// Detected state of the monitored mains circuit.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PowerState {
    Present,
    Absent,
}

impl PowerState {
    pub fn is_present(self) -> bool {
        matches!(self, PowerState::Present)
    }
}

/// ADC-backed EMF detector. Reads the BC547 cascade output (GPIO4 / ADC1_CH3) as an
/// analog signal and infers AC presence from the peak-to-peak swing.
pub struct EmfSensor {
    adc: Adc<'static, ADC1<'static>, Blocking>,
    pin: AdcPin<GPIO4<'static>, ADC1<'static>>,
}

impl EmfSensor {
    /// Configure GPIO4 as an ADC1 input.
    ///
    /// 11 dB attenuation (≈ 0–3.9 V full scale). The cascade output never exceeds
    /// ~0.8 V, so this leaves plenty of headroom; lower the attenuation later for
    /// more resolution if needed.
    pub fn new(adc1: ADC1<'static>, gpio4: GPIO4<'static>) -> Self {
        let mut config = AdcConfig::new();
        let pin = config.enable_pin(gpio4, Attenuation::_11dB);
        let adc = Adc::new(adc1, config);
        Self { adc, pin }
    }

    /// Take one detection window of samples and classify power presence.
    pub async fn sample(&mut self) -> PowerState {
        let mut buf = [0u16; EMF_SAMPLE_COUNT];
        for slot in buf.iter_mut() {
            // The one-shot conversion finishes in a few µs; `nb::block!` busy-waits
            // only for that, then we yield for the inter-sample spacing. The read is
            // infallible in practice (error type is `()`); treat a spurious error as 0.
            *slot = nb::block!(self.adc.read_oneshot(&mut self.pin)).unwrap_or(0);
            Timer::after(Duration::from_micros(EMF_SAMPLE_INTERVAL_US)).await;
        }
        let pp = peak_to_peak(&buf);
        let state = detect_power_from_samples(&buf, EMF_DETECTION_THRESHOLD);
        debug!(
            "EMF sensor: peak-to-peak = {} (threshold {}), state = {:?}",
            pp, EMF_DETECTION_THRESHOLD, state
        );
        state
    }
}

// ─── Pure detection logic — no HAL deps ──────────────────────────────────────

/// Returns the peak-to-peak swing (max − min) of `samples`.
pub fn peak_to_peak(samples: &[u16]) -> u16 {
    let max = samples.iter().copied().max().unwrap_or(0);
    let min = samples.iter().copied().min().unwrap_or(0);
    max.saturating_sub(min)
}

/// `Present` when the peak-to-peak swing exceeds `threshold`, else `Absent`.
pub fn detect_power_from_samples(samples: &[u16], threshold: u16) -> PowerState {
    if peak_to_peak(samples) > threshold {
        PowerState::Present
    } else {
        PowerState::Absent
    }
}
