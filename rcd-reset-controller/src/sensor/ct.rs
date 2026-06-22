//! CT current-transformer power presence sensor (feature = "sensor-ct").
//!
//! Hardware: SCT-013-000 (100 A / 1 V output) → RBDimmer SCT-013 Adapter → GPIO1.
//!
//! The adapter centres the AC waveform at VCC/2 (≈ 1.65 V) and outputs 0–3.3 V.
//! With 11 dB ADC attenuation the full-scale input is ≈ 3.9 V → 12-bit = 4095.
//!
//! Detection algorithm:
//!   Take N samples spaced 100 µs apart (200 samples = 20 ms = one 50 Hz cycle).
//!   Compute peak-to-peak (max − min). A swing above `CT_DETECTION_THRESHOLD`
//!   indicates AC current is flowing (mains present downstream of RCD).
//!
//! Assumption: the circuit protected by the RCD has at least a small permanent load
//! so that current flows whenever the RCD is closed and mains power is present.

use super::PowerState;
use crate::config::{CT_DETECTION_THRESHOLD, CT_SAMPLE_COUNT, CT_SAMPLE_INTERVAL_US};
use crate::controller::SensorRead;
use embassy_time::{Duration, Timer};
use esp_idf_hal::adc::{
    attenuation,
    oneshot::{config::AdcChannelConfig, AdcChannelDriver, AdcDriver},
    Adc, AdcChannel,
};
use esp_idf_hal::gpio::ADCPin;
use log::debug;

/// Concrete sensor type for ESP32-H2 GPIO1 (ADC1 channel 0).
pub type Gpio1Sensor<'d> =
    PowerSensor<'d, esp_idf_hal::adc::ADCCH0<esp_idf_hal::adc::ADCU1>>;

pub struct PowerSensor<'d, C: AdcChannel> {
    channel: AdcChannelDriver<'d, C, AdcDriver<'d, C::AdcUnit>>,
}

impl<'d, C: AdcChannel> PowerSensor<'d, C> {
    pub fn new<A, PIN>(adc: A, pin: PIN) -> anyhow::Result<Self>
    where
        A: Adc<AdcUnit = C::AdcUnit> + 'd,
        PIN: ADCPin<AdcChannel = C> + 'd,
    {
        let adc_driver = AdcDriver::new(adc)
            .map_err(|e| anyhow::anyhow!("ADC init: {e}"))?;
        let config = AdcChannelConfig {
            attenuation: attenuation::DB_12,
            ..Default::default()
        };
        let channel = AdcChannelDriver::new(adc_driver, pin, &config)
            .map_err(|e| anyhow::anyhow!("ADC channel init: {e}"))?;
        Ok(Self { channel })
    }
}

// ─── Blocking method (used by integration test examples) ─────────────────────

impl<'d, C: AdcChannel> PowerSensor<'d, C> {
    pub fn sample_blocking(&mut self) -> anyhow::Result<PowerState> {
        let mut buf = [0u16; CT_SAMPLE_COUNT];
        for slot in buf.iter_mut() {
            *slot = self
                .channel
                .read_raw()
                .map_err(|e| anyhow::anyhow!("ADC read: {e}"))?;
            std::thread::sleep(std::time::Duration::from_micros(CT_SAMPLE_INTERVAL_US));
        }
        let pp = peak_to_peak(&buf);
        let state = detect_power_from_samples(&buf, CT_DETECTION_THRESHOLD);
        debug!(
            "Sensor: peak-to-peak = {} (threshold {}), state = {:?}",
            pp, CT_DETECTION_THRESHOLD, state
        );
        Ok(state)
    }
}

// ─── SensorRead trait implementation ─────────────────────────────────────────

impl<'d, C: AdcChannel> SensorRead for PowerSensor<'d, C> {
    async fn sample(&mut self) -> anyhow::Result<PowerState> {
        let mut buf = [0u16; CT_SAMPLE_COUNT];
        for slot in buf.iter_mut() {
            *slot = self
                .channel
                .read_raw()
                .map_err(|e| anyhow::anyhow!("ADC read: {e}"))?;
            Timer::after(Duration::from_micros(CT_SAMPLE_INTERVAL_US)).await;
        }
        let pp = peak_to_peak(&buf);
        let state = detect_power_from_samples(&buf, CT_DETECTION_THRESHOLD);
        debug!(
            "Sensor: peak-to-peak = {} (threshold {}), state = {:?}",
            pp, CT_DETECTION_THRESHOLD, state
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

    fn sine_like_signal(amplitude: u16, offset: u16, n: usize) -> Vec<u16> {
        let half = n / 2;
        (0..n)
            .map(|i| {
                if i < half {
                    offset + (amplitude * i as u16) / half as u16
                } else {
                    offset + (amplitude * (n - i) as u16) / half as u16
                }
            })
            .collect()
    }

    #[test]
    fn flat_signal_reads_absent() {
        let samples = flat_signal(2048, CT_SAMPLE_COUNT);
        assert_eq!(
            detect_power_from_samples(&samples, CT_DETECTION_THRESHOLD),
            PowerState::Absent
        );
    }

    #[test]
    fn large_swing_reads_present() {
        let samples = sine_like_signal(500, 1548, CT_SAMPLE_COUNT);
        assert_eq!(
            detect_power_from_samples(&samples, CT_DETECTION_THRESHOLD),
            PowerState::Present
        );
    }

    #[test]
    fn swing_exactly_at_threshold_reads_absent() {
        // Peak-to-peak must be strictly GREATER than threshold.
        let mut samples = flat_signal(2048, CT_SAMPLE_COUNT);
        samples[0] = 2048 - CT_DETECTION_THRESHOLD / 2;
        samples[1] = 2048 + CT_DETECTION_THRESHOLD / 2;
        let pp = peak_to_peak(&samples);
        if pp <= CT_DETECTION_THRESHOLD {
            assert_eq!(
                detect_power_from_samples(&samples, CT_DETECTION_THRESHOLD),
                PowerState::Absent
            );
        }
    }

    #[test]
    fn swing_one_above_threshold_reads_present() {
        let swing = CT_DETECTION_THRESHOLD + 1;
        let mut samples = flat_signal(2048, CT_SAMPLE_COUNT);
        samples[0] = 2048;
        samples[1] = 2048 + swing;
        assert_eq!(
            detect_power_from_samples(&samples, CT_DETECTION_THRESHOLD),
            PowerState::Present
        );
    }

    #[test]
    fn empty_samples_return_absent() {
        assert_eq!(
            detect_power_from_samples(&[], CT_DETECTION_THRESHOLD),
            PowerState::Absent
        );
    }

    #[test]
    fn peak_to_peak_of_constant_signal_is_zero() {
        assert_eq!(peak_to_peak(&flat_signal(1000, 50)), 0);
    }

    #[test]
    fn peak_to_peak_calculates_correctly() {
        let samples = vec![100u16, 500, 200, 50, 400];
        assert_eq!(peak_to_peak(&samples), 450); // 500 - 50
    }

    #[test]
    fn peak_to_peak_does_not_overflow_on_max_values() {
        assert_eq!(peak_to_peak(&[0u16, 4095]), 4095);
    }
}
