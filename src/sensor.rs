//! Contactless EMF power-presence sensor (esp-hal ADC backend).
//!
//! Hardware: an LM358 op-amp configured as a high-gain, band-limited inverting AC
//! amplifier, mounted at the antenna and biased to a ~1.65 V virtual ground. Its
//! low-impedance output is buffered over a multi-foot shielded cable into GPIO4. A
//! 2.2 nF feedback cap band-limits the stage to ~50 Hz. See `220AC_EMF_Remote_detector.md`.
//!
//! GPIO4 is read as a **calibrated analog ADC1 input** (channel 3). Calibration
//! (`AdcCalLine`) corrects the H2's large raw ADC offset/gain error and returns
//! **millivolts**, so the bias sits mid-range with full headroom instead of clipping
//! against the ADC ceiling. Power presence is inferred from the **peak-to-peak swing**
//! of the 50 Hz signal over a sampling window (~200 mV pp live, < 100 mV ambient).

use embassy_time::{Duration, Timer};
use esp_hal::analog::adc::{Adc, AdcCalLine, AdcConfig, AdcPin, Attenuation};
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

/// GPIO4 / ADC1_CH3 with line-fit (efuse) calibration. The calibrated read returns
/// **millivolts**, not raw counts - see [`EmfSensor::new`].
type EmfAdcPin = AdcPin<GPIO4<'static>, ADC1<'static>, AdcCalLine<ADC1<'static>>>;

/// ADC-backed EMF detector. Reads the LM358 amplifier output (GPIO4 / ADC1_CH3) as an
/// analog signal and infers AC presence from the peak-to-peak swing.
pub struct EmfSensor {
    adc: Adc<'static, ADC1<'static>, Blocking>,
    pin: EmfAdcPin,
}

impl EmfSensor {
    /// Configure GPIO4 as a **calibrated** ADC1 input.
    ///
    /// 11 dB attenuation (~ 0-3.3 V range). We use `enable_pin_with_cal` with the
    /// line-fit (`AdcCalLine`) scheme rather than a raw `enable_pin`: the ESP32-H2's
    /// uncalibrated ADC has a large built-in offset/gain error (it read ~3730 for a
    /// 1.65 V input, jamming the operating point against the 4095 ceiling and clipping
    /// the AC swing). Calibration applies the efuse offset/gain correction and makes
    /// `read_oneshot` return **millivolts**, so a 1.65 V bias reads ~1650 mV mid-range
    /// with full headroom for the 50 Hz swing both ways.
    pub fn new(adc1: ADC1<'static>, gpio4: GPIO4<'static>) -> Self {
        let mut config = AdcConfig::new();
        let pin = config.enable_pin_with_cal::<GPIO4<'static>, AdcCalLine<ADC1<'static>>>(
            gpio4,
            Attenuation::_11dB,
        );
        let adc = Adc::new(adc1, config);
        Self { adc, pin }
    }

    /// Take one detection window of samples and classify power presence.
    pub async fn sample(&mut self) -> PowerState {
        let mut buf = [0u16; EMF_SAMPLE_COUNT];
        for slot in buf.iter_mut() {
            // The one-shot conversion finishes in a few us; `nb::block!` busy-waits
            // only for that, then we yield for the inter-sample spacing. The read is
            // infallible in practice (error type is `()`); treat a spurious error as 0.
            *slot = nb::block!(self.adc.read_oneshot(&mut self.pin)).unwrap_or(0);
            Timer::after(Duration::from_micros(EMF_SAMPLE_INTERVAL_US)).await;
        }
        let pp = peak_to_peak(&buf);
        let state = detect_power_from_samples(&buf, EMF_DETECTION_THRESHOLD);
        let min = buf.iter().copied().min().unwrap_or(0);
        let max = buf.iter().copied().max().unwrap_or(0);
        let mean = (buf.iter().map(|&s| s as u32).sum::<u32>() / buf.len() as u32) as u16;
        // All values are in millivolts (calibrated read).
        debug!(
            "EMF sensor: pp = {} mV (threshold {} mV), min = {} mV, max = {} mV, mean = {} mV, state = {:?}",
            pp, EMF_DETECTION_THRESHOLD, min, max, mean, state
        );
        state
    }
}

// --- Pure detection logic - no HAL deps --------------------------------------

/// Returns the peak-to-peak swing (max - min) of `samples`.
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
