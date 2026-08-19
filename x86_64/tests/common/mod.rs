//! Shared by the test images.  A directory, not a flat file: cargo does
//! not treat one as a test target, and xtask does not take it for an image
//! whose `[[test]]` entry was forgotten.

/// Report and end the run on the first failure.  A test image has no
/// unwinding and nothing to hand a failure back to but its exit status.
///
/// Paths are absolute so that the expansion does not depend on what the
/// image happens to have imported.
macro_rules! check {
    ($cond:expr, $($arg:tt)+) => {
        if $cond {
            ::port::println!("ok    {}", format_args!($($arg)+));
        } else {
            ::port::println!("FAIL  {}", format_args!($($arg)+));
            ::x86_64::qemu::exit(port::qemu::FAIL);
        }
    };
}
