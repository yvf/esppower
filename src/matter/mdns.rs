//! `Mdns` adapter over openthread **SRP** (Matter-over-Thread registers services
//! with the border router's SRP server, which advertises them on the infra link;
//! Matter's UDP mDNS responder isn't used). Port of esp-idf-matter's
//! `EspMatterThreadSrp`, swapping the esp-idf SRP calls for openthread's.

use core::fmt::Write as _;

use heapless::{String, Vec};

use openthread::{OpenThread, SrpConf, SrpService, SrpServiceSlot};

use rs_matter_stack::matter::crypto::Crypto;
use rs_matter_stack::matter::error::{Error, ErrorCode};
use rs_matter_stack::matter::transport::network::MatterLocalService;
use rs_matter_stack::matter::Matter;
use rs_matter_stack::mdns::Mdns;
use rs_matter_stack::nal::UdpBind;

use core::net::{Ipv4Addr, Ipv6Addr};

/// max-fabrics-2 → up to (fabrics + 1) operational/commissionable services.
const MAX_MATTER_SERVICES: usize = 3;
const OT_MDNS_BUF_SZ: usize = 256;

fn to_err<E: core::fmt::Debug>(err: E) -> Error {
    // rs-matter's error codes are coarse; SRP/Ot failures map to a network error.
    log::warn!("[matter] OtMdns/SRP error: {err:?}");
    ErrorCode::NoNetworkInterface.into()
}

/// `Mdns` over openthread SRP.
pub struct OtMdns<'a> {
    ot: OpenThread<'a>,
    eui64: [u8; 8],
    services: Vec<(MatterLocalService, SrpServiceSlot), MAX_MATTER_SERVICES>,
    mdns_buf: Vec<u8, OT_MDNS_BUF_SZ>,
}

impl<'a> OtMdns<'a> {
    pub fn new(ot: OpenThread<'a>, eui64: [u8; 8]) -> Self {
        Self {
            ot,
            eui64,
            services: Vec::new(),
            mdns_buf: Vec::new(),
        }
    }

    async fn run_srp(&mut self, matter: &Matter<'_>) -> Result<(), Error> {
        let mut hostname = String::<16>::new();
        for b in self.eui64 {
            write!(hostname, "{b:02X}").unwrap();
        }

        log::info!("[matter] OtMdns::run starting (SRP)");
        // Non-fatal: SRP auto-start just enables the client; it connects once
        // Thread is up + a server is found, so a failure here must not kill the run.
        if let Err(e) = self.ot.srp_autostart() {
            log::warn!("[matter] srp_autostart failed (will retry as Thread comes up): {e:?}");
        }

        // (Re)register the SRP host if it isn't already our hostname.
        let register_host = self
            .ot
            .srp_conf(|conf, _state, free| Ok(free || conf.host_name != hostname.as_str()))
            .map_err(to_err)?;

        if register_host {
            self.ot
                .srp_set_conf(&SrpConf {
                    host_name: &hostname,
                    // Shorten the SRP leases from OpenThread's defaults (2 h lease /
                    // 14-day KEY lease). The key-lease is what reserves the hostname for
                    // our SRP key on the border router: with the 14-day default, if the
                    // device is re-commissioned with a new key (e.g. after a factory
                    // reset) it collides with the stale reservation (OT_ERROR_DUPLICATED)
                    // for up to two weeks. 1 h means a stale reservation clears quickly;
                    // the device auto-renews well within the interval so normal operation
                    // is unaffected.
                    default_lease_secs: 3600,
                    default_key_lease_secs: 3600,
                    ..SrpConf::new()
                })
                .map_err(to_err)?;
        }

        loop {
            matter.transport().wait_mdns().await;

            let mut services = Vec::<_, MAX_MATTER_SERVICES>::new();
            matter.mdns_services(|service| {
                services
                    .push(service)
                    .map_err(|_| Error::from(ErrorCode::ConstraintError))?;
                Ok(())
            })?;

            self.update_services(matter, &services)?;
        }
    }

    fn update_services(
        &mut self,
        matter: &Matter,
        services: &[MatterLocalService],
    ) -> Result<(), Error> {
        // Register newly-appeared services.
        for service in services {
            if !self.services.iter().any(|(s, _)| s == service) {
                let slot = self.register(matter, service)?;
                self.services
                    .push((service.clone(), slot))
                    .map_err(|_| Error::from(ErrorCode::ConstraintError))?;
            }
        }

        // Deregister services that disappeared.
        loop {
            let removed = self
                .services
                .iter()
                .find(|(service, _)| !services.contains(service))
                .map(|(_, slot)| *slot);

            if let Some(slot) = removed {
                self.ot.srp_remove_service(slot, false).map_err(to_err)?;
                self.services.retain(|(_, s)| *s != slot);
            } else {
                break;
            }
        }

        Ok(())
    }

    fn register(
        &mut self,
        matter: &Matter,
        service: &MatterLocalService,
    ) -> Result<SrpServiceSlot, Error> {
        self.mdns_buf.resize_default(OT_MDNS_BUF_SZ).unwrap();

        let (service, _) = service.service(matter.dev_det(), matter.port(), &mut self.mdns_buf)?;
        // ManuallyDrop works around the `'a` lifetime on `srp_add_service`'s arg.
        let service = core::mem::ManuallyDrop::new(service);

        let srp_service = core::mem::ManuallyDrop::new(SrpService {
            name: service.service_protocol,
            instance_name: service.name,
            port: service.port,
            subtype_labels: service.service_subtypes.clone(),
            txt_entries: service.txt_kvs.clone().map(|(k, v)| (k, v.as_bytes())),
            priority: 0,
            weight: 0,
            lease_secs: 0,
            key_lease_secs: 0,
        });

        self.ot.srp_add_service(&srp_service).map_err(to_err)
    }
}

impl Mdns for OtMdns<'_> {
    async fn run<C, U>(
        &mut self,
        matter: &Matter<'_>,
        _crypto: C,
        _udp: U,
        _mac: &[u8],
        _ipv4: Ipv4Addr,
        _ipv6: Ipv6Addr,
        _interface: u32,
    ) -> Result<(), Error>
    where
        C: Crypto,
        U: UdpBind,
    {
        self.run_srp(matter).await
    }
}
