//! Flash-persisted OpenThread settings.
//!
//! OpenThread stores small settings blobs (network info, parent info, and - crucially -
//! the **SRP ECDSA key**) via its `Settings` trait. The default `SimpleRamSettings` keeps
//! them in RAM, so on every reboot the device generates a *new* SRP key. The Thread
//! border router still reserves the device's SRP hostname for the *old* key (SRP key-lease
//! defaults to 14 days), so re-registration is rejected as `OT_ERROR_DUPLICATED` and the
//! device becomes undiscoverable (HomeKit can't reach it) until the lease expires.
//!
//! [`FlashSettings`] fixes that: it keeps settings in a [`SimpleRamSettings`] (which
//! implements the fiddly multimap semantics) and **writes the whole set through to flash**
//! on every change, restoring it on boot. Persisting the SRP key means the device refreshes
//! its *own* existing registration after a reboot instead of colliding with it.
//!
//! The `Settings` trait is synchronous and our flash ops are synchronous (block_on over an
//! immediately-completing future), so a plain write-through works - no background task.

use embassy_time::{Duration, Instant};

use openthread::{Settings, SettingsError, SimpleRamSettings};

use static_cell::StaticCell;

use super::kv::SettingsFlash;

/// How long to defer a persisted-key flash write after the change. The SRP ECDSA key
/// (the only persisted key) is generated *during* SRP registration, and an esp-storage
/// erase blocks interrupts ~15 ms - long enough on the single shared 2.4 GHz radio to
/// drop the SRP UPDATE and leave the device undiscoverable. Deferring the write past the
/// registration + first-CASE window lets it land in a later radio lull instead. It is
/// flushed lazily on the next settings access (OpenThread writes transient keys 3/4/5
/// constantly during operation, so the flush fires well within a few seconds).
const FLASH_DEFER: Duration = Duration::from_secs(10);

/// Flash key (within the OpenThread settings region) holding the serialized blob.
const BLOB_KEY: u16 = 1;

/// OpenThread setting keys we persist across reboots. Deliberately ONLY the SRP client
/// ECDSA key (`OT_SETTINGS_KEY_SRP_ECDSA_KEY = 11`): it is written once and is the sole
/// thing needed to avoid the `OT_ERROR_DUPLICATED` SRP conflict on reboot (same key -> the
/// border router accepts refreshing our own registration).
///
/// Everything else is intentionally NOT persisted: the dataset is re-applied by Matter on
/// reboot, and the transient keys - NetworkInfo(3), ParentInfo(4), ChildInfo(5) - are
/// rewritten *constantly* during a Thread attach. Persisting those made every attach
/// trigger a ~15 ms interrupts-off flash erase, starving the radio (802.15.4
/// `ChannelAccessFailure`) and breaking operational CASE. Restricting persistence to the
/// write-once SRP key keeps flash writes rare and off the hot path.
const PERSIST_KEYS: &[u16] = &[11];

/// RAM cache for the live settings (same 1 KB as the previous SimpleRamSettings buffer).
static RAM_BUF: StaticCell<[u8; 1024]> = StaticCell::new();
/// Scratch for (de)serializing the settings set to/from the single flash blob.
static SCRATCH: StaticCell<[u8; 1024]> = StaticCell::new();

pub struct FlashSettings {
    ram: SimpleRamSettings<'static>,
    flash: SettingsFlash,
    scratch: &'static mut [u8; 1024],
    /// A persisted key changed but hasn't been written to flash yet (see [`FLASH_DEFER`]).
    /// `Some(t)` records when it went dirty; the flush fires `FLASH_DEFER` after `t`.
    dirty_since: Option<Instant>,
}

impl FlashSettings {
    /// Build the settings store, restoring any previously persisted settings from flash.
    /// Must be constructed before `OpenThread::new` so the restored settings are visible.
    pub fn new() -> Result<Self, SettingsError> {
        let ram_buf = RAM_BUF.init([0u8; 1024]);
        let scratch = SCRATCH.init([0u8; 1024]);
        let mut flash = SettingsFlash::new().map_err(|_| SettingsError::NotImplemented)?;
        let mut ram = SimpleRamSettings::new(ram_buf);

        // Restore: read the serialized blob and replay each entry into the RAM cache.
        // Format per entry: [key: u16 LE][len: u16 LE][value bytes].
        match flash.load(BLOB_KEY, scratch) {
            Ok(Some(len)) => {
                let mut b = &scratch[..len];
                let mut restored = 0;
                while b.len() >= 4 {
                    let key = u16::from_le_bytes([b[0], b[1]]);
                    let vlen = u16::from_le_bytes([b[2], b[3]]) as usize;
                    b = &b[4..];
                    if b.len() < vlen {
                        break;
                    }
                    // `add` (not the trait method) so we don't re-persist during restore.
                    let _ = ram.add(key, &b[..vlen]);
                    b = &b[vlen..];
                    restored += 1;
                }
                log::info!("[matter] OT settings: restored {restored} entries from flash");
            }
            Ok(None) => log::info!("[matter] OT settings: none saved (fresh)"),
            Err(_) => log::warn!("[matter] OT settings: load failed - starting fresh"),
        }

        Ok(Self {
            ram,
            flash,
            scratch,
            dirty_since: None,
        })
    }

    /// Mark the persisted set as changed, to be written after [`FLASH_DEFER`]. Records the
    /// time only on the 0->dirty transition so the flush fires relative to the *first*
    /// pending change (which, for the write-once SRP key, is the change itself).
    fn mark_dirty(&mut self) {
        if self.dirty_since.is_none() {
            self.dirty_since = Some(Instant::now());
        }
    }

    /// Flush the deferred write if its defer window has elapsed. Called at the top of every
    /// settings op (OpenThread accesses settings frequently), so the pending SRP-key write
    /// lands in a radio lull a few seconds after commissioning rather than mid-registration.
    fn flush_if_due(&mut self) {
        if let Some(t) = self.dirty_since {
            if t.elapsed() >= FLASH_DEFER {
                self.persist();
                self.dirty_since = None;
            }
        }
    }

    /// Serialize the persisted (whitelisted) settings and write them to flash as one blob.
    fn persist(&mut self) {
        let mut n = 0;
        for (key, value) in self.ram.iter() {
            if !PERSIST_KEYS.contains(&key) {
                continue;
            }
            if n + 4 + value.len() > self.scratch.len() {
                log::warn!("[matter] OT settings too large to persist - truncating");
                break;
            }
            self.scratch[n..n + 2].copy_from_slice(&key.to_le_bytes());
            self.scratch[n + 2..n + 4].copy_from_slice(&(value.len() as u16).to_le_bytes());
            self.scratch[n + 4..n + 4 + value.len()].copy_from_slice(value);
            n += 4 + value.len();
        }
        if let Err(e) = self.flash.store(BLOB_KEY, &self.scratch[..n]) {
            log::warn!("[matter] OT settings persist failed: {e:?}");
        } else {
            log::info!("[matter] OT settings persisted ({n} bytes)");
        }
    }
}

impl Settings for FlashSettings {
    fn init(&mut self, sensitive_keys: &[u16]) {
        self.ram.init(sensitive_keys);
    }

    fn deinit(&mut self) {
        self.ram.deinit();
    }

    fn get(
        &mut self,
        key: u16,
        index: usize,
        buf: &mut [u8],
    ) -> Result<Option<usize>, SettingsError> {
        self.flush_if_due();
        self.ram.get(key, index, buf)
    }

    fn set(&mut self, key: u16, value: &[u8]) -> Result<(), SettingsError> {
        self.flush_if_due();
        let r = self.ram.set(key, value);
        // Only persist when a persisted key changes - keeps the frequent transient writes
        // (network/parent/child info during attach) off the flash/radio path entirely.
        // Deferred (not written now): the SRP key changes *during* SRP registration, and a
        // synchronous erase there starves the radio and kills the registration.
        if PERSIST_KEYS.contains(&key) {
            self.mark_dirty();
        }
        r
    }

    fn add(&mut self, key: u16, value: &[u8]) -> Result<(), SettingsError> {
        self.flush_if_due();
        let r = self.ram.add(key, value);
        if PERSIST_KEYS.contains(&key) {
            self.mark_dirty();
        }
        r
    }

    fn remove(&mut self, key: u16, index: Option<usize>) -> Result<bool, SettingsError> {
        self.flush_if_due();
        let r = self.ram.remove(key, index);
        if PERSIST_KEYS.contains(&key) {
            self.mark_dirty();
        }
        r
    }

    fn clear(&mut self) -> Result<(), SettingsError> {
        self.ram.clear();
        // A full clear must also wipe the persisted blob. Deferred like the others so it
        // never lands on the radio hot path; flushed on the next settings access.
        self.mark_dirty();
        Ok(())
    }
}
