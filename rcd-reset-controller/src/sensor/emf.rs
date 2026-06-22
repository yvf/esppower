//! Contactless EMF power-presence sensor (feature = "sensor-emf").
//!
//! Hardware: a 3-stage BC547 NPN transistor cascade (high-gain Darlington-style
//! amplifier) with a short spiral copper antenna on the base of Q1 and a 1–10 MΩ
//! bleeder to ground. The amplified output of the cascade is fed through a 10 kΩ
//! series resistor into GPIO4.
//!
//! GPIO4 is read as an **analog ADC1 input** (channel 3), NOT a digital pin. The
//! cascade output is an emitter-follower chain whose output node is clamped by the
//! final transistor's base-emitter junction to ≈ 0.8 V, so it never crosses the
//! logic-HIGH threshold (~2.5 V) — a digital read can never see a transition.
//! Verified on a scope (see `220AC_EMF_detector.md`):
//!   - field present : ~750 mV peak-to-peak 50 Hz swing at the output node
//!   - field absent   : ~300 mV peak-to-peak (ambient noise floor)
//! Both sit entirely below 0.8 V. So power presence is inferred from the **analog
//! peak-to-peak swing** of the 50 Hz signal, exactly like the CT backend.
//!
//! Detection algorithm:
//!   Take `EMF_SAMPLE_COUNT` ADC samples spaced `EMF_SAMPLE_INTERVAL_US` apart
//!   (400 × 100 µs = 40 ms ≈ two full 50 Hz cycles). Compute peak-to-peak
//!   (max − min). A swing above `EMF_DETECTION_THRESHOLD` ⇒ AC present.
//!
//! Unlike the CT backend this is purely contactless: no galvanic or magnetic-core
//! connection to the monitored circuit, and no permanent load is required.

use super::PowerState;
use crate::config::{EMF_DETECTION_THRESHOLD, EMF_SAMPLE_COUNT, EMF_SAMPLE_INTERVAL_US};
use crate::controller::SensorRead;
use embassy_time::{Duration, Timer};
use esp_idf_hal::adc::{
    attenuation,
    oneshot::{config::AdcChannelConfig, AdcChannelDriver, AdcDriver},
    Adc, AdcChannel,
};
use esp_idf_hal::gpio::ADCPin;
use log::debug;

/// Concrete EMF sensor type for ESP32-H2 GPIO4 (ADC1 channel 3).
pub type EmfSensor<'d> =
    EmfAdcSensor<'d, esp_idf_hal::adc::ADCCH3<esp_idf_hal::adc::ADCU1>>;

/// ADC-backed EMF detector. Reads the BC547 cascade output as an analog signal.
pub struct EmfAdcSensor<'d, C: AdcChannel> {
    channel: AdcChannelDriver<'d, C, AdcDriver<'d, C::AdcUnit>>,
}

impl<'d, C: AdcChannel> EmfAdcSensor<'d, C> {
    /// Configure GPIO4 as an ADC1 input.
    ///
    /// 12 dB attenuation (≈ 0–3.9 V full scale). The cascade output never exceeds
    /// ~0.8 V, so this leaves plenty of headroom; lower the attenuation later for
    /// more resolution if needed.
    pub fn new<A, PIN>(adc: A, pin: PIN) -> anyhow::Result<Self>
    where
        A: Adc<AdcUnit = C::AdcUnit> + 'd,
        PIN: ADCPin<AdcChannel = C> + 'd,
    {
        let adc_driver =
            AdcDriver::new(adc).map_err(|e| anyhow::anyhow!("EMF ADC init: {e}"))?;
        let config = AdcChannelConfig {
            attenuation: attenuation::DB_12,
            ..Default::default()
        };
        let channel = AdcChannelDriver::new(adc_driver, pin, &config)
            .map_err(|e| anyhow::anyhow!("EMF ADC channel init: {e}"))?;
        Ok(Self { channel })
    }
}

// ─── Blocking method (used by integration test examples) ─────────────────────

impl<'d, C: AdcChannel> EmfAdcSensor<'d, C> {
    pub fn sample_blocking(&mut self) -> anyhow::Result<PowerState> {
        let mut buf = [0u16; EMF_SAMPLE_COUNT];
        for slot in buf.iter_mut() {
            *slot = self
                .channel
                .read_raw()
                .map_err(|e| anyhow::anyhow!("EMF ADC read: {e}"))?;
            std::thread::sleep(std::time::Duration::from_micros(EMF_SAMPLE_INTERVAL_US));
        }
        let pp = peak_to_peak(&buf);
        let state = detect_power_from_samples(&buf, EMF_DETECTION_THRESHOLD);
        debug!(
            "EMF sensor: peak-to-peak = {} (threshold {}), state = {:?}",
            pp, EMF_DETECTION_THRESHOLD, state
        );
        Ok(state)
    }
}

// ─── SensorRead trait implementation ─────────────────────────────────────────

impl<'d, C: AdcChannel> SensorRead for EmfAdcSensor<'d, C> {
    async fn sample(&mut self) -> anyhow::Result<PowerState> {
        let mut buf = [0u16; EMF_SAMPLE_COUNT];
        for slot in buf.iter_mut() {
            *slot = self
                .channel
                .read_raw()
                .map_err(|e| anyhow::anyhow!("EMF ADC read: {e}"))?;
            Timer::after(Duration::from_micros(EMF_SAMPLE_INTERVAL_US)).await;
        }
        let pp = peak_to_peak(&buf);
        let state = detect_power_from_samples(&buf, EMF_DETECTION_THRESHOLD);
        debug!(
            "EMF sensor: peak-to-peak = {} (threshold {}), state = {:?}",
            pp, EMF_DETECTION_THRESHOLD, state
        );
        Ok(state)
    }
}

// ─── Pure detection logic — no HAL deps, fully unit-testable ─────────────────

/// Returns the peak-to-peak swing of `samples`.
pub fn peak_to_peak(samples: &[u16]) -> u16 {
    if samples.is_empty() {
        return 0;
    }
    let max = samples.iter().copied().max().unwrap_or(0);
    let min = samples.iter().copied().min().unwrap_or(0);
    max.saturating_sub(min)
}

/// Returns `PowerState::Present` when the peak-to-peak swing exceeds `threshold`.
pub fn detect_power_from_samples(samples: &[u16], threshold: u16) -> PowerState {
    if peak_to_peak(samples) > threshold {
        PowerState::Present
    } else {
        PowerState::Absent
    }
}

// ─── Unit tests (host-runnable) ───────────────────────────────────────────────
#[cfg(test)]
mod tests {
    use super::*;

    fn flat_signal(value: u16, n: usize) -> Vec<u16> {
        vec![value; n]
    }

    /// A 50 Hz-like triangle wave centred on `offset`, rising to `offset +
    /// amplitude` at the midpoint and back (peak-to-peak ≈ `amplitude`).
    /// Uses u32 intermediates so `amplitude * i` can't overflow for large `n`.
    fn triangle(amplitude: u16, offset: u16, n: usize) -> Vec<u16> {
        let half = (n / 2) as u32;
        let (amp, off) = (amplitude as u32, offset as u32);
        (0..n)
            .map(|i| {
                let i = i as u32;
                let v = if i < half {
                    off + amp * i / half
                } else {
                    off + amp * (n as u32 - i) / half
                };
                v as u16
            })
            .collect()
    }

    #[test]
    fn peak_to_peak_of_constant_signal_is_zero() {
        assert_eq!(peak_to_peak(&flat_signal(400, EMF_SAMPLE_COUNT)), 0);
    }

    #[test]
    fn peak_to_peak_calculates_correctly() {
        assert_eq!(peak_to_peak(&[100u16, 500, 200, 50, 400]), 450); // 500 - 50
    }

    #[test]
    fn peak_to_peak_does_not_overflow_on_max_values() {
        assert_eq!(peak_to_peak(&[0u16, 4095]), 4095);
    }

    #[test]
    fn empty_window_reads_absent() {
        assert_eq!(peak_to_peak(&[]), 0);
        assert_eq!(
            detect_power_from_samples(&[], EMF_DETECTION_THRESHOLD),
            PowerState::Absent
        );
    }

    #[test]
    fn quiet_noise_floor_reads_absent() {
        // ~300 mV ambient noise floor (≈ 315 ADC counts) must read ABSENT.
        let samples = triangle(315, 100, EMF_SAMPLE_COUNT);
        assert!(peak_to_peak(&samples) < EMF_DETECTION_THRESHOLD);
        assert_eq!(
            detect_power_from_samples(&samples, EMF_DETECTION_THRESHOLD),
            PowerState::Absent
        );
    }

    #[test]
    fn live_field_reads_present() {
        // ~750 mV field swing (≈ 790 ADC counts) must read PRESENT.
        let samples = triangle(790, 50, EMF_SAMPLE_COUNT);
        assert!(peak_to_peak(&samples) > EMF_DETECTION_THRESHOLD);
        assert_eq!(
            detect_power_from_samples(&samples, EMF_DETECTION_THRESHOLD),
            PowerState::Present
        );
    }

    #[test]
    fn swing_one_above_threshold_reads_present() {
        let swing = EMF_DETECTION_THRESHOLD + 1;
        let samples = [0u16, swing];
        assert_eq!(
            detect_power_from_samples(&samples, EMF_DETECTION_THRESHOLD),
            PowerState::Present
        );
    }
}
