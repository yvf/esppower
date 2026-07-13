# Phase 4b - rs-matter-stack transport glue (design / roadmap)

> **Status (2026-07-05): fully implemented and working on hardware.** All five
> adapters exist under `src/matter/` (`net.rs`, `netif.rs`, `mdns.rs`, `netctl.rs`,
> `gatt.rs`) and the stack commissions + operates end-to-end. **Key deviation from
> the plan below:** the run loop uses **non-concurrent `stack.run(...)`, NOT
> `run_coex(...)`** - on the H2's single shared radio, BLE + Thread simultaneously is
> unreliable, so BLE runs only while un-commissioned and shuts off once a fabric
> exists (`PreexistingWireless` implements both `Thread`+`Gatt` and `ThreadCoex`, so
> it's the same five adapters, different orchestration). See `src/matter/stack.rs`.
> The C2 variable-length wrinkle (below) was solved with a `heapless 0.9` Vec value.

Goal: run a Matter-over-Thread node by feeding rs-matter-stack our openthread
(Thread) + trouble (BLE) transports. The full dependency graph already builds
(Phase 4a); this is the integration code.

## Approach: `PreexistingWireless` (no custom `ThreadCoex`)

rs-matter-stack provides `PreexistingWireless<S, N, C, M, P>` which **already
implements `Thread` + `ThreadCoex`** (`wireless/thread.rs:478,501`) given five
components. So we don't implement `ThreadCoex` by hand - we supply adapters and
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
sends the operational dataset via the NetworkCommissioning cluster -> our `NetCtl`
applies it to openthread (`set_active_dataset_tlv` + `enable_thread`). This is why
we don't need to hardcode `THREAD_DATASET` for the real flow.

## Adapters to write (order = easiest->hardest; build after each)

All rs-matter traits are reachable as `rs_matter_stack::matter::dm::...`. NOTE the
embassy-sync split: trouble pulls 0.7, rs-matter pulls 0.8 - any `Mutex` inside
an adapter that touches rs-matter types must use **0.8** (rs-matter-stack's),
e.g. `rs_matter_stack::matter::utils::sync::...` / `CriticalSectionRawMutex`.

1. **NetStack** (`rs_matter_stack::nal::NetStack`) - `OpenThread` already impls
   edge-nal `UdpBind`/`UdpConnect` (`openthread/src/enal.rs`). Wrap it: `udp_bind`/
   `udp_connect` -> the OpenThread handle; `tcp_bind`/`tcp_connect`/`dns` ->
   `rs_matter_stack::nal::noop::NoopNet` (Matter-over-Thread is UDP-only).
   Watch the GAT lifetimes (`type UdpBind<'a>`).

2. **Netif** = `NetifDiag` + `NetChangeNotif` (`dm::clusters::gen_diag` +
   `dm::networks`). `netifs()` builds a `NetifInfo` from `ot.ipv6_addrs(..)` +
   `ot.net_status()` (interface type = `Thread`); `wait_changed()` =
   `ot.wait_changed().await`.

3. **Mdns** (`rs_matter_stack::mdns::...`/`dm`) - register the Matter operational
   + commissionable services over openthread **SRP** (`ot.srp_set_conf` +
   `ot.srp_add_service`, see `openthread/examples/.../srp.rs`).

4. **NetCtl** + **ThreadDiag** (`dm::clusters::net_comm` + `thread_diag`) - the
   critical one. `NetCtl`: apply the dataset the commissioner provides
   (`ot.set_active_dataset_tlv` + `ot.enable_thread(true)`), report scan/connect.
   `ThreadDiag`: report role/pan-id/channel from `ot.net_status()`/`ot.netdata`.

5. **GattPeripheral** (`rs_matter_stack::ble::GattPeripheral`) - **the hardest;
   not yet implemented.** It owns the whole BLE stack lifecycle (trouble Host +
   runner + GATT server + advertise + the BTP pumps), so it's roughly Phase-3-in-
   full plus the BTP shuttle. All the pieces are researched - concrete build sheet:

   **BTP service** (`#[gatt_service]`, reuse the Phase-3 trouble pattern):
   - Service UUID `0000FFF6-0000-1000-8000-00805F9B34FB` (16-bit 0xFFF6).
   - **C1** `18EE2EF5-263D-4559-959F-4F9C429F9D11` - props `write`
     (commissioner->device). C1_MAX_LEN = `MAX_BTP_SEGMENT_SIZE + GATT_HEADER_SIZE`.
   - **C2** `...9D12` - props `indicate` (device->commissioner) + a CCCD.
   - (C3 `...8F04`, `read`, additional-commissioning-data - optional; skip first.)
   - Constants in `rs_matter::transport::network::btp::gatt` (UUIDs, lengths).

   **`run(&mut self, btp, service_name, service_adv)`:** build the trouble Host
   from the stored `BleConnector`/controller (as Phase-3 `ble_peripheral`);
   advertise the BTP service with `service_adv` (an `AdvData` - encode its bytes
   into the adv payload alongside the 0xFFF6 service UUID); `accept()` ->
   `with_attribute_server`; then `select3` of:
   - **runner** (`runner.run()`),
   - **incoming pump**: on `GattConnectionEvent::Gatt{event}` where
     `WriteEvent::handle() == server.btp.c1.handle` -> `btp.process_incoming(
     Some(conn.raw().att_mtu()), peer_to_btaddr(conn.raw().peer_address()),
     event.data())`; then `event.accept()`.
   - **outgoing pump**: `loop { btp.wait_outgoing().await; let n =
     btp.process_outgoing(Some(mtu), &mut buf)?; if n>0 { server.btp.c2
     .indicate(&conn, &buf_value).await } }`.

   **trouble API confirmed (0.6):** `Characteristic::indicate(&conn, &value)`
   (attribute.rs:970, no-op if CCCD unsubscribed); `WriteEvent::handle()`/`data()`
   (gatt.rs:408+); `Connection::att_mtu()` (connection.rs:486);
   `Connection::peer_address() -> BdAddr` (501) -> map to `rs_matter...btp::BtAddr`.

   **Wrinkle to solve:** trouble characteristics are *typed/fixed-size* but C2
   indications are *variable-length*. First cut: declare C2 as `[u8; C2_MAX_LEN]`
   and indicate the fixed buffer; verify on hardware whether the peer needs the
   exact length (if so, use trouble's variable-length value path). This is the
   main thing to validate with a real commissioner.

   Shape: `run` advertises the BTP service then `select`s the trouble runner, the
   incoming pump (`process_incoming`), and the outgoing pump (`process_outgoing`).

## Run loop (after adapters compile)

`ThreadMatterStack::<BUMP, ()>` in a `StaticCell` (mind RAM - we have lots now);
`stack.run_coex(PreexistingWireless::new(...), &crypto, (NODE, handler), &kv, ())`.
Device model + handler = the On/Off plug (RCD resetter) + the contact sensor
(mains presence). (The shipping firmware uses non-concurrent `run`, not `run_coex`
- see `no-std-plan.md`.)

## Build

`./build.sh` - see `no-std-plan.md` for the host prerequisites (brew-LLVM clang, cmake).
