//! Flash-backed `KvBlobStore` for Matter persistence.
//!
//! Persists the Matter commissioning state (fabrics/NOC/ACLs) and the Thread networks
//! store (the operational dataset) into the `nvs` flash partition, so a commissioned
//! device survives reboots: on the next boot `is_commissioned()` is true and the
//! non-concurrent `run()` skips BLE and goes straight to Thread (see stack.rs).
//!
//! Layers:
//! - [`esp_storage::FlashStorage`] - raw NOR flash (blocking `embedded-storage`).
//! - [`esp_bootloader_esp_idf::partitions`] - locates the `nvs` partition at runtime
//!   (no hardcoded offsets), giving the flash range to operate within.
//! - [`sequential_storage::map`] - a wear-levelled keyed-blob map over that range. It is
//!   async-only, and `FlashStorage` is blocking, so [`AsyncFlash`] bridges the two and we
//!   drive it with `block_on` (the flash ops are CPU-blocking and never truly await).

use core::ops::Range;

use embassy_futures::block_on;
use embedded_storage::nor_flash::{
    ErrorType, NorFlash as BlockingNorFlash, ReadNorFlash as BlockingReadNorFlash,
};
use embedded_storage_async::nor_flash::{
    MultiwriteNorFlash as AsyncMultiwrite, NorFlash as AsyncNorFlash,
    ReadNorFlash as AsyncReadNorFlash,
};

use esp_bootloader_esp_idf::partitions::{
    read_partition_table, DataPartitionSubType, PartitionType, PARTITION_TABLE_MAX_LEN,
};
use esp_storage::FlashStorage;

use sequential_storage::cache::NoCache;
use sequential_storage::map::{fetch_item, remove_item, store_item};

use rs_matter_stack::matter::error::{Error, ErrorCode};
use rs_matter_stack::matter::persist::KvBlobStore;

use static_cell::StaticCell;

/// Working buffer for sequential-storage. Must exceed the largest Matter blob (a fabric,
/// bounded by the exchange buffer at ~1.5 KB) plus item overhead. One flash sector is
/// generous and simple. Also reused to read the partition table during construction.
const SS_BUF_SIZE: usize = 4096;

/// The `nvs` partition (24 KB by default) is split into two disjoint sequential-storage
/// regions so the Matter KV and the OpenThread settings never share a store (two
/// independent sequential-storage instances over the same range would corrupt each
/// other's page state). The OpenThread settings are tiny (SRP key + a few blobs), so the
/// tail 8 KB (2 sectors, the sequential-storage minimum) is plenty; Matter KV gets the
/// rest.
const OT_REGION_LEN: u32 = 0x2000;

/// esp-storage requires flash read/write buffers to be word-aligned (WORD_SIZE = 4).
/// A bare `[u8; N]` is only byte-aligned, which fails with `NotAligned`, so force
/// 4-byte alignment.
#[repr(align(4))]
struct AlignedBuf([u8; SS_BUF_SIZE]);

static SS_BUF: StaticCell<AlignedBuf> = StaticCell::new();

/// Separate working buffer for the OpenThread-settings store (its own sequential-storage
/// instance over the tail region). 2 KB comfortably holds the whole serialized settings
/// blob plus overhead.
const OT_BUF_SIZE: usize = 2048;

#[repr(align(4))]
struct OtAlignedBuf([u8; OT_BUF_SIZE]);

static OT_SS_BUF: StaticCell<OtAlignedBuf> = StaticCell::new();

/// Async `NorFlash` adapter over the blocking esp-storage `FlashStorage`. esp-storage's
/// flash ops are synchronous and CPU-blocking (they busy-wait with interrupts/cache
/// disabled), so each async method simply performs the blocking call and returns Ready -
/// there is never any real awaiting, so `block_on` completes in a single poll.
struct AsyncFlash(FlashStorage);

impl ErrorType for AsyncFlash {
    type Error = <FlashStorage as ErrorType>::Error;
}

impl AsyncReadNorFlash for AsyncFlash {
    const READ_SIZE: usize = <FlashStorage as BlockingReadNorFlash>::READ_SIZE;

    async fn read(&mut self, offset: u32, bytes: &mut [u8]) -> Result<(), Self::Error> {
        BlockingReadNorFlash::read(&mut self.0, offset, bytes)
    }

    fn capacity(&self) -> usize {
        BlockingReadNorFlash::capacity(&self.0)
    }
}

impl AsyncNorFlash for AsyncFlash {
    const WRITE_SIZE: usize = <FlashStorage as BlockingNorFlash>::WRITE_SIZE;
    const ERASE_SIZE: usize = <FlashStorage as BlockingNorFlash>::ERASE_SIZE;

    async fn erase(&mut self, from: u32, to: u32) -> Result<(), Self::Error> {
        BlockingNorFlash::erase(&mut self.0, from, to)
    }

    async fn write(&mut self, offset: u32, bytes: &[u8]) -> Result<(), Self::Error> {
        BlockingNorFlash::write(&mut self.0, offset, bytes)
    }
}

// `FlashStorage` is `MultiwriteNorFlash` (NOR flash allows clearing bits without erase),
// which `remove_item` requires.
impl AsyncMultiwrite for AsyncFlash {}

/// Locate the `nvs` partition and return the absolute flash range `[offset, offset+len)`.
/// `scratch` is used to read the partition table (any sufficiently large buffer).
fn nvs_range(flash: &mut FlashStorage, scratch: &mut [u8]) -> Result<Range<u32>, Error> {
    let table = read_partition_table(flash, scratch).map_err(|e| {
        log::warn!("[matter] read_partition_table failed: {e:?}");
        ErrorCode::StdIoError
    })?;
    let nvs = table
        .find_partition(PartitionType::Data(DataPartitionSubType::Nvs))
        .map_err(|e| {
            log::warn!("[matter] find nvs partition failed: {e:?}");
            ErrorCode::StdIoError
        })?
        .ok_or_else(|| {
            log::warn!("[matter] no nvs partition in the table");
            Error::from(ErrorCode::StdIoError)
        })?;
    let offset = nvs.offset();
    log::info!("[matter] nvs partition: offset={offset:#x} len={:#x}", nvs.len());
    Ok(offset..offset + nvs.len())
}

/// A flash-backed `KvBlobStore` over the `nvs` partition.
pub struct FlashKv {
    flash: AsyncFlash,
    range: Range<u32>,
    buf: &'static mut [u8; SS_BUF_SIZE],
}

impl FlashKv {
    /// Construct the store, locating the `nvs` partition. Takes its 4 KB working buffer
    /// from a module `static` (kept off the tight main-task stack).
    pub fn new() -> Result<Self, Error> {
        let buf = &mut SS_BUF.init(AlignedBuf([0u8; SS_BUF_SIZE])).0;

        let mut flash = FlashStorage::new();
        // Reuse `buf` to read the partition table before it becomes the KV working buffer.
        // `read_partition_table` rejects any buffer larger than PARTITION_TABLE_MAX_LEN, so
        // cap the slice (our 4 KB buffer would otherwise be flagged Invalid).
        let full = nvs_range(&mut flash, &mut buf[..PARTITION_TABLE_MAX_LEN])?;
        // Matter KV takes everything except the OpenThread-settings tail region.
        let range = full.start..full.end - OT_REGION_LEN;

        Ok(Self {
            flash: AsyncFlash(flash),
            range,
            buf,
        })
    }
}

/// A flash-backed keyed-blob store over the OpenThread-settings tail region of `nvs`.
/// Used by [`super::ot_settings::FlashSettings`] to persist OpenThread's settings (most
/// importantly the SRP ECDSA key, so re-registration survives reboots).
pub struct SettingsFlash {
    flash: AsyncFlash,
    range: Range<u32>,
    buf: &'static mut [u8; OT_BUF_SIZE],
}

impl SettingsFlash {
    pub fn new() -> Result<Self, Error> {
        let buf = &mut OT_SS_BUF.init(OtAlignedBuf([0u8; OT_BUF_SIZE])).0;

        let mut flash = FlashStorage::new();
        let full = nvs_range(&mut flash, &mut buf[..PARTITION_TABLE_MAX_LEN.min(OT_BUF_SIZE)])?;
        let range = full.end - OT_REGION_LEN..full.end;

        Ok(Self {
            flash: AsyncFlash(flash),
            range,
            buf,
        })
    }

    /// Load the blob for `key` into `out`, returning its length (or `None`).
    pub fn load(&mut self, key: u16, out: &mut [u8]) -> Result<Option<usize>, Error> {
        let found: Option<&[u8]> = block_on(fetch_item::<u16, &[u8], _>(
            &mut self.flash,
            self.range.clone(),
            &mut NoCache::new(),
            &mut self.buf[..],
            &key,
        ))
        .map_err(to_matter_err)?;

        match found {
            Some(v) if v.len() <= out.len() => {
                out[..v.len()].copy_from_slice(v);
                Ok(Some(v.len()))
            }
            Some(_) => Err(ErrorCode::NoSpace.into()),
            None => Ok(None),
        }
    }

    pub fn store(&mut self, key: u16, data: &[u8]) -> Result<(), Error> {
        block_on(store_item::<u16, &[u8], _>(
            &mut self.flash,
            self.range.clone(),
            &mut NoCache::new(),
            &mut self.buf[..],
            &key,
            &data,
        ))
        .map_err(to_matter_err)
    }
}

fn to_matter_err<E: core::fmt::Debug>(e: sequential_storage::Error<E>) -> Error {
    log::warn!("[matter] flash KV op failed: {e:?}");
    ErrorCode::StdIoError.into()
}

impl KvBlobStore for FlashKv {
    fn load<'a>(&mut self, key: u16, buf: &'a mut [u8]) -> Result<Option<&'a [u8]>, Error> {
        let found: Option<&[u8]> = block_on(fetch_item::<u16, &[u8], _>(
            &mut self.flash,
            self.range.clone(),
            &mut NoCache::new(),
            &mut self.buf[..],
            &key,
        ))
        .map_err(to_matter_err)?;

        match found {
            Some(value) => {
                log::info!("[matter] KV load key={key} -> {} bytes", value.len());
                if value.len() > buf.len() {
                    return Err(ErrorCode::NoSpace.into());
                }
                buf[..value.len()].copy_from_slice(value);
                Ok(Some(&buf[..value.len()]))
            }
            // `None` is logged at trace only: loading scans all 255 fabric slots on every
            // boot, so info-level would flood the console.
            None => Ok(None),
        }
    }

    fn store(&mut self, key: u16, data: &[u8], _buf: &mut [u8]) -> Result<(), Error> {
        block_on(store_item::<u16, &[u8], _>(
            &mut self.flash,
            self.range.clone(),
            &mut NoCache::new(),
            &mut self.buf[..],
            &key,
            &data,
        ))
        .map_err(to_matter_err)?;
        log::info!("[matter] KV store key={key} <- {} bytes ok", data.len());
        Ok(())
    }

    fn remove(&mut self, key: u16, _buf: &mut [u8]) -> Result<(), Error> {
        block_on(remove_item::<u16, _>(
            &mut self.flash,
            self.range.clone(),
            &mut NoCache::new(),
            &mut self.buf[..],
            &key,
        ))
        .map_err(to_matter_err)
    }
}

/// Erase the entire `nvs` partition, wiping all persisted Matter/Thread pairing state.
///
/// Constructs its own `FlashStorage` (independent of any live [`FlashKv`]); intended to be
/// called by the factory-reset path immediately before resetting the chip, so there is no
/// concurrent flash access to worry about. On the next boot the store reads back empty and
/// the device is un-commissioned -> BLE commissioning.
pub fn wipe_pairing_data() -> Result<(), Error> {
    let mut flash = FlashStorage::new();
    let mut scratch = [0u8; 512];
    let range = nvs_range(&mut flash, &mut scratch)?;

    let mut flash = AsyncFlash(flash);
    block_on(sequential_storage::erase_all(&mut flash, range)).map_err(to_matter_err)
}
