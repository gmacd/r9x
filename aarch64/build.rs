//! Stage the built server ELFs into `OUT_DIR` for the server-embedding
//! integration images to `include_bytes!`.
//!
//! Each ELF is produced by xtask's `ServerStep` (aarch64 only) at
//! `target/<target>/<profile>/<server>.elf`.  This script reruns when any of
//! them changes (mtime), so a rebuilt server re-embeds the new bytes — the
//! "change the server, the image rebuilds" property the arc proves.  A bare
//! `cargo build` of the image outside xtask, with no prior server build, fails
//! loudly here rather than with a missing-file mystery at the `include_bytes!`.
//!
//! The build script always runs for the host, so it may use `std` (the aarch64
//! crate it belongs to is `no_std` — the script is a separate host artifact).
//! It runs once per build of the crate, for every target of it, so the kernel
//! image build transitively needs every server ELF staged even though only the
//! server-embedding images (`console_server`, and the `namespace` image) embed
//! them; xtask's build path runs the `ServerStep` first, and a bare build
//! fails here instead.

use std::path::PathBuf;

/// The aarch64 user-space servers the embedding images may include; must match
/// xtask's `ServerStep::SERVERS` (the two stage the same paths).
const SERVERS: [&str; 5] = ["console", "nameserver", "init", "heaptask", "child"];

fn main() {
    let manifest_dir =
        PathBuf::from(std::env::var("CARGO_MANIFEST_DIR").expect("CARGO_MANIFEST_DIR"));
    // The aarch64 crate sits at the workspace root's edge, so its parent is
    // the root the conventional `target/` lives in.
    let root = manifest_dir.parent().expect("aarch64 crate has a parent directory");
    // The target dir: an explicit CARGO_TARGET_DIR (absolute, or relative to
    // the workspace root) or the conventional <root>/target.
    let target_dir = match std::env::var("CARGO_TARGET_DIR") {
        Ok(dir) if !dir.is_empty() => {
            let p = PathBuf::from(&dir);
            if p.is_absolute() { p } else { root.join(p) }
        }
        _ => root.join("target"),
    };
    // The staged ELFs sit under the user-space target name (`r9x-aarch64`, the
    // spec's filename stem) — a fixed part of the repo, not `TARGET`, which
    // varies by how this crate is being built: a JSON-spec build and the
    // host-toolchain check step use different target names for it.  It must
    // match xtask's `Arch::user_target` (the two stage the same paths).
    let profile = std::env::var("PROFILE").expect("PROFILE");
    let out_dir = std::env::var("OUT_DIR").expect("OUT_DIR");
    for server in SERVERS {
        let elf = target_dir.join("r9x-aarch64").join(&profile).join(format!("{server}.elf"));

        // Rebuild this script — and so the embedding image — when this
        // server's ELF changes: the edge of the "change the server, the image
        // rebuilds" property.
        println!("cargo:rerun-if-changed={}", elf.display());

        if !elf.exists() {
            panic!(
                "{server}.elf not found at {};\nbuild the servers first: `cargo xtask build --arch aarch64`",
                elf.display()
            );
        }
        std::fs::copy(&elf, PathBuf::from(&out_dir).join(format!("{server}.elf")))
            .unwrap_or_else(|e| panic!("copy {server}.elf into OUT_DIR: {e}"));
    }
}
