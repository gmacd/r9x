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
    curr_toolchain: String,
}

impl RustupState {
    /// Runs rustup command to get a list of all installed toolchains.
    /// Also caches the current toolchain.
    fn new() -> Self {
        Self {
            installed_targets: Self::installed_rustup_targets().unwrap(),
            curr_toolchain: env::var("RUSTUP_TOOLCHAIN").unwrap(),
        }
    }

    /// Call `rustup target list --installed` to get all installed target triples
    fn installed_rustup_targets() -> Result<Vec<Triple>> {
        let output =
            Command::new("rustup").arg("target").arg("list").arg("--installed").output()?;
        if !output.status.success() {
            return Err(String::from_utf8(output.stdout.clone())?.into());
        }

        Ok(String::from_utf8(output.stdout.clone())?.lines().flat_map(Triple::from_str).collect())
    }

    /// For the given arch, return a compatible toolchain triple that is
    /// installed and can be used by cargo check.  It will prefer the default
    /// toolchain if it's a match, otherwise it will look for the
    /// <arch-unknown-linux-gnu> toolchain.
    fn std_supported_target(&self, arch: &str) -> Option<&Triple> {
        let arch = Self::target_arch(arch);
        self.installed_targets.iter().filter(|&t| t.architecture.to_string() == arch).find(|&t| {
            self.curr_toolchain.ends_with(&t.to_string())
                || t.to_string() == arch.to_owned() + "-unknown-linux-gnu"
        })
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
    let llvm_objcopy = {
        let toolchain = env_or("RUSTUP_TOOLCHAIN", "nightly-x86_64-unknown-none");

        // find host architecture by taking last 3 segments from toolchain
        let mut arch_segments: Box<[_]> = toolchain.split('-').rev().take(3).collect();
        arch_segments.reverse();
        let host = arch_segments.join("-");

        let home = env_or("RUSTUP_HOME", "");
        let mut path = PathBuf::from(home);
        path.push("toolchains");
        path.push(toolchain);
        path.push("lib");
        path.push("rustlib");
        path.push(host);
        path.push("bin");
        path.push("llvm-objcopy");
        if path.exists() {
            path.into_os_string().into_string().unwrap()
        } else {
            "llvm-objcopy".into()
        }
    };
    env_or("OBJCOPY", &llvm_objcopy)
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
        let mut cmd = Command::new(cargo());
        cmd.arg("build");

        apply_to_build_step(
            &mut cmd,
            &self.config,
            &self.arch.target(),
            &self.profile,
            workspace().to_str().unwrap(),
        );

        cmd.current_dir(workspace());
        cmd.arg("--workspace");
        cmd.arg("--exclude").arg("xtask");
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

        Self { arch, config, profile, wait_for_gdb, kvm, dump_dtb, verbose }
    }

    fn run(self) -> Result<()> {
        let out = target_dir().join(self.arch.target()).join(self.profile.dir());
        let qemu_system = self.arch.qemu_system();

        if self.kvm && self.arch != Arch::X86_64 {
            return Err("KVM only supported under x86-64".into());
        }

        match self.arch {
            Arch::Aarch64 => {
                let mut cmd = Command::new(qemu_system);

                apply_to_qemu_step(&mut cmd, &self.config);

                // TODO Choose UART at cmdline
                // If using UART0 (PL011), this enables serial
                cmd.arg("-nographic");

                // If using UART1 (MiniUART), this enables serial
                cmd.arg("-serial");
                cmd.arg("null");
                cmd.arg("-serial");
                cmd.arg("mon:stdio");

                if self.wait_for_gdb {
                    cmd.arg("-s").arg("-S");
                }
                cmd.arg("-kernel");
                cmd.arg(out.join("aarch64-qemu.gz"));
                cmd.current_dir(workspace());
                if self.verbose {
                    // Show exception level change events in stdout
                    cmd.arg("-d");
                    cmd.arg("int");

                    println!("Executing {cmd:?}");
                }
                let status = annotated_status(&mut cmd)?;
                if !status.success() {
                    return Err("qemu failed".into());
                }
            }
            Arch::Riscv64 => {
                let mut cmd = Command::new(qemu_system);
                cmd.arg("-nographic");
                //cmd.arg("-curses");
                // cmd.arg("-bios").arg("none");
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
                cmd.current_dir(workspace());
                if self.verbose {
                    println!("Executing {cmd:?}");
                }
                let status = annotated_status(&mut cmd)?;
                if !status.success() {
                    return Err("qemu failed".into());
                }
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
                cmd.arg("-smp");
                cmd.arg("8");
                cmd.arg("-s");
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
                cmd.current_dir(workspace());
                if self.verbose {
                    println!("Executing {cmd:?}");
                }
                let status = annotated_status(&mut cmd)?;
                if !status.success() {
                    return Err("qemu failed".into());
                }
            }
        };

        Ok(())
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
        let rustup_state = RustupState::new();
        let Some(target) = rustup_state.std_supported_target(host) else {
            return Err(format!("no target with std is installed for {host}").into());
        };

        // port and aarch64 build for any host, so their tests run
        // everywhere.  riscv64 and x86_64 build only for their own
        // architecture -- their inline asm, and riscv64's sbi-rt dependency,
        // do not assemble elsewhere -- so run them only natively.  Neither
        // has any tests today.
        let mut packages = vec!["port", "aarch64"];
        if ["riscv64", "x86_64"].contains(&host) {
            packages.push(host);
        }

        for package in packages {
            let mut cmd = Command::new(cargo());
            cmd.current_dir(workspace());

            // What there is to test differs by package.  port is a host
            // library with integration tests, which --tests picks up and
            // --lib would miss.  aarch64, riscv64, and x86_64 have their
            // tests in their libraries, and their binaries cannot be built
            // for a host: the boot assembly is only assembled for the bare
            // metal target.
            let targets = match package {
                "port" => "--tests",
                "aarch64" | "riscv64" | "x86_64" => "--lib",
                _ => "--bins",
            };
            cmd.args(["test", "--package", package, targets, "--target", &target.to_string()]);
            if self.json_output {
                cmd.arg("--message-format=json").arg("--quiet");
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
        // Libs and bins, linted the way the kernel is built.
        let mut cmd = self.command();
        cmd.arg("--workspace");
        exclude_other_arches(self.arch, &mut cmd);
        self.lint(cmd)?;

        // Tests and benches are separate targets and are not covered above.
        // port's build like any host library.
        let mut cmd = self.command();
        cmd.arg("--package").arg("port").arg("--tests").arg("--benches");
        self.lint(cmd)?;

        // The arch packages' tests need std, so they need an OS-specific
        // toolchain; where none is installed, skip them as check does.
        let package = self.arch.to_string().to_lowercase();
        if let Some(target) = RustupState::new().std_supported_target(&package) {
            let mut cmd = self.command();
            cmd.arg("--package").arg(&package).arg("--tests").arg("--benches");
            cmd.arg("--target").arg(target.to_string());
            self.lint(cmd)?;
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
        let bins_lib_package_cmd_args = vec![
            vec![
                "check".to_string(),
                "--package".to_string(),
                "aarch64".to_string(),
                "--bins".to_string(),
            ],
            vec![
                "check".to_string(),
                "--package".to_string(),
                "riscv64".to_string(),
                "--bins".to_string(),
            ],
            vec![
                "check".to_string(),
                "--package".to_string(),
                "x86_64".to_string(),
                "--bins".to_string(),
            ],
            vec![
                "check".to_string(),
                "--package".to_string(),
                "port".to_string(),
                "--lib".to_string(),
                "--tests".to_string(),
                "--benches".to_string(),
            ],
        ];

        let rustup_state = RustupState::new();

        // However, running check for tests and benches in arch packages requires
        // that we use a toolchain with `std`, so we need an OS-specific toolchain.
        // If the arch matches that of the current toolchain, then that will be used
        // for check.  Otherwise we'll always default to <arch>-unknown-linux-gnu.
        let mut benches_tests_package_cmd_args = Vec::new();

        for arch in ["aarch64", "riscv64", "x86_64"] {
            let Some(target) = rustup_state.std_supported_target(arch) else {
                continue;
            };

            benches_tests_package_cmd_args.push(vec![
                "check".to_string(),
                "--package".to_string(),
                arch.to_string(),
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
        let mut failed = Vec::new();
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

            let runner = ArchIntegrationTests {
                arch,
                config: load_named_config(arch, &self.config_name),
                profile: self.profile,
                timeout: self.timeout,
                verbose: self.verbose,
            };
            for name in &tests {
                println!("\n--- {arch} {name} ---");
                ran += 1;
                // An image that will not compile is that image failing, the
                // same as a non-zero exit or a timeout.  Aborting here would
                // hide every later image.
                let elf = match runner.compile(name) {
                    Ok(elf) => elf,
                    Err(err) => {
                        println!("{arch} {name}: FAILED ({err})");
                        failed.push(format!("{arch} {name}"));
                        continue;
                    }
                };
                // Laying the image out and starting qemu, by contrast, use
                // host tools that say nothing about this image and will say
                // the same for every one of them.
                let image = runner.image(name, &elf)?;
                match runner.qemu(&image)? {
                    Some(code) if code == arch.passing_status() => {
                        println!("{arch} {name}: ok")
                    }
                    Some(code) => {
                        println!("{arch} {name}: FAILED (exit {code})");
                        failed.push(format!("{arch} {name}"));
                    }
                    None => {
                        println!("{arch} {name}: TIMED OUT after {}s", self.timeout.as_secs());
                        failed.push(format!("{arch} {name}"));
                    }
                }
            }
        }

        // Having nothing to run is not a failure.  Naming a single arch is
        // the documented way to run one, and two of the three have no
        // images, so reporting the fact is the whole answer -- the loop
        // above has already said so per arch.
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
        );
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
                cmd.arg("-serial").arg("null");
                cmd.arg("-serial").arg("mon:stdio");
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
        if self.verbose {
            let stdout = child.stdout.take().expect("stdout was piped");
            let stderr = child.stderr.take().expect("stderr was piped");
            std::thread::spawn(move || {
                Self::filter_and_print(stdout);
                Self::filter_and_print(stderr);
            });
        }
        let deadline = std::time::Instant::now() + self.timeout;
        loop {
            if let Some(status) = child.try_wait()? {
                return Ok(Some(status.code().unwrap_or(-1)));
            }
            if std::time::Instant::now() >= deadline {
                child.kill()?;
                child.wait()?;
                return Ok(None);
            }
            std::thread::sleep(Duration::from_millis(50));
        }
    }

    /// Reads a stream and prints it to stdout, but filters out
    /// terminal reset and clear-screen sequences.
    fn filter_and_print(stream: impl std::io::Read) {
        use std::io::{BufReader, Read, Write};
        let mut reader = BufReader::new(stream);
        let mut buffer = [0u8; 1024];
        loop {
            match reader.read(&mut buffer) {
                Ok(0) | Err(_) => break,
                Ok(n) => {
                    let mut output = Vec::new();
                    let mut i = 0;
                    while i < n {
                        if buffer[i] == b'\x1b' && i + 1 < n && buffer[i + 1] == b'[' {
                            // Skip common clear-screen and reset sequences
                            // \x1b[2J (clear screen), \x1b[H (home), etc.
                            let mut j = i + 2;
                            while j < n
                                && buffer[j] != b';'
                                && buffer[j] != b'm'
                                && buffer[j] != b'J'
                                && buffer[j] != b'H'
                            {
                                j += 1;
                            }
                            if j < n {
                                j += 1; // consume the terminator
                            }
                            i = j;
                        } else if buffer[i] == b'\x1b' && i + 1 < n && buffer[i + 1] == b'c' {
                            i += 2; // skip RIS reset
                        } else {
                            output.push(buffer[i]);
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

        heading("test");
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
    println!("\n=== xtask: {step} ===");
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
    match arch {
        Arch::Aarch64 => {
            cmd.arg("--exclude").arg("riscv64");
            cmd.arg("--exclude").arg("x86_64");
        }
        Arch::Riscv64 => {
            cmd.arg("--exclude").arg("aarch64");
            cmd.arg("--exclude").arg("x86_64");
        }
        Arch::X86_64 => {
            cmd.arg("--exclude").arg("aarch64");
            cmd.arg("--exclude").arg("riscv64");
        }
    }
}

/// Annotates the error result with the calling binary's name.
fn annotated_status(cmd: &mut Command) -> Result<process::ExitStatus> {
    Ok(cmd.status().map_err(|e| format!("{}: {}", cmd.get_program().to_string_lossy(), e))?)
}
