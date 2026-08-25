use crate::config::Configuration;
use config::{apply_to_build_step, apply_to_clippy_step, apply_to_qemu_step};
use std::{
    env, fmt,
    io::{BufRead, BufReader},
    path::{Path, PathBuf},
    process::{self, Command},
    str::FromStr,
    time::Duration,
};
use target_lexicon::Triple;

mod config;

type DynError = Box<dyn std::error::Error>;
type Result<T> = std::result::Result<T, DynError>;

#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Profile {
    Debug,
    Release,
}

impl Profile {
    fn from(matches: &clap::ArgMatches) -> Self {
        if matches.get_flag("release") { Profile::Release } else { Profile::Debug }
    }

    fn dir(&self) -> &'static str {
        match self {
            Profile::Debug => "debug",
            Profile::Release => "release",
        }
    }
}

impl fmt::Display for Profile {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        write!(f, "{self:?}")
    }
}

#[derive(Clone, Copy, Debug, PartialEq, clap::ValueEnum)]
enum Arch {
    Aarch64,
    Riscv64,
    X86_64,
}

impl Arch {
    /// Every architecture r9 supports, in the order the gates run them.
    const ALL: [Arch; 3] = [Arch::Aarch64, Arch::Riscv64, Arch::X86_64];

    /// The workspace package for this arch.  Also what
    /// `std::env::consts::ARCH` reports on a host of this architecture.
    fn package(&self) -> String {
        self.to_string().to_lowercase()
    }

    fn from(matches: &clap::ArgMatches) -> Self {
        *matches.get_one::<Arch>("arch").unwrap_or(&Arch::X86_64)
    }

    fn qemu_system(&self) -> String {
        env_or(
            "QEMU",
            match self {
                Arch::Aarch64 => "qemu-system-aarch64",
                Arch::Riscv64 => "qemu-system-riscv64",
                Arch::X86_64 => "qemu-system-x86_64",
            },
        )
    }

    fn target(&self) -> String {
        env_or("TARGET", format!("{}-unknown-none-elf", self.to_string().to_lowercase()).as_str())
    }

    /// The process exit status a passing test image leaves QEMU with.
    ///
    /// Zero everywhere the guest can choose its own status.  x86-64 exits
    /// through isa-debug-exit, which returns `(value << 1) | 1` and so can
    /// never return zero; the value is the one its `qemu::PASS` writes.
    fn passing_status(&self) -> i32 {
        match self {
            Arch::Aarch64 | Arch::Riscv64 => 0,
            Arch::X86_64 => port::qemu::PASS_STATUS,
        }
    }
}

impl fmt::Display for Arch {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        write!(f, "{self:?}")
    }
}

struct RustupState {
    installed_targets: Vec<Triple>,
    host_triple: String,
}

impl RustupState {
    /// Runs rustup command to get a list of all installed toolchains.
    /// Also caches the host triple.
    fn new() -> Result<Self> {
        Ok(Self {
            installed_targets: Self::installed_rustup_targets()?,
            host_triple: Self::host_triple()?,
        })
    }

    /// The triple rustc actually runs on, from `rustc -vV`.  The toolchain
    /// name is no guide: a linked toolchain can be called anything.
    fn host_triple() -> Result<String> {
        let output =
            Command::new("rustc").arg("-vV").output().map_err(|e| format!("rustc -vV: {e}"))?;
        if !output.status.success() {
            return Err(String::from_utf8_lossy(&output.stderr).trim().to_string().into());
        }
        String::from_utf8(output.stdout)?
            .lines()
            .find_map(|line| line.strip_prefix("host: "))
            .map(str::to_string)
            .ok_or_else(|| "rustc -vV reported no host triple".into())
    }

    /// Call `rustup target list --installed` to get all installed target triples
    fn installed_rustup_targets() -> Result<Vec<Triple>> {
        let output = Command::new("rustup")
            .arg("target")
            .arg("list")
            .arg("--installed")
            .output()
            .map_err(|e| format!("rustup target list --installed: {e}"))?;
        if !output.status.success() {
            // rustup reports its errors on stderr
            return Err(String::from_utf8_lossy(&output.stderr).trim().to_string().into());
        }

        Ok(String::from_utf8(output.stdout)?.lines().flat_map(Triple::from_str).collect())
    }

    /// For the given arch, return a compatible target triple that is
    /// installed and can be used by cargo check.  It prefers the host's
    /// own triple (whose binaries can run), and otherwise looks for
    /// <arch>-unknown-linux-gnu.
    fn std_supported_target(&self, arch: &str) -> Option<&Triple> {
        let arch = Self::target_arch(arch);
        let matching: Vec<&Triple> =
            self.installed_targets.iter().filter(|t| t.architecture.to_string() == arch).collect();
        matching
            .iter()
            .find(|t| t.to_string() == self.host_triple)
            .or_else(|| {
                matching.iter().find(|t| t.to_string() == format!("{arch}-unknown-linux-gnu"))
            })
            .copied()
    }

    /// Return the arch in a form compatible with the supported targets and toolchains
    fn target_arch(arch: &str) -> &str {
        match arch {
            "riscv64" => "riscv64gc",
            _ => arch,
        }
    }
}

fn main() {
    let matches = clap::Command::new("xtask")
        .version("0.1.0")
        .author("The r9 Authors")
        .about("Build support for the r9 operating system")
        .arg_required_else_help(true)
        .subcommand(
            clap::Command::new("build").about("Builds r9").args(&[
                clap::arg!(--release "Build release version").conflicts_with("debug"),
                clap::arg!(--debug "Build debug version (default)").conflicts_with("release"),
                clap::arg!(--arch <arch> "Target architecture")
                    .value_parser(clap::builder::EnumValueParser::<Arch>::new()),
                clap::arg!(--config <name> "Configuration")
                    .value_parser(clap::builder::NonEmptyStringValueParser::new())
                    .default_value("default"),
                clap::arg!(--verbose "Print commands"),
            ]),
        )
        .subcommand(
            clap::Command::new("expand").about("Expands r9 macros").args(&[
                clap::arg!(--release "Build release version").conflicts_with("debug"),
                clap::arg!(--debug "Build debug version (default)").conflicts_with("release"),
                clap::arg!(--arch <arch> "Target architecture")
                    .value_parser(clap::builder::EnumValueParser::<Arch>::new()),
                clap::arg!(--verbose "Print commands"),
            ]),
        )
        .subcommand(
            clap::Command::new("kasm").about("Emits r9 assembler").args(&[
                clap::arg!(--release "Build release version").conflicts_with("debug"),
                clap::arg!(--debug "Build debug version (default)").conflicts_with("release"),
                clap::arg!(--arch <arch> "Target architecture")
                    .value_parser(clap::builder::EnumValueParser::<Arch>::new()),
                clap::arg!(--verbose "Print commands"),
            ]),
        )
        .subcommand(
            clap::Command::new("dist").about("Builds a multibootable r9 image").args(&[
                clap::arg!(--release "Build a release version").conflicts_with("debug"),
                clap::arg!(--debug "Build a debug version").conflicts_with("release"),
                clap::arg!(--arch <arch> "Target architecture")
                    .value_parser(clap::builder::EnumValueParser::<Arch>::new()),
                clap::arg!(--config <name> "Configuration")
                    .value_parser(clap::builder::NonEmptyStringValueParser::new())
                    .default_value("default"),
                clap::arg!(--verbose "Print commands"),
            ]),
        )
        .subcommand(clap::Command::new("test").about("Runs unit tests").args(&[
            clap::arg!(--release "Build a release version").conflicts_with("debug"),
            clap::arg!(--debug "Build a debug version").conflicts_with("release"),
            clap::arg!(--json "Output messages as json"),
            clap::arg!(--verbose "Print commands"),
        ]))
        .subcommand(
            clap::Command::new("clippy").about("Runs clippy").args(&[
                clap::arg!(--release "Build a release version").conflicts_with("debug"),
                clap::arg!(--debug "Build a debug version").conflicts_with("release"),
                clap::arg!(--arch <arch> "Target architecture")
                    .value_parser(clap::builder::EnumValueParser::<Arch>::new()),
                clap::arg!(--config <name> "Configuration")
                    .value_parser(clap::builder::NonEmptyStringValueParser::new())
                    .default_value("default"),
                clap::arg!(--verbose "Print commands"),
            ]),
        )
        .subcommand(clap::Command::new("check").about("Runs check").args(&[
            clap::arg!(--json "Output messages as json"),
            clap::arg!(--verbose "Print commands"),
        ]))
        .subcommand(
            clap::Command::new("integration-test")
                .about("Runs the QEMU integration tests for an architecture")
                .args(&[
                    clap::arg!(--release "Build a release version").conflicts_with("debug"),
                    clap::arg!(--debug "Build a debug version").conflicts_with("release"),
                    clap::arg!(--arch <arch> "Target architecture")
                        .value_parser(clap::builder::EnumValueParser::<Arch>::new()),
                    clap::arg!(--config <name> "Configuration")
                        .value_parser(clap::builder::NonEmptyStringValueParser::new())
                        .default_value("default"),
                    clap::arg!(--timeout <secs> "Seconds before an image is considered hung")
                        .value_parser(clap::value_parser!(u64))
                        .default_value(DEFAULT_TIMEOUT_SECS),
                    clap::arg!(--verbose "Print commands"),
                ]),
        )
        .subcommand(clap::Command::new("fmt").about("Runs rustfmt over the workspace").args(&[
            clap::arg!(--check "Check formatting without rewriting files"),
            clap::arg!(--verbose "Print commands"),
        ]))
        .subcommand(
            clap::Command::new("ci")
                .about("Runs fmt, check, clippy (all arches) and test")
                .args(&[
                    clap::arg!(--release "Build a release version").conflicts_with("debug"),
                    clap::arg!(--debug "Build a debug version").conflicts_with("release"),
                    clap::arg!(--fix "Reformat in place rather than failing on badly formatted code"),
                    clap::arg!(--config <name> "Configuration")
                        .value_parser(clap::builder::NonEmptyStringValueParser::new())
                        .default_value("default"),
                    clap::arg!(--verbose "Print commands"),
                ]),
        )
        .subcommand(
            clap::Command::new("qemu").about("Run r9 under QEMU").args(&[
                clap::arg!(--release "Build a release version").conflicts_with("debug"),
                clap::arg!(--debug "Build a debug version").conflicts_with("release"),
                clap::arg!(--arch <arch> "Target architecture")
                    .value_parser(clap::builder::EnumValueParser::<Arch>::new()),
                clap::arg!(--gdb "Wait for gdb connection on start"),
                clap::arg!(--kvm "Run with KVM"),
                clap::arg!(--config <name> "Configuration")
                    .value_parser(clap::builder::NonEmptyStringValueParser::new())
                    .default_value("default"),
                clap::arg!(--verbose "Print commands"),
                clap::arg!(--dump_dtb <file> "Dump the DTB from QEMU to a file")
                    .value_parser(clap::value_parser!(String)),
                clap::arg!(--timeout <secs> "Seconds before a hung guest is killed; 0 waits indefinitely")
                    .value_parser(clap::value_parser!(u64))
                    .default_value("15"),
                clap::arg!(--image <name> "Build and run one of the arch's QEMU test images instead of the kernel")
                    .value_parser(clap::builder::NonEmptyStringValueParser::new()),
            ]),
        )
        .subcommand(clap::Command::new("clean").about("Cargo clean"))
        .get_matches();

    if let Err(e) = match matches.subcommand() {
        Some(("build", m)) => BuildStep::new(m).run(),
        Some(("expand", m)) => ExpandStep::new(m).run(),
        Some(("kasm", m)) => KasmStep::new(m).run(),
        Some(("dist", m)) => {
            let s1 = BuildStep::new(m);
            let s2 = DistStep::new(m);
            s1.run().and_then(|_| s2.run())
        }
        Some(("test", m)) => TestStep::new(m).run(),
        Some(("clippy", m)) => ClippyStep::new(m).run(),
        Some(("check", m)) => CheckStep::new(m).run(),
        Some(("integration-test", m)) => IntegrationTestStep::new(m).run(),
        Some(("fmt", m)) => FmtStep::new(m).run(),
        Some(("ci", m)) => CiStep::new(m).run(),
        Some(("qemu", m)) => {
            let s1 = BuildStep::new(m);
            let s2 = DistStep::new(m);
            let s3 = QemuStep::new(m);
            s1.run().and_then(|_| s2.run()).and_then(|_| s3.run())
        }
        Some(("clean", _)) => CleanStep::new().run(),
        _ => Err("bad subcommand".into()),
    } {
        eprintln!("{e}");
        process::exit(1);
    }
}

fn env_or(var: &str, default: &str) -> String {
    let default = default.to_string();
    env::var(var).unwrap_or(default)
}

fn cargo() -> String {
    env_or("CARGO", "cargo")
}

fn objcopy() -> String {
    if let Ok(objcopy) = env::var("OBJCOPY") {
        return objcopy;
    }

    // The llvm-tools component installs llvm-objcopy in the toolchain's
    // host rustlib directory, which is the only rustlib directory that
    // carries a bin directory at all.  Finding it there means the host
    // triple never has to be spelled out, and no host binutils or llvm
    // package is needed.
    let home = env::var("RUSTUP_HOME")
        .ok()
        .or_else(|| env::var("HOME").ok().map(|home| format!("{home}/.rustup")));
    if let (Some(home), Some(toolchain)) = (home, env::var("RUSTUP_TOOLCHAIN").ok()) {
        let rustlib =
            Path::new(&home).join("toolchains").join(&toolchain).join("lib").join("rustlib");
        if let Ok(entries) = std::fs::read_dir(rustlib) {
            for entry in entries.flatten() {
                let path = entry.path().join("bin").join("llvm-objcopy");
                if path.is_file() {
                    return path.into_os_string().into_string().unwrap();
                }
            }
        }
    }
    "llvm-objcopy".into()
}

fn load_config(arch: Arch, matches: &clap::ArgMatches) -> Configuration {
    let default = "default".to_string();
    let config_file = matches.try_get_one("config").ok().flatten().unwrap_or(&default);
    load_named_config(arch, config_file)
}

fn load_named_config(arch: Arch, name: &str) -> Configuration {
    Configuration::load(format!(
        "{}/{}/lib/config_{}.toml",
        workspace().display(),
        arch.to_string().to_lowercase(),
        name
    ))
}

fn verbose(matches: &clap::ArgMatches) -> bool {
    matches.get_flag("verbose")
}

struct BuildStep {
    arch: Arch,
    config: Configuration,
    profile: Profile,
    verbose: bool,
}

impl BuildStep {
    fn new(matches: &clap::ArgMatches) -> Self {
        let arch = Arch::from(matches);
        let config = load_config(arch, matches);
        let profile = Profile::from(matches);
        let verbose = verbose(matches);

        Self { arch, config, profile, verbose }
    }

    fn for_arch(arch: Arch, config_name: &str, profile: Profile, verbose: bool) -> Self {
        Self { arch, config: load_named_config(arch, config_name), profile, verbose }
    }

    fn run(self) -> Result<()> {
        // The aarch64 crate's build.rs stages the console server's ELF (the
        // console_server image embeds it), so the kernel-image build needs it
        // present; build the server first.  A bare build outside xtask hits the
        // build.rs's loud failure instead.
        ServerStep::new(self.arch, self.profile, self.verbose).run()?;
        let mut cmd = Command::new(cargo());
        cmd.arg("build");

        apply_to_build_step(
            &mut cmd,
            &self.config,
            &self.arch.target(),
            &self.profile,
            workspace().to_str().unwrap(),
        )?;

        cmd.current_dir(workspace());
        cmd.arg("--workspace");
        cmd.arg("--exclude").arg("xtask");
        // The servers are separate user-space executables, not part of the
        // kernel image; the ServerStep builds them (aarch64 only, where the
        // loader's per-process Aspace exists).  Excluding them here also keeps
        // the aarch64-only servers out of the other arches' image builds
        // (their syscall shims use aarch64 register names).
        cmd.arg("--exclude").arg("console");
        cmd.arg("--exclude").arg("nameserver");
        exclude_other_arches(self.arch, &mut cmd);
        if self.profile == Profile::Release {
            cmd.arg("--release");
        }
        cmd.arg("-Z").arg("build-std=core,alloc");
        cmd.arg("-Z").arg("json-target-spec");
        if self.verbose {
            println!("Executing {cmd:?}");
        }
        let status = annotated_status(&mut cmd)?;
        if !status.success() {
            return Err("build kernel failed".into());
        }
        Ok(())
    }
}

struct DistStep {
    arch: Arch,
    profile: Profile,
    verbose: bool,
}

impl DistStep {
    fn new(matches: &clap::ArgMatches) -> Self {
        let arch = Arch::from(matches);
        let profile = Profile::from(matches);
        let verbose = verbose(matches);
        Self { arch, profile, verbose }
    }

    fn for_arch(arch: Arch, profile: Profile, verbose: bool) -> Self {
        Self { arch, profile, verbose }
    }

    /// One of this arch's build artefacts, wherever cargo was told to put
    /// them.
    fn artifact(&self, name: &str) -> PathBuf {
        target_dir().join(self.arch.target()).join(self.profile.dir()).join(name)
    }

    fn run(self) -> Result<()> {
        match self.arch {
            Arch::Aarch64 => {
                // Qemu needs a flat binary in order to handle device tree files correctly
                let mut cmd = Command::new(objcopy());
                cmd.arg("-O");
                cmd.arg("binary");
                cmd.arg(self.artifact("aarch64"));
                cmd.arg(self.artifact("aarch64-qemu"));
                cmd.current_dir(workspace());
                if self.verbose {
                    println!("Executing {cmd:?}");
                }
                let status = annotated_status(&mut cmd)?;
                if !status.success() {
                    return Err("objcopy failed".into());
                }

                // Compress the binary.  We do this because they're much faster when used
                // for netbooting and qemu also accepts them.
                let mut cmd = Command::new("gzip");
                cmd.arg("-k");
                cmd.arg("-f");
                cmd.arg(self.artifact("aarch64-qemu"));
                cmd.current_dir(workspace());
                if self.verbose {
                    println!("Executing {cmd:?}");
                }
                let status = annotated_status(&mut cmd)?;
                if !status.success() {
                    return Err("gzip failed".into());
                }
            }
            Arch::X86_64 => {
                let mut cmd = Command::new(objcopy());
                cmd.arg("--input-target=elf64-x86-64");
                cmd.arg("--output-target=elf32-i386");
                cmd.arg(self.artifact("x86_64"));
                cmd.arg(self.artifact("r9.elf32"));
                cmd.current_dir(workspace());
                if self.verbose {
                    println!("Executing {cmd:?}");
                }
                let status = annotated_status(&mut cmd)?;
                if !status.success() {
                    return Err("objcopy failed".into());
                }
            }
            Arch::Riscv64 => {
                // Qemu needs a flat binary in order to handle device tree files correctly
                let mut cmd = Command::new(objcopy());
                cmd.arg("-O");
                cmd.arg("binary");
                cmd.arg(self.artifact("riscv64"));
                cmd.arg(self.artifact("riscv64-qemu"));
                cmd.current_dir(workspace());
                if self.verbose {
                    println!("Executing {cmd:?}");
                }
                let status = annotated_status(&mut cmd)?;
                if !status.success() {
                    return Err("objcopy failed".into());
                }
            }
        };

        Ok(())
    }
}

struct QemuStep {
    arch: Arch,
    config: Configuration,
    profile: Profile,
    wait_for_gdb: bool,
    kvm: bool,
    dump_dtb: String,
    verbose: bool,
    timeout_secs: u64,
    image: String,
}

impl QemuStep {
    fn new(matches: &clap::ArgMatches) -> Self {
        let arch = Arch::from(matches);
        let config = load_config(arch, matches);
        let profile = Profile::from(matches);
        let wait_for_gdb = matches.get_flag("gdb");
        let kvm = matches.get_flag("kvm");
        let dump_dtb: String = matches
            .try_get_one::<String>("dump_dtb")
            .ok()
            .flatten()
            .unwrap_or(&"".to_string())
            .clone();
        let verbose = verbose(matches);
        let timeout_secs = *matches.get_one::<u64>("timeout").expect("timeout has a default");
        let image = matches.get_one::<String>("image").cloned().unwrap_or_default();

        Self { arch, config, profile, wait_for_gdb, kvm, dump_dtb, verbose, timeout_secs, image }
    }

    /// Spawn QEMU with the console attached and wait for it to exit on
    /// its own, or for the timeout: a hung guest must not outlive the
    /// command that started it.  A timeout kills the guest and is an
    /// error; `--timeout 0` waits indefinitely (gdb sessions).
    fn run_bounded(&self, mut cmd: Command) -> Result<()> {
        if self.verbose {
            println!("Executing {cmd:?}");
        }
        let mut child = cmd.spawn().map_err(|e| format!("qemu: {e}"))?;
        let deadline = (self.timeout_secs > 0)
            .then(|| std::time::Instant::now() + Duration::from_secs(self.timeout_secs));
        loop {
            if let Some(status) = child.try_wait()? {
                // A guest that finished maps onto QEMU's exit code the
                // arch-dependent way; only the passing status is a pass.
                let code = status.code().unwrap_or(-1);
                if code == self.arch.passing_status() {
                    return Ok(());
                }
                return Err(format!("qemu failed (exit {code})").into());
            }
            if let Some(deadline) = deadline
                && std::time::Instant::now() >= deadline
            {
                child.kill()?;
                child.wait()?;
                return Err(format!("qemu timed out after {}s; killed", self.timeout_secs).into());
            }
            std::thread::sleep(Duration::from_millis(50));
        }
    }

    fn run(self) -> Result<()> {
        let out = target_dir().join(self.arch.target()).join(self.profile.dir());
        let qemu_system = self.arch.qemu_system();

        if self.kvm && self.arch != Arch::X86_64 {
            return Err("KVM only supported under x86-64".into());
        }

        // A named image is one of this arch's QEMU test images: built
        // the way the integration tests build them, so `xtask qemu`
        // stays the single bounded way to watch a guest.
        let image_path = if self.image.is_empty() {
            None
        } else {
            // A server-embedding image needs the server's ELF built first.
            ServerStep::new(self.arch, self.profile, self.verbose).run()?;
            let runner = ArchIntegrationTests {
                arch: self.arch,
                config: self.config.clone(),
                profile: self.profile,
                timeout: Duration::from_secs(self.timeout_secs),
                verbose: self.verbose,
            };
            let elf = runner.compile(&self.image)?;
            Some(runner.image(&self.image, &elf)?)
        };

        match self.arch {
            Arch::Aarch64 => {
                let mut cmd = Command::new(qemu_system);

                apply_to_qemu_step(&mut cmd, &self.config);

                // TODO Choose UART at cmdline
                cmd.arg("-nographic");

                // The PL011 (UART0, serial_hd(0)) is the early console, so it
                // lands on the terminal; the mini-UART (UART1, serial_hd(1))
                // goes to the null sink.
                cmd.arg("-serial");
                cmd.arg("mon:stdio");
                cmd.arg("-serial");
                cmd.arg("null");

                if self.wait_for_gdb {
                    cmd.arg("-s").arg("-S");
                }
                match &image_path {
                    Some(image) => {
                        // Test images leave QEMU via semihosting, and a
                        // rebooted image would just start again.
                        cmd.arg("-semihosting");
                        cmd.arg("-no-reboot");
                        cmd.arg("-kernel");
                        cmd.arg(image);
                    }
                    None => {
                        cmd.arg("-kernel");
                        cmd.arg(out.join("aarch64-qemu.gz"));
                    }
                }
                cmd.current_dir(workspace());
                if self.verbose {
                    // Show exception level change events in stdout
                    cmd.arg("-d");
                    cmd.arg("int");
                }
                self.run_bounded(cmd)
            }
            Arch::Riscv64 => {
                let mut cmd = Command::new(qemu_system);
                cmd.arg("-nographic");
                //cmd.arg("-curses");
                // cmd.arg("-bios").arg("none");
                match &image_path {
                    // The integration test images run on the bare virt
                    // machine, mirroring the harness.
                    Some(image) => {
                        cmd.arg("-machine").arg("virt");
                        cmd.arg("-cpu").arg("rv64");
                        cmd.arg("-smp").arg("4");
                        cmd.arg("-m").arg("1024M");
                        cmd.arg("-serial").arg("mon:stdio");
                        cmd.arg("-no-reboot");
                        cmd.arg("-kernel");
                        cmd.arg(image);
                    }
                    None => {
                        let dump_dtb = &self.dump_dtb;
                        if !dump_dtb.is_empty() {
                            cmd.arg("-machine").arg(format!("virt,dumpdtb={dump_dtb}"));
                        } else {
                            cmd.arg("-machine").arg("virt");
                        }
                        cmd.arg("-cpu").arg("rv64");
                        // FIXME: This is not needed as of now, and will only work once the
                        // FIXME: disk.bin is also taken care of. Doesn't exist by default.
                        if false {
                            cmd.arg("-drive").arg("file=disk.bin,format=raw,id=hd0");
                            cmd.arg("-device").arg("virtio-blk-device,drive=hd0");
                        }
                        cmd.arg("-netdev").arg("type=user,id=net0");
                        cmd.arg("-device").arg("virtio-net-device,netdev=net0");
                        cmd.arg("-smp").arg("4");
                        cmd.arg("-m").arg("1024M");
                        cmd.arg("-serial").arg("mon:stdio");
                        if self.wait_for_gdb {
                            cmd.arg("-s").arg("-S");
                        }
                        cmd.arg("-d").arg("guest_errors,unimp");
                        cmd.arg("-kernel");
                        cmd.arg(out.join("riscv64"));
                    }
                }
                cmd.current_dir(workspace());
                self.run_bounded(cmd)
            }
            Arch::X86_64 => {
                let mut cmd = Command::new(qemu_system);
                cmd.arg("-nographic");
                // cmd.arg("-display");
                // cmd.arg("curses");
                if self.kvm {
                    cmd.arg("-accel").arg("kvm");
                    cmd.arg("-cpu").arg("host,pdpe1gb,xsaveopt,fsgsbase,apic,msr");
                } else {
                    cmd.arg("-M").arg("q35");
                    cmd.arg("-cpu").arg("qemu64,pdpe1gb,xsaveopt,fsgsbase,apic,msr");
                }
                match &image_path {
                    // The integration test images exit through
                    // isa-debug-exit, mirroring the harness.
                    Some(image) => {
                        cmd.arg("-M").arg("q35");
                        cmd.arg("-cpu").arg("qemu64,pdpe1gb,xsaveopt,fsgsbase,apic,msr");
                        cmd.arg("-smp").arg("8");
                        cmd.arg("-m").arg("8192");
                        cmd.arg("-serial").arg("mon:stdio");
                        cmd.arg("-device").arg("isa-debug-exit,iobase=0xf4,iosize=0x04");
                        cmd.arg("-no-reboot");
                        cmd.arg("-kernel");
                        cmd.arg(image);
                    }
                    None => {
                        cmd.arg("-smp");
                        cmd.arg("8");
                        cmd.arg("-m");
                        cmd.arg("8192");
                        if self.wait_for_gdb {
                            cmd.arg("-s").arg("-S");
                        }
                        //cmd.arg("-device");
                        //cmd.arg("ahci,id=ahci0");
                        //cmd.arg("-drive");
                        //cmd.arg("id=sdahci0,file=sdahci0.img,if=none");
                        //cmd.arg("-device");
                        //cmd.arg("ide-hd,drive=sdahci0,bus=ahci0.0");
                        cmd.arg("-kernel");
                        cmd.arg(out.join("r9.elf32"));
                    }
                }
                cmd.current_dir(workspace());
                self.run_bounded(cmd)
            }
        }
    }
}

struct ExpandStep {
    arch: Arch,
    profile: Profile,
    verbose: bool,
}

impl ExpandStep {
    fn new(matches: &clap::ArgMatches) -> Self {
        let arch = Arch::from(matches);
        let profile = Profile::from(matches);
        let verbose = verbose(matches);

        Self { arch, profile, verbose }
    }

    fn run(self) -> Result<()> {
        let mut cmd = Command::new(cargo());
        cmd.current_dir(workspace());
        cmd.arg("rustc");
        cmd.arg("-Z").arg("build-std=core,alloc");
        cmd.arg("-p").arg(self.arch.to_string().to_lowercase());
        cmd.arg("--target").arg(format!("lib/{}.json", self.arch.target()));
        cmd.arg("--");
        cmd.arg("-Z").arg("unpretty=expanded");
        if self.profile == Profile::Release {
            cmd.arg("--release");
        }
        if self.verbose {
            println!("Executing {cmd:?}");
        }
        let status = annotated_status(&mut cmd)?;
        if !status.success() {
            return Err("build kernel failed".into());
        }
        Ok(())
    }
}

struct KasmStep {
    arch: Arch,
    profile: Profile,
    verbose: bool,
}

impl KasmStep {
    fn new(matches: &clap::ArgMatches) -> Self {
        let arch = Arch::from(matches);
        let profile = Profile::from(matches);
        let verbose = verbose(matches);

        Self { arch, profile, verbose }
    }

    fn run(self) -> Result<()> {
        let mut cmd = Command::new(cargo());
        cmd.current_dir(workspace());
        cmd.arg("rustc");
        cmd.arg("-Z").arg("build-std=core,alloc");
        cmd.arg("-p").arg(self.arch.to_string().to_lowercase());
        cmd.arg("--target").arg(format!("lib/{}.json", self.arch.target()));
        cmd.arg("--").arg("--emit").arg("asm");
        if self.profile == Profile::Release {
            cmd.arg("--release");
        }
        if self.verbose {
            println!("Executing {cmd:?}");
        }
        let status = annotated_status(&mut cmd)?;
        if !status.success() {
            return Err("build kernel failed".into());
        }
        Ok(())
    }
}

/// Run tests for the current host toolchain.
struct TestStep {
    json_output: bool,
    verbose: bool,
}

impl TestStep {
    fn new(matches: &clap::ArgMatches) -> Self {
        let json_output = matches.get_flag("json");
        let verbose = verbose(matches);

        Self { json_output, verbose }
    }

    fn run(self) -> Result<()> {
        // Tests need std, and the arch packages set a bare-metal
        // default-target that has no std, so spell the host out.
        let host = std::env::consts::ARCH;
        let rustup_state = RustupState::new()?;
        let Some(target) = rustup_state.std_supported_target(host) else {
            return Err(format!("no target with std is installed for {host}").into());
        };

        // Tests execute, so an arch package only runs natively: its inline
        // asm assembles only for its own architecture (aarch64's irq
        // masking is a case in point).  Coverage on a foreign host is
        // check's and clippy's job -- they build the arch for its
        // <arch>-unknown-linux-gnu target instead, which is why
        // rust-toolchain.toml names one for aarch64.  riscv64 and x86_64
        // have no tests today; the selection still runs whatever is added.
        let mut packages: Vec<String> = vec!["port".to_string()];
        let mut skipped = Vec::new();
        for arch in Arch::ALL {
            if arch.package() == host {
                packages.push(arch.package());
            } else {
                skipped.push(arch.package());
            }
        }
        // Loud skip: a quiet pass here reads as the whole test gate when
        // it is only the host's share of it.
        if !skipped.is_empty() {
            println!(
                "xtask: skipping {}: an arch package's tests run only on its native host",
                skipped.join(", ")
            );
        }

        // On an aarch64 host the aarch64 package's tests include the
        // console_server image, whose build.rs stages the console server's ELF
        // into OUT_DIR; build the server first so the ELF is present.  A bare
        // `cargo test` (or this step) on a clean tree would otherwise panic in
        // the build script before anything compiles.
        if host == "aarch64" {
            ServerStep::new(Arch::Aarch64, Profile::Debug, self.verbose).run()?;
        }

        for package in packages {
            let mut cmd = Command::new(cargo());
            cmd.current_dir(workspace());

            // --tests covers every package's lib unit tests, its binary's,
            // and any integration tests.  The arch binaries build for a
            // host in test mode: no_main is off under cfg(test), and the
            // boot assembly is position-independent code with no host
            // entry symbol to collide with.  The QEMU integration images
            // stay out by themselves: cargo skips a target whose
            // required-features are not requested.
            cmd.args(["test", "--package", &package, "--tests", "--target", &target.to_string()]);
            if self.json_output {
                cmd.arg("--message-format=json").arg("--quiet");
            } else if !self.verbose {
                cmd.arg("--quiet");
            }

            if self.verbose {
                println!("Executing {cmd:?}");
            }
            let status = annotated_status(&mut cmd)?;
            if !status.success() {
                return Err("test failed".into());
            }
        }
        Ok(())
    }
}

struct ClippyStep {
    arch: Arch,
    config: Configuration,
    profile: Profile,
    verbose: bool,
}

impl ClippyStep {
    fn new(matches: &clap::ArgMatches) -> Self {
        let arch = Arch::from(matches);
        let config = load_config(arch, matches);
        let profile = Profile::from(matches);
        let verbose = verbose(matches);

        Self { arch, config, profile, verbose }
    }

    fn for_arch(arch: Arch, config_name: &str, profile: Profile, verbose: bool) -> Self {
        Self { arch, config: load_named_config(arch, config_name), profile, verbose }
    }

    fn run(self) -> Result<()> {
        // A server-embedding image's build.rs stages the server's ELF; build
        // it before any lint pass so such an image finds it.
        ServerStep::new(self.arch, self.profile, self.verbose).run()?;

        // Libs and bins, linted the way the kernel is built.
        let mut cmd = self.command();
        cmd.arg("--workspace");
        exclude_other_arches(self.arch, &mut cmd);
        // The servers are aarch64-only; on the other arches they would not
        // build (their syscall shims use aarch64 register names), so they are
        // excluded there and linted only by the aarch64 clippy.
        if self.arch != Arch::Aarch64 {
            cmd.arg("--exclude").arg("console");
            cmd.arg("--exclude").arg("nameserver");
        }
        self.lint(cmd)?;

        // Tests and benches are separate targets and are not covered above.
        // port's build like any host library.
        let mut cmd = self.command();
        cmd.arg("--package").arg("port").arg("--tests").arg("--benches");
        self.lint(cmd)?;

        // The arch packages' tests need std, so they need an OS-specific
        // toolchain; where none is installed, skip them as check does.
        let package = self.arch.package();
        if let Some(target) = RustupState::new()?.std_supported_target(&package) {
            let mut cmd = self.command();
            cmd.arg("--package").arg(&package).arg("--tests").arg("--benches");
            cmd.arg("--target").arg(target.to_string());
            self.lint(cmd)?;
        } else {
            println!(
                "xtask: skipping {package} tests and benches: no installed target has std for it"
            );
        }

        // The QEMU integration test images are bare metal kernels behind a
        // feature, so nothing above reaches them: cargo skips a target whose
        // required-features are missing, and the pass just above builds for
        // a host, where a no_std kernel image cannot compile.  They are
        // named one at a time because --tests would also ask for the lib
        // unit tests, which need libtest and so need a host.
        for name in IntegrationTestStep::test_names(self.arch)? {
            let mut cmd = self.command();
            cmd.arg("--package").arg(&package);
            cmd.arg("--test").arg(&name);
            cmd.arg("--features").arg(QEMU_TEST_FEATURE);
            self.lint(cmd)?;
        }
        Ok(())
    }

    /// A clippy invocation carrying the configured cfgs and the build profile.
    fn command(&self) -> Command {
        let mut cmd = Command::new(cargo());
        cmd.arg("clippy");
        apply_to_clippy_step(&mut cmd, &self.config);
        cmd.current_dir(workspace());
        if self.profile == Profile::Release {
            cmd.arg("--release");
        }
        cmd
    }

    fn lint(&self, mut cmd: Command) -> Result<()> {
        cmd.arg("--").arg("-Dwarnings");
        if self.verbose {
            println!("Executing {cmd:?}");
        }
        if !annotated_status(&mut cmd)?.success() {
            return Err("clippy failed".into());
        }
        Ok(())
    }
}

/// Build the user-space server (`servers/console`) into a stable ELF path.
///
/// The server is a static, non-PIE, fixed-base ELF linked at
/// [`port::user::IMAGE_BASE`] with its `start` symbol as the entry — the
/// format the loader's `Image::Elf` arm reads.  It is aarch64-only for the
/// arc (the per-process `Aspace` the loader needs has only landed for
/// aarch64); the other arches' servers appear when their `Aspace` lands.
///
/// Wired ahead of the steps that build server-embedding images (the
/// integration test, a named `qemu --image`, and the per-test clippy/check
/// passes): cargo's mtime caching makes a re-run with an unchanged server a
/// no-op, so this ordering is what makes the "server before image" dependency
/// hold — the embedding image's `build.rs` reruns only when the ELF changes.
struct ServerStep {
    arch: Arch,
    profile: Profile,
    verbose: bool,
}

/// The aarch64 user-space servers, in build order: the console server (stage
/// 5) and the nameserver (stage 6).  Every consumer of a server ELF (an
/// embedding image's `build.rs`, the gate) stages and tracks these paths; each
/// is built for every aarch64 build (a no-op for an image that does not embed
/// it).
const SERVERS: [&str; 2] = ["console", "nameserver"];

impl ServerStep {
    fn new(arch: Arch, profile: Profile, verbose: bool) -> Self {
        Self { arch, profile, verbose }
    }

    /// The staged ELF for `server`, or none if this arch has no servers.
    fn elf(&self, server: &str) -> Option<PathBuf> {
        (self.arch == Arch::Aarch64).then(|| {
            target_dir()
                .join(self.arch.target())
                .join(self.profile.dir())
                .join(format!("{server}.elf"))
        })
    }

    fn run(self) -> Result<()> {
        for server in SERVERS {
            let Some(elf) = self.elf(server) else {
                return Ok(());
            };
            let mut cmd = Command::new(cargo());
            cmd.current_dir(workspace());
            cmd.arg("build");
            cmd.arg("-p").arg(server);
            cmd.arg("--target").arg(format!("lib/{}.json", self.arch.target()));
            if self.profile == Profile::Release {
                cmd.arg("--release");
            }
            cmd.arg("-Z").arg("build-std=core");
            cmd.arg("-Z").arg("json-target-spec");
            // The user-binary format: static, non-PIE, fixed-base.  The base is
            // the shared `port::user::IMAGE_BASE` the loader's placement check
            // reads, so build and loader cannot drift.  `--image-base` sets the
            // fixed base; `-e start` names the entry symbol, so the ELF's
            // `e_entry` is the server's `start`.
            let base = format!("0x{:x}", port::user::IMAGE_BASE);
            let flags = [
                "-Crelocation-model=static",
                &format!("-Clink-arg=--image-base={base}"),
                "-Clink-arg=-estart",
            ];
            cmd.arg("--config").arg(format!(
                "build.rustflags=[{}]",
                flags.iter().map(|f| config::toml_basic_string(f)).collect::<Vec<_>>().join(",")
            ));
            if self.verbose {
                println!("Executing {cmd:?}");
            }
            let status = annotated_status(&mut cmd)?;
            if !status.success() {
                return Err(format!("build server {server} failed").into());
            }
            // The built bin (named after the package) is the ELF.  Stage it at
            // the stable, extensioned name above, but only when it is newer
            // than the staged copy (or the copy is absent): a re-run with an
            // unchanged server then leaves the staged ELF's mtime alone, so the
            // embedding image is not rebuilt needlessly.
            let built = elf.with_file_name(server);
            let newer = |p: &Path| p.metadata().and_then(|m| m.modified()).ok();
            let restage = match (newer(&built), newer(&elf)) {
                (Some(b), Some(e)) => b > e,
                _ => true,
            };
            if restage {
                std::fs::copy(&built, &elf).map_err(|e| format!("stage {elf:?}: {e}"))?;
            }
        }
        Ok(())
    }
}

/// Run check for all packages for all relevant toolchains.
/// This assumes that the <arch>-unknown-linux-gnu toolchain has been installed
/// for any arch we care about.
struct CheckStep {
    json_output: bool,
    verbose: bool,
}

impl CheckStep {
    fn new(matches: &clap::ArgMatches) -> Self {
        let json_output = matches.get_flag("json");
        let verbose = verbose(matches);

        Self { json_output, verbose }
    }

    fn run(self) -> Result<()> {
        // To run check for bins and lib we use the default toolchain, which has
        // been set to the OS-independent arch toolchain in each Cargo.toml file.
        // The same applies to tests and benches for non-arch-specific lib packages.
        let mut bins_lib_package_cmd_args: Vec<Vec<String>> = Arch::ALL
            .iter()
            .map(|arch| {
                vec![
                    "check".to_string(),
                    "--package".to_string(),
                    arch.package(),
                    "--bins".to_string(),
                ]
            })
            .collect();
        bins_lib_package_cmd_args.push(vec![
            "check".to_string(),
            "--package".to_string(),
            "port".to_string(),
            "--lib".to_string(),
            "--tests".to_string(),
            "--benches".to_string(),
        ]);

        let rustup_state = RustupState::new()?;

        // However, running check for tests and benches in arch packages requires
        // that we use a toolchain with `std`, so we need an OS-specific toolchain.
        // If the arch matches that of the current toolchain, then that will be used
        // for check.  Otherwise we'll always default to <arch>-unknown-linux-gnu.
        let mut benches_tests_package_cmd_args = Vec::new();

        for arch in Arch::ALL {
            let package = arch.package();
            let Some(target) = rustup_state.std_supported_target(&package) else {
                // Loud skip: this is the only compile signal the arch's
                // test code gets on a foreign host, so dropping it
                // silently would let a broken test stay green everywhere.
                println!(
                    "xtask: skipping {package} tests and benches: no installed target has std \
                     for it"
                );
                continue;
            };

            benches_tests_package_cmd_args.push(vec![
                "check".to_string(),
                "--package".to_string(),
                package,
                "--tests".to_string(),
                "--benches".to_string(),
                "--target".to_string(),
                target.to_string(),
            ]);
        }

        // The QEMU integration test images are bare metal kernels behind a
        // feature, so neither list above reaches them: cargo skips a target
        // whose required-features are missing, and the std toolchain the
        // tests and benches use cannot build a no_std image.  They are named
        // one at a time because --tests would also ask for the lib unit
        // tests, which need libtest and so need that host toolchain.
        let mut qemu_test_cmd_args = Vec::new();

        for arch in Arch::ALL {
            for name in IntegrationTestStep::test_names(arch)? {
                qemu_test_cmd_args.push(vec![
                    "check".to_string(),
                    "--package".to_string(),
                    arch.to_string().to_lowercase(),
                    "--test".to_string(),
                    name,
                    "--features".to_string(),
                    QEMU_TEST_FEATURE.to_string(),
                ]);
            }
        }

        // A server-embedding image's build.rs stages the server's ELF; build
        // it before the per-test check passes so such an image finds it.
        for arch in Arch::ALL {
            ServerStep::new(arch, Profile::Debug, self.verbose).run()?;
        }

        for cmd_args in
            [bins_lib_package_cmd_args, benches_tests_package_cmd_args, qemu_test_cmd_args].concat()
        {
            let mut cmd = Command::new(cargo());
            cmd.args(cmd_args);
            if self.json_output {
                cmd.arg("--message-format=json").arg("--quiet");
            }
            cmd.current_dir(workspace());

            if self.verbose {
                println!("Executing {cmd:?}");
            }
            let status = annotated_status(&mut cmd)?;
            if !status.success() {
                return Err("check failed".into());
            }
        }
        Ok(())
    }
}

/// Run rustfmt over every package in the workspace.
struct FmtStep {
    check: bool,
    verbose: bool,
}

impl FmtStep {
    fn new(matches: &clap::ArgMatches) -> Self {
        let check = matches.get_flag("check");
        let verbose = verbose(matches);

        Self { check, verbose }
    }

    fn run(self) -> Result<()> {
        let mut cmd = Command::new(cargo());
        cmd.current_dir(workspace());
        cmd.arg("fmt").arg("--all");
        if self.check {
            cmd.arg("--check");
        }
        if self.verbose {
            println!("Executing {cmd:?}");
        }
        let status = annotated_status(&mut cmd)?;
        if !status.success() {
            return Err("fmt failed".into());
        }
        Ok(())
    }
}

/// Seconds an integration test image may run before it is taken as hung.
/// Shared between the command line default and the ci step so the two
/// cannot drift.
const DEFAULT_TIMEOUT_SECS: &str = "60";

/// Build each integration test as a kernel image and run it under QEMU.
///
/// Every test in `<arch>/tests` is a whole kernel: it links the arch
/// library, supplies its own `main9`, runs the initialisation it needs and
/// leaves QEMU with an exit status.  Zero is a pass.
struct IntegrationTestStep {
    arches: Vec<Arch>,
    config_name: String,
    profile: Profile,
    timeout: Duration,
    verbose: bool,
}

impl IntegrationTestStep {
    fn new(matches: &clap::ArgMatches) -> Self {
        // No --arch means every architecture, so that the bare command
        // runs everything there is to run.
        let arches =
            matches.get_one::<Arch>("arch").map_or_else(|| Arch::ALL.to_vec(), |&arch| vec![arch]);
        let config_name =
            matches.get_one::<String>("config").expect("config has a default").clone();
        let profile = Profile::from(matches);
        let timeout =
            Duration::from_secs(*matches.get_one::<u64>("timeout").expect("timeout has a default"));
        let verbose = verbose(matches);

        Self { arches, config_name, profile, timeout, verbose }
    }

    fn for_ci(config_name: &str, profile: Profile, verbose: bool) -> Self {
        Self {
            arches: Arch::ALL.to_vec(),
            config_name: config_name.to_string(),
            profile,
            timeout: Duration::from_secs(
                DEFAULT_TIMEOUT_SECS.parse().expect("default timeout is a number"),
            ),
            verbose,
        }
    }

    fn run(self) -> Result<()> {
        let mut ran = 0;
        let failed: std::sync::Mutex<Vec<String>> = std::sync::Mutex::new(Vec::new());
        let mut undeclared = Vec::new();
        for &arch in &self.arches {
            let tests = Self::test_names(arch)?;
            let all_stanzas = Self::all_test_stanzas(arch)?;
            for (name, no_stanza) in Self::undeclared_images(arch, &all_stanzas)? {
                if no_stanza {
                    println!("{arch}: tests/{name}.rs has no [[test]] entry, so nothing builds it");
                } else {
                    println!(
                        "{arch}: tests/{name}.rs has a [[test]] entry, but is missing the {QEMU_TEST_FEATURE} feature"
                    );
                }
                undeclared.push(format!("{arch} {name}"));
            }
            if tests.is_empty() {
                // An architecture with no tests is a fact to report, not a
                // failure -- but never call it a pass.
                println!("{arch}: no integration tests");
                continue;
            }

            // A server-embedding image needs the server's ELF, built before it
            // compiles (cargo's mtime caching makes an unchanged server a
            // no-op).
            ServerStep::new(arch, self.profile, self.verbose).run()?;

            let runner = ArchIntegrationTests {
                arch,
                config: load_named_config(arch, &self.config_name),
                profile: self.profile,
                timeout: self.timeout,
                verbose: self.verbose,
            };

            // Phase 1: compile all test images (serial — cargo's build lock
            // serializes them regardless).
            let mut images: Vec<(String, PathBuf)> = Vec::new();
            for name in &tests {
                ran += 1;
                let elf = match runner.compile(name) {
                    Ok(elf) => elf,
                    Err(err) => {
                        println!("{arch} {name}: FAILED ({err})");
                        failed.lock().unwrap().push(format!("{arch} {name}"));
                        continue;
                    }
                };
                let image = runner.image(name, &elf)?;
                images.push((name.clone(), image));
            }

            // Phase 2: run QEMU instances in parallel.  Each instance is an
            // isolated process with its own serial pipe; the only shared
            // resource is CPU, so bound concurrency to the core count.
            let n = std::thread::available_parallelism().map(|n| n.get()).unwrap_or(4).min(4);
            let runner = &runner;
            let failed = &failed;
            for chunk in images.chunks(n) {
                std::thread::scope(|s| {
                    for (name, image) in chunk {
                        s.spawn(move || {
                            let result = runner.qemu(image);
                            match result {
                                Ok(Some(code)) if code == arch.passing_status() => {
                                    if self.verbose {
                                        println!("{arch} {name}: ok")
                                    }
                                }
                                Ok(Some(code)) => {
                                    println!("{arch} {name}: FAILED (exit {code})");
                                    failed.lock().unwrap().push(format!("{arch} {name}"));
                                }
                                Ok(None) => {
                                    println!(
                                        "{arch} {name}: TIMED OUT after {}s",
                                        self.timeout.as_secs()
                                    );
                                    failed.lock().unwrap().push(format!("{arch} {name}"));
                                }
                                Err(e) => {
                                    println!("{arch} {name}: FAILED ({e})");
                                    failed.lock().unwrap().push(format!("{arch} {name}"));
                                }
                            }
                        });
                    }
                });
            }
        }

        // Having nothing to run is not a failure.  Naming a single arch is
        // the documented way to run one, and two of the three have no
        // images, so reporting the fact is the whole answer -- the loop
        // above has already said so per arch.
        let failed = failed.into_inner().unwrap();
        if ran > 0 {
            println!("\n{} of {ran} passed", ran - failed.len());
        }

        // An undeclared image is a test that nothing compiles and nothing
        // runs, which is worse than one that fails: it looks like it
        // passed.  Say so in the exit status, not only in the log.
        let mut problems = Vec::new();
        if !failed.is_empty() {
            problems.push(format!("failed: {}", failed.join(", ")));
        }
        if !undeclared.is_empty() {
            problems.push(format!("no [[test]] entry: {}", undeclared.join(", ")));
        }
        if problems.is_empty() { Ok(()) } else { Err(problems.join("; ").into()) }
    }

    /// Every test target in the arch's manifest that asks for the
    /// [`QEMU_TEST_FEATURE`] is one test image.
    ///
    /// Cargo builds what the manifest declares, so reading it is the only
    /// way to agree with cargo about what there is to run: listing tests/
    /// instead would count a file cargo does not build, and miss the
    /// feature that says which targets are kernels rather than ordinary
    /// test binaries.
    fn all_test_stanzas(arch: Arch) -> Result<Vec<ManifestTest>> {
        let manifest_path = workspace().join(arch.to_string().to_lowercase()).join("Cargo.toml");
        let manifest = std::fs::read_to_string(&manifest_path)
            .map_err(|e| format!("{}: {e}", manifest_path.display()))?;
        let manifest: ArchManifest =
            toml::from_str(&manifest).map_err(|e| format!("{}: {e}", manifest_path.display()))?;

        Ok(manifest.test)
    }

    /// Every test target in the arch's manifest that asks for the
    /// [`QEMU_TEST_FEATURE`] is one test image.
    fn test_names(arch: Arch) -> Result<Vec<String>> {
        let mut names: Vec<String> = Self::all_test_stanzas(arch)?
            .into_iter()
            .filter(|test| test.required_features.iter().any(|f| f == QEMU_TEST_FEATURE))
            .map(|test| test.name)
            .collect();
        names.sort();
        Ok(names)
    }

    /// Files directly in `tests/` that no manifest entry claims.
    ///
    /// Reading the manifest means cargo and this agree on what to run, but
    /// it also means a forgotten `[[test]]` stanza is a test that silently
    /// never runs.  Report those.  Shared helpers belong in a subdirectory
    /// of `tests/`, which cargo does not treat as a target and this does
    /// not look into.
    fn undeclared_images(arch: Arch, all_stanzas: &[ManifestTest]) -> Result<Vec<(String, bool)>> {
        let dir = workspace().join(arch.to_string().to_lowercase()).join("tests");
        if !dir.is_dir() {
            return Ok(Vec::new());
        }
        let mut results = Vec::new();
        for entry in std::fs::read_dir(dir)? {
            let entry = entry?;
            let path = entry.path();
            if !path.extension().is_some_and(|e| e == "rs") {
                continue;
            }
            let stem = path.file_stem().unwrap().to_string_lossy().into_owned();
            match all_stanzas.iter().find(|t| t.name == stem) {
                None => {
                    results.push((stem, true)); // No stanza at all
                }
                Some(test) if !test.required_features.iter().any(|f| f == QEMU_TEST_FEATURE) => {
                    results.push((stem, false)); // Stanza exists but missing feature
                }
                _ => {}
            }
        }
        results.sort_by(|a, b| a.0.cmp(&b.0));
        Ok(results)
    }
}

/// The feature that builds an arch library so its test images can leave
/// QEMU with a status, and that marks those targets in the manifest.
const QEMU_TEST_FEATURE: &str = "qemu-test";

/// An arch's Cargo.toml, cut down to the test targets.
#[derive(serde::Deserialize)]
struct ArchManifest {
    #[serde(default)]
    test: Vec<ManifestTest>,
}

#[derive(serde::Deserialize)]
struct ManifestTest {
    name: String,
    #[serde(default, rename = "required-features")]
    required_features: Vec<String>,
}

/// Building and running one architecture's test images.
struct ArchIntegrationTests {
    arch: Arch,
    config: Configuration,
    profile: Profile,
    timeout: Duration,
    verbose: bool,
}

impl ArchIntegrationTests {
    fn package(&self) -> String {
        self.arch.to_string().to_lowercase()
    }

    /// Compile one test image, returning the ELF cargo produced.
    ///
    /// Kept apart from [`Self::image`] because only this part can fail on
    /// account of the test itself.
    fn compile(&self, name: &str) -> Result<PathBuf> {
        let mut cmd = Command::new(cargo());
        cmd.arg("build");
        apply_to_build_step(
            &mut cmd,
            &self.config,
            &self.arch.target(),
            &self.profile,
            workspace().to_str().unwrap(),
        )?;
        cmd.current_dir(workspace());
        cmd.arg("--package").arg(self.package());
        cmd.arg("--test").arg(name);
        cmd.arg("--features").arg(QEMU_TEST_FEATURE);
        if self.profile == Profile::Release {
            cmd.arg("--release");
        }
        cmd.arg("-Z").arg("build-std=core,alloc");
        cmd.arg("-Z").arg("json-target-spec");
        // Ask cargo which file it built.  Diagnostics still render to
        // stderr, so only the machine readable part is captured.
        cmd.arg("--message-format=json-render-diagnostics");
        if self.verbose {
            println!("Executing {cmd:?}");
        }
        built_test_binary(&mut cmd, name)
    }

    /// Turn a compiled test into an image QEMU will boot, the same way
    /// DistStep does for the kernel.
    ///
    /// Everything here is a host tool.  If one is missing or broken it is
    /// missing for every image, so a failure says nothing about the test.
    fn image(&self, name: &str, elf: &Path) -> Result<PathBuf> {
        // Each arch is booted the way its own qemu step boots the kernel.
        let out = target_dir().join(self.arch.target()).join(self.profile.dir());

        match self.arch {
            // riscv64's qemu loads the ELF directly, so there is nothing
            // to prepare.
            Arch::Riscv64 => return Ok(elf.to_path_buf()),
            // x86-64 boots a multiboot elf32, exactly as for the kernel.
            Arch::X86_64 => {
                let elf32 = out.join(format!("{name}.elf32"));
                let mut cmd = Command::new(objcopy());
                cmd.arg("--input-target=elf64-x86-64");
                cmd.arg("--output-target=elf32-i386");
                cmd.arg(elf).arg(&elf32);
                if !annotated_status(&mut cmd)?.success() {
                    return Err("objcopy failed".into());
                }
                return Ok(elf32);
            }
            Arch::Aarch64 => {}
        }

        // QEMU needs a flat binary to handle the device tree correctly,
        // and takes it gzipped, exactly as for the kernel.
        let flat = out.join(format!("{name}-qemu"));
        let mut cmd = Command::new(objcopy());
        cmd.arg("-O").arg("binary").arg(elf).arg(&flat);
        if !annotated_status(&mut cmd)?.success() {
            return Err("objcopy failed".into());
        }
        let mut cmd = Command::new("gzip");
        cmd.arg("-k").arg("-f").arg(&flat);
        if !annotated_status(&mut cmd)?.success() {
            return Err("gzip failed".into());
        }
        Ok(out.join(format!("{name}-qemu.gz")))
    }

    /// Run an image, returning its exit code, or None if it outstayed the
    /// timeout.  A kernel that hangs before reaching a test has to fail the
    /// run rather than block it.
    fn qemu(&self, image: &Path) -> Result<Option<i32>> {
        let mut cmd = Command::new(self.arch.qemu_system());
        cmd.arg("-nographic");
        match self.arch {
            Arch::Aarch64 => {
                apply_to_qemu_step(&mut cmd, &self.config);
                // PL011 (serial_hd(0)) to the terminal, mini-UART to null.
                cmd.arg("-serial").arg("mon:stdio");
                cmd.arg("-serial").arg("null");
                // Semihosting is how the test leaves QEMU with a status.
                cmd.arg("-semihosting");
            }
            // The finisher the test writes to is part of the virt machine,
            // so nothing has to be added for it.
            Arch::Riscv64 => {
                cmd.arg("-machine").arg("virt");
                cmd.arg("-cpu").arg("rv64");
                cmd.arg("-smp").arg("4");
                cmd.arg("-m").arg("1024M");
                cmd.arg("-serial").arg("mon:stdio");
            }
            Arch::X86_64 => {
                cmd.arg("-M").arg("q35");
                cmd.arg("-cpu").arg("qemu64,pdpe1gb,xsaveopt,fsgsbase,apic,msr");
                cmd.arg("-smp").arg("8");
                cmd.arg("-m").arg("8192");
                cmd.arg("-serial").arg("mon:stdio");
                // How the test leaves QEMU with a status at all.  The
                // guest writes to iobase; QEMU exits (value << 1) | 1.
                cmd.arg("-device").arg("isa-debug-exit,iobase=0xf4,iosize=0x04");
            }
        }
        cmd.arg("-no-reboot");
        cmd.arg("-kernel").arg(image);
        cmd.stdin(process::Stdio::null());
        if self.verbose {
            cmd.stdout(process::Stdio::piped());
            cmd.stderr(process::Stdio::piped());
        } else {
            cmd.stdout(process::Stdio::null());
            cmd.stderr(process::Stdio::null());
        }
        cmd.current_dir(workspace());
        if self.verbose {
            println!("Executing {cmd:?}");
        }

        let mut child = cmd.spawn().map_err(|e| format!("{}: {e}", self.arch.qemu_system()))?;
        // One drain thread per stream: reading them in sequence would let
        // the undrained pipe fill and block QEMU mid-write, turning a
        // passing test into a timeout.
        let mut drains = Vec::new();
        if self.verbose {
            let stdout = child.stdout.take().expect("stdout was piped");
            let stderr = child.stderr.take().expect("stderr was piped");
            drains.push(std::thread::spawn(move || Self::filter_and_print(stdout)));
            drains.push(std::thread::spawn(move || Self::filter_and_print(stderr)));
        }
        let deadline = std::time::Instant::now() + self.timeout;
        let code = loop {
            if let Some(status) = child.try_wait()? {
                break Some(status.code().unwrap_or(-1));
            }
            if std::time::Instant::now() >= deadline {
                child.kill()?;
                child.wait()?;
                break None;
            }
            std::thread::sleep(Duration::from_millis(50));
        };
        // The child has exited, so the streams are at EOF; join so the
        // tail of its output lands before the result is reported.
        for drain in drains {
            let _ = drain.join();
        }
        Ok(code)
    }

    /// Reads a stream and prints it to stdout, but filters out
    /// terminal reset and clear-screen sequences.
    fn filter_and_print(stream: impl std::io::Read) {
        use std::io::{BufReader, Read, Write};
        let mut reader = BufReader::new(stream);
        let mut buffer = [0u8; 1024];
        let mut carry = Vec::new();
        loop {
            match reader.read(&mut buffer) {
                Ok(0) | Err(_) => break,
                Ok(n) => {
                    let mut data = std::mem::take(&mut carry);
                    data.extend_from_slice(&buffer[..n]);
                    let mut output = Vec::new();
                    let mut i = 0;
                    while i < data.len() {
                        if data[i] == b'\x1b' {
                            if i + 1 < data.len() && data[i + 1] == b'[' {
                                let mut j = i + 2;
                                while j < data.len() && (data[j] < 0x40) {
                                    j += 1;
                                }
                                if j < data.len() {
                                    j += 1; // consume the final byte (0x40-0x7E)
                                    i = j;
                                } else {
                                    carry.extend_from_slice(&data[i..]);
                                    i = data.len();
                                }
                            } else if i + 1 < data.len() && data[i + 1] == b'c' {
                                i += 2;
                            } else if i + 1 < data.len() {
                                output.push(data[i]);
                                i += 1;
                            } else {
                                carry.push(data[i]);
                                i += 1;
                            }
                        } else {
                            output.push(data[i]);
                            i += 1;
                        }
                    }
                    let _ = std::io::stdout().write_all(&output);
                }
            }
        }
    }
}

/// Run `cmd`, a cargo build emitting JSON messages, and return the path it
/// reports for the `name` test binary.
///
/// Only cargo knows which `<name>-<hash>` in deps/ belongs to this build.
/// A different config, profile or feature set leaves another hash beside
/// it, and a build cargo finds fresh does not touch the file, so neither
/// the name nor the timestamp picks the image that was asked for.
///
/// The messages are matched as text rather than parsed, to keep a JSON
/// library out of the build tool for three fields.  Cargo writes them
/// compactly and their names are documented; if that ever stops holding
/// this finds no executable and the build fails saying so, which is the
/// one behaviour worth protecting -- it cannot quietly pick a different
/// image, which is the reason for asking cargo at all.
fn built_test_binary(cmd: &mut Command, name: &str) -> Result<PathBuf> {
    cmd.stdout(process::Stdio::piped());
    let mut child =
        cmd.spawn().map_err(|e| format!("{}: {e}", cmd.get_program().to_string_lossy()))?;
    let stdout = child.stdout.take().expect("stdout is piped");

    let mut executable = None;
    let mut read_error = None;
    for line in BufReader::new(stdout).lines() {
        // Returning from here would leave cargo running with nobody to
        // wait for it, and stopping the read without stopping cargo would
        // block it on a pipe nobody is draining.  Take the error out to
        // where the child can be dealt with.
        let line = match line {
            Ok(line) => line,
            Err(err) => {
                read_error = Some(err);
                break;
            }
        };
        // Everything build-std compiles reports itself here too, and so do
        // build scripts, which have an executable of their own.  Only the
        // test target answers to all three.
        if line.contains(r#""reason":"compiler-artifact""#)
            && line.contains(&format!(r#""name":"{name}""#))
            && line.contains(r#""kind":["test"]"#)
            && let Some(path) = json_string_field(&line, "executable")
        {
            executable = Some(PathBuf::from(path));
        }
    }

    if let Some(err) = read_error {
        // Stop cargo rather than wait on a build whose output is no longer
        // being read.
        let _ = child.kill();
        let _ = child.wait();
        return Err(format!("reading cargo's output for {name}: {err}").into());
    }

    if !child.wait()?.success() {
        return Err(format!("building test {name} failed").into());
    }
    executable.ok_or_else(|| format!("cargo built no test binary for {name}").into())
}

/// The value of a `"key":"value"` string field in one of cargo's JSON
/// messages, or None if the key is absent or does not hold a string.
///
/// Understands the escapes a path can arrive with.  A path holding a
/// character JSON writes as `\n` or `\uXXXX` would come back wrong, and
/// then fail at objcopy rather than boot anything.
fn json_string_field(line: &str, key: &str) -> Option<String> {
    let opening = format!("\"{key}\":\"");
    let mut chars = line[line.find(&opening)? + opening.len()..].chars();
    let mut value = String::new();
    while let Some(c) = chars.next() {
        match c {
            '"' => return Some(value),
            '\\' => value.push(chars.next()?),
            _ => value.push(c),
        }
    }
    None
}

/// Run every gate the tree is expected to pass: formatting, check across all
/// arches, clippy for each arch in turn, the unit tests, and the QEMU
/// integration tests.  The last of these needs qemu on the path.
struct CiStep {
    config_name: String,
    profile: Profile,
    fix: bool,
    verbose: bool,
}

impl CiStep {
    fn new(matches: &clap::ArgMatches) -> Self {
        let config_name =
            matches.get_one::<String>("config").expect("config has a default").clone();
        let profile = Profile::from(matches);
        let fix = matches.get_flag("fix");
        let verbose = verbose(matches);

        Self { config_name, profile, fix, verbose }
    }

    fn run(self) -> Result<()> {
        heading("fmt");
        FmtStep { check: !self.fix, verbose: self.verbose }.run()?;

        heading("check");
        CheckStep { json_output: false, verbose: self.verbose }.run()?;

        for arch in Arch::ALL {
            heading(&format!("clippy {arch}"));
            ClippyStep::for_arch(arch, &self.config_name, self.profile, self.verbose).run()?;
        }

        heading(&format!("test (host {})", std::env::consts::ARCH));
        TestStep { json_output: false, verbose: self.verbose }.run()?;

        // Everything above stops at metadata or at a test binary, so none
        // of it links a kernel.  Building the image is the only thing that
        // says the linker script still resolves and the entry point is
        // still reachable.
        for arch in Arch::ALL {
            heading(&format!("dist {arch}"));
            BuildStep::for_arch(arch, &self.config_name, self.profile, self.verbose).run()?;
            DistStep::for_arch(arch, self.profile, self.verbose).run()?;
        }

        heading("integration-test");
        IntegrationTestStep::for_ci(&self.config_name, self.profile, self.verbose).run()?;

        heading("ok");
        Ok(())
    }
}

fn heading(step: &str) {
    println!("\nxtask: {step}");
}

struct CleanStep {}

impl CleanStep {
    fn new() -> Self {
        Self {}
    }

    fn run(self) -> Result<()> {
        let mut cmd = Command::new(cargo());
        cmd.current_dir(workspace());
        cmd.arg("clean");
        let status = annotated_status(&mut cmd)?;
        if !status.success() {
            return Err("clean failed".into());
        }
        Ok(())
    }
}

fn workspace() -> PathBuf {
    Path::new(&env!("CARGO_MANIFEST_DIR")).ancestors().nth(1).unwrap().to_path_buf()
}

/// Where cargo writes build artefacts.
///
/// Composing `target/...` by hand instead looks in the wrong place the
/// moment CARGO_TARGET_DIR is set, and the failure is an objcopy that
/// cannot find a file cargo says it just built.  A relative value is
/// relative to the directory cargo runs in, which for every command here
/// is the workspace.
///
/// Only the environment variable is honoured; `build.target-dir` in a
/// cargo config file is not read.
fn target_dir() -> PathBuf {
    match env::var("CARGO_TARGET_DIR") {
        Ok(dir) if !dir.is_empty() => workspace().join(dir),
        _ => workspace().join("target"),
    }
}

/// Exclude architectures other than the one being built
fn exclude_other_arches(arch: Arch, cmd: &mut Command) {
    for other in Arch::ALL.iter().filter(|&&other| other != arch) {
        cmd.arg("--exclude").arg(other.package());
    }
}

/// Annotates the error result with the calling binary's name.
fn annotated_status(cmd: &mut Command) -> Result<process::ExitStatus> {
    Ok(cmd.status().map_err(|e| format!("{}: {}", cmd.get_program().to_string_lossy(), e))?)
}
