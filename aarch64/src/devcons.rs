// Racy to start.

use crate::uartpl011::Pl011Uart;
use port::devcons::{Console, IprintOps, Uart};
use port::once::Once;
use r9x_core::fdt::DeviceTree;

#[cfg(target_os = "none")]
use port::println;

// The aarch64 devcons implementation is focussed on Raspberry Pi 4 for now.

// Useful links
// - Raspberry Pi Processors
//     https://www.raspberrypi.com/documentation/computers/processors.html
// - Raspberry Pi Hardware
//     https://www.raspberrypi.com/documentation/computers/raspberry-pi.html
// - Raspi4 BCM2711
//     Datasheet https://datasheets.raspberrypi.com/bcm2711/bcm2711-peripherals.pdf
// - Mailbox
//     https://github.com/raspberrypi/firmware/wiki/Mailbox-property-interface

// Raspberry Pi 4 has 4 UARTs:
// - UART0 PL011
// - UART1 miniUART
// - UART2 PL011
// - UART3 PL011

static UART: Once<Pl011Uart> = Once::new();

static IPRINT_OPS: IprintOps = IprintOps { putb: iputb };

/// Direct polled write for iprint, bypassing the console lock.
/// `Pl011Uart::putb` needs only a shared reference, so this can safely
/// alias the reference held by the console.  Drops the byte if the
/// console is not up yet — `Once` makes that a check rather than an
/// assumption.
pub(crate) fn iputb(b: u8) {
    if let Some(uart) = UART.get() {
        uart.putb(b);
    }
}

pub fn init(dt: &DeviceTree) {
    Console::set_uart(|| {
        let uart = Pl011Uart::new(dt);

        // Return a statically initialised Pl011Uart.  If that couldn't be done for some reason,
        // return None and hope that things work out regardless
        match uart {
            Ok(uart) => {
                uart.init();
                match UART.set(uart) {
                    Ok(uart) => {
                        port::devcons::set_iprint_ops(&IPRINT_OPS);
                        Ok(uart as &'static dyn Uart)
                    }
                    Err(_) => Err("uart already initialised"),
                }
            }
            Err(msg) => {
                println!("can't initialise uart: {msg:?}");
                Err("can't initialise uart")
            }
        }
    });
}
