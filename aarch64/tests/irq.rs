//! Integration test: IRQ routing table and `try_send` into a user channel.
//!
//! The kernel claims INTID 129 (PL011 UART SPI) directly via
//! `ipc::sys_irq_claim`, then verifies the routing table has an entry,
//! then delivers the interrupt by calling the same code the trap handler
//! runs: `ipc::route(129)` + `port::ipc::try_send`.  The `try_send`
//! succeeds (the message is enqueued on the channel).
//!
//! The GIC IAR->EOI path is already exercised by the timer PPI through the
//! same trap handler IRQ dispatch.  This image proves the SPI-specific
//! half: the routing table, the claim, and `try_send` into a process-owned
//! channel.

#![no_std]
#![no_main]

use aarch64::{boot, ipc, qemu};
use port::ipc::Message;
use port::println;

#[macro_use]
mod common;

const PL011_INTID: u16 = 129;

#[unsafe(no_mangle)]
pub extern "C" fn main9(dtb_va: usize) {
    boot::irq_ops();
    let dt = unsafe { boot::device_tree(dtb_va) };
    boot::page_allocator(&dt, dtb_va).unwrap();
    boot::console(&dt);
    boot::interrupts(&dt);

    println!("running irq-route");

    // Create the channel the IRQ will be routed to.
    let ch = ipc::create();
    assert_eq!(ch as u32, 0, "channel handle is 0");

    // Claim INTID 129 on channel 0: enables the GIC interrupt and adds
    // the routing table entry.
    let result = ipc::sys_irq_claim(PL011_INTID as u64, 0);
    check!(result == 0, "SYSIRQCLAIM succeeded, got {result}");

    // Verify the routing table has an entry for INTID 129.
    let route = ipc::route(PL011_INTID);
    check!(route.is_some(), "routing table has an entry for INTID {PL011_INTID}");

    // Deliver the interrupt: the same code the trap handler runs.
    if let Some(channel) = route {
        let msg = Message::new(PL011_INTID, 0, &[]);
        let result = port::ipc::try_send(&ipc::KernSched, channel, msg);
        check!(result.is_ok(), "try_send succeeded: {result:?}");
    }

    println!("irq-route passed");
    qemu::exit(qemu::PASS);
}
