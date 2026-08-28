---
status: done
---

# Pl011Uart configures GPIO pull-up/down through the PL011 register range

`Pl011Uart::new` finds and maps the GPIO register block into
`gpio_virtrange` (`aarch64/src/uartpl011.rs:38-46`), stores it (`:57`), and
then never reads it. `grep` finds no use of `self.gpio_virtrange` anywhere.

Meanwhile `Pl011Uart::gpiosetpull` (`aarch64/src/uartpl011.rs:92-114`)
writes `GPPUD` and `GPPUDCLK0` — which are GPIO controller registers —
through `self.pl011_virtrange`, the UART's own register block:

    write_reg(self.pl011_virtrange, GPPUD, pull as u32);
    write_reg(self.pl011_virtrange, gppudclk_reg, pud_bit);

Compare `MiniUart::gpiosetpull` (`aarch64/src/uartmini.rs:124-133`), which
writes exactly those registers through `self.gpio_virtrange`. One of the two
is wrong, and the PL011 one looks like the copy that drifted.

Consequence, with the manuals: `GPPUD` (0x94) and `GPPUDCLK0` (0x98) are
BCM2835 GPIO block registers — *BCM2835 ARM Peripherals* §6.1, at absolute
0x7E200094 and 0x7E200098. In the PL011 register map those same offsets are
reserved space: *ARM PrimeCell UART (PL011) TRM* r1p5 §3.2 defines registers
at 0x000–0x04C and then 0xFE0–0xFFC, nothing between. So on a real Pi the
pull-up/down configuration that `init()`'s comment claims to perform never
reaches the pad control, and the writes land in reserved device space where
the block's behaviour is not architecturally defined.

QEMU's PL011 model ignores writes to those reserved offsets, so the bug is
invisible on the emulated target — the devboard hides it rather than the bug
being absent. The PL011 is also qemu's default UART, where GPIO pull state
does not matter at all, which is likely why this has gone unnoticed.

The struct carries `#[allow(dead_code)]` (`:25`) and the impl block does
too (`:34`), which is what suppresses the unused-field warning that would
otherwise have caught this.

Evidence that it is pre-existing, not introduced by the by-value sweep: the
same lines read `write_reg(&self.pl011_virtrange, GPPUD, ...)` before the
sweep; only the `&` changed.

Fix direction: point `gpiosetpull` at `self.gpio_virtrange`, matching
`uartmini.rs`. Then check whether the `#[allow(dead_code)]` on the struct
and impl can be dropped — they may be hiding more.

Done when: `gpiosetpull` writes GPPUD/GPPUDCLK0 through the GPIO register
range; `gpio_virtrange` is actually used; gates clean on all three
architectures.

Origin: noticed while implementing `range-by-value-sweep.md` — the sweep
touched these lines but deliberately did not change their behaviour.

## Status: done

Landed in b41e765: `gpiosetpull` writes GPPUD/GPPUDCLK0 through
`gpio_virtrange`, matching `uartmini.rs`, and both `#[allow(dead_code)]`
(struct and impl) came out with it — `gpio_virtrange` is now used, and the
warning suppression that hid this bug is gone. Two follow-ups in the same
arc: ab22228 adds the bcm2711 GPIO compatible the rpi4b's DT actually
uses, and 97a672d lands the pl011 and mini-uart integration images that
construct the real drivers from the DT and read the config registers
back — the pl011 image is what caught the bcm2711 gap.
