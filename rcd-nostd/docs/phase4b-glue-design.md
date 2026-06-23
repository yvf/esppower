# Phase 4b — rs-matter-stack transport glue (design / roadmap)

Goal: run a Matter-over-Thread node by feeding rs-matter-stack our openthread
(Thread) + trouble (BLE) transports. The full dependency graph already builds
(Phase 4a); this is the integration code.

## Approach: `PreexistingWireless` (no custom `ThreadCoex`)

rs-matter-stack provides `PreexistingWireless<S, N, C, M, P>` which **already
implements `Thread` + `ThreadCoex`** (`wireless/thread.rs:478,501`) given five
components. So we don't implement `ThreadCoex` by hand — we supply adapters and
hand the bundle to `ThreadMatterStack::run_coex(...)`:

```
PreexistingWireless::new(net_stack, netif, net_ctl, mdns, gatt)
  S = NetStack                          (UDP over Thread)
  N = NetifDiag + NetChangeNotif        (interface addresses)
  C = NetCtl + ThreadDiag + NetChangeNotif   (apply dataset / status)
  M = Mdns                              (service registration)
  P = GattPeripheral                    (BLE commissioning)
```

The device gets onto Thread **during Matter commissioning**: the commissioner
sends the operational dataset via the NetworkCommissioning cluster → our `NetCtl`
applies it to openthread (`set_active_dataset_tlv` + `enable_thread`). This is why
we don't need to hardcode `THREAD_DATASET` for the real flow.

## Adapters to write (order = easiest→hardest; build after each)

All rs-matter traits are reachable as `rs_matter_stack::matter::dm::...`. NOTE the
embassy-sync split: trouble pulls 0.7, rs-matter pulls 0.8 — any `Mutex` inside
an adapter that touches rs-matter types must use **0.8** (rs-matter-stack's),
e.g. `rs_matter_stack::matter::utils::sync::...` / `CriticalSectionRawMutex`.

1. **NetStack** (`rs_matter_stack::nal::NetStack`) — `OpenThread` already impls
   edge-nal `UdpBind`/`UdpConnect` (`openthread/src/enal.rs`). Wrap it: `udp_bind`/
   `udp_connect` → the OpenThread handle; `tcp_bind`/`tcp_connect`/`dns` →
   `rs_matter_stack::nal::noop::NoopNet` (Matter-over-Thread is UDP-only).
   Watch the GAT lifetimes (`type UdpBind<'a>`).

2. **Netif** = `NetifDiag` + `NetChangeNotif` (`dm::clusters::gen_diag` +
   `dm::networks`). Template: esp-idf-matter `netif.rs:29-90`. `netifs()` builds a
   `NetifInfo` from `ot.ipv6_addrs(..)` + `ot.net_status()` (interface type =
   `Thread`); `wait_changed()` = `ot.wait_changed().await`.

3. **Mdns** (`rs_matter_stack::mdns::...`/`dm`) — register the Matter operational
   + commissionable services over openthread **SRP** (`ot.srp_set_conf` +
   `ot.srp_add_service`, see `openthread/examples/.../srp.rs`). Template:
   esp-idf-matter `thread.rs` `EspMatterThreadSrp` (Mdns impl ~line 605).

4. **NetCtl** + **ThreadDiag** (`dm::clusters::net_comm` + `thread_diag`) — the
   critical one. `NetCtl`: apply the dataset the commissioner provides
   (`ot.set_active_dataset_tlv` + `ot.enable_thread(true)`), report scan/connect.
   `ThreadDiag`: report role/pan-id/channel from `ot.net_status()`/`ot.netdata`.
   Template: esp-idf-matter `thread.rs` `EspMatterThreadCtl` (NetCtl ~94,
   ThreadDiag ~307).

5. **GattPeripheral** (`rs_matter_stack::ble::GattPeripheral`) — hardest.
   `run(&mut self, btp: &Btp, service_name, service_adv)`: advertise the Matter
   BTP service (UUID **0xFFF6**) with `service_adv`, and on connect shuttle the
   two BTP characteristics — **C1** (`18EE2EF5-263D-4559-959F-4F9C429F9D11`,
   commissioner→device, Write) into `btp`, and `btp` output → **C2**
   (`18EE2EF5-263D-4559-959F-4F9C429F9D12`, device→commissioner, Indicate). Reuse
   the Phase-3 trouble plumbing (advertise/connect/GATT events); replace the
   placeholder battery service with the BTP service + a CCCD on C2. Template:
   esp-idf-matter `ble.rs` (`EspBtpGattPeripheral`) for the C1/C2↔Btp pump shape.

## Run loop (after adapters compile)

`ThreadMatterStack::<BUMP, ()>` in a `StaticCell` (mind RAM — we have lots now);
`stack.run_coex(PreexistingWireless::new(...), &crypto, (NODE, handler), &kv, ())`.
`run_coex` auto-prints the commissioning QR + pairing code. Device model + handler
= the On/Off plug ported from the esp-idf `PlugHooks` (Phase 5).

## Build

`./build.sh` (cmake 3.x + brew-LLVM clang pinned). Keep the crate green: develop
the `matter` module and only `mod matter;` it once it compiles; wire `run_coex`
into `main` last.
