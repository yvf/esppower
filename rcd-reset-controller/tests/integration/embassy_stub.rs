// Satisfies the linker for integration test examples that use only blocking I/O.
//
// esp-idf-svc's embassy-time-driver always compiles in `_embassy_time_schedule_wake`, which
// references `__embassy_time_queue_item_from_waker`. Without the executor running, LTO + GC
// eliminates the real definition from embassy-executor (because its callee `task_from_waker`
// is dead). This stub provides the missing symbol. It is unreachable at runtime because
// these examples never schedule async timers.
#[no_mangle]
unsafe fn __embassy_time_queue_item_from_waker(_: &::core::task::Waker) -> *mut () {
    unreachable!()
}
