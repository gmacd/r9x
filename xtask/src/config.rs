/// Test
///
use crate::{Command, Profile};

use serde::{Deserialize, Serialize};
use std::{
    collections::HashMap,
    fs::{self, File, create_dir_all},
    io::Write,
    process::exit,
};

/// build section
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Build {
    /// The buildflags controls build-time operations and compiler settings.
    pub buildflags: Option<Vec<String>>,

    /// A list of custom flags to pass to all compiler invocations that Cargo performs.
    pub rustflags: Option<Vec<String>>,

    /// Build for the given architecture.
    pub target: String,
}

/// Config section
/// currently available configuration sections are dev, ip, link, nodev, nouart
/// the section name is becomes the prefix for the configuration option
/// example usage for section "dev"
/// ```toml
///  dev = [
///     'arch',
///     'cap',
///     'foo="baz"'
///  ]
/// ```
///  this will create the following configuration options
///  dev_arch, dev_cap and dev_foo="baz"
///
/// usage example:
///  ```rust
/// #[cfg(dev_arch)]
/// pub mod devarch;
/// ```
/// ```rust
/// #[cfg(dev_foo = "baz")]
/// pub mod foobaz;
/// ```
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Config {
    pub dev: Option<Vec<String>>,
    pub features: Option<Vec<String>>,
    pub ip: Option<Vec<String>>,
    pub link: Option<Vec<String>>,
    pub nodev: Option<Vec<String>>,
    pub nouart: Option<Vec<String>>,

    /// platform/board possible values: empty, vfive2, nezha, virt etc.
    ///
    /// example usage
    /// ´´´rust
    /// #[cfg(platform = "virt")]
    /// pub mod virt;
    /// ```
    pub platform: Option<String>,

    /// Filepath of DTB file relative to crate
    pub dtb: Option<String>,
}

/// Qemu section
/// Affects arguments to be passed to qemu - doesn't affect build artefacts.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Qemu {
    /// Machine (`-M`) value for qemu: raspi4b, etc.
    pub machine: Option<String>,

    /// Filepath of DTB file relative to crate
    pub dtb: Option<String>,
}

/// the TOML document
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Configuration {
    pub build: Option<Build>,
    pub config: Option<Config>,
    pub link: Option<HashMap<String, String>>,
    pub qemu: Option<Qemu>,
}

impl Configuration {
    pub fn load(filename: String) -> Self {
        let contents = match fs::read_to_string(filename.clone()) {
            Ok(c) => c,
            Err(_) => {
                eprintln!("Could not read file `{filename}`");
                exit(1);
            }
        };
        let config: Configuration = match toml::from_str(&contents) {
            Ok(d) => d,
            Err(e) => {
                eprintln!("TOML: Unable to load data from `{filename}`");
                eprintln!("{e}");
                exit(1);
            }
        };
        config
    }
}

fn apply_build(cmd: &mut Command, rustflags: &mut Vec<String>, config: &Configuration) {
    if let Some(config) = &config.build {
        let target = &config.target;
        cmd.arg("--target").arg(target);

        if let Some(flags) = &config.buildflags {
            // add the buildflags to the command
            for f in flags {
                cmd.arg(f);
            }
        }

        if let Some(flags) = &config.rustflags {
            // store the passed rustflags temporarily
            for f in flags {
                rustflags.push(f.to_string());
            }
        }
    }
}

fn apply_platform_config(cmd: &mut Command, rustflags: &mut Vec<String>, config: &Configuration) {
    if let Some(config) = &config.config {
        // if the target will use features make them available
        if let Some(features) = &config.features {
            let mut joined = features.join(",");
            if !features.is_empty() && joined.is_empty() {
                joined = features.first().unwrap().into();
            }
            cmd.arg(format!("--features={joined}"));
        }

        if let Some(platform) = &config.platform {
            push_cfg(rustflags, &format!("platform=\"{platform}\""));
        }

        // Each section name prefixes the settings it contains: a 'dev' entry
        // 'arch' becomes dev_arch.
        for (prefix, settings) in [
            ("dev", &config.dev),
            ("ip", &config.ip),
            ("link", &config.link),
            ("nodev", &config.nodev),
            ("nouart", &config.nouart),
        ] {
            for setting in settings.iter().flatten() {
                push_cfg(rustflags, &format!("{prefix}_{setting}"));
            }
        }
    }
}

/// Set a cfg and declare it in the same breath, so that a `#[cfg(...)]` naming
/// something no config file sets is a build error rather than silently dead
/// code.  Every cfg the build injects is declared here and nowhere else.
fn push_cfg(rustflags: &mut Vec<String>, cfg: &str) {
    rustflags.push("--cfg".into());
    rustflags.push(cfg.to_string());
    rustflags.push("--check-cfg".into());
    rustflags.push(check_cfg(cfg));
}

/// Render `name` or `name="value"` as the matching --check-cfg expression.
fn check_cfg(cfg: &str) -> String {
    match cfg.split_once('=') {
        Some((name, value)) => format!("cfg({name},values({value}))"),
        None => format!("cfg({cfg})"),
    }
}

fn apply_link(
    rustflags: &mut Vec<String>,
    config: &Configuration,
    target: &str,
    profile: &Profile,
    workspace_path: &str,
) -> crate::Result<()> {
    // we don't need to handle the linker script for clippy
    if let Some(link) = &config.link {
        let Some(filename) = link.get("script") else {
            return Err("config [link] table has no 'script' key".into());
        };

        // do we have a linker script ?
        if !filename.is_empty() {
            let mut contents = match fs::read_to_string(format!("{workspace_path}/{filename}")) {
                Ok(c) => c,
                Err(e) => {
                    return Err(format!("could not read linker script `{filename}`: {e}").into());
                }
            };

            // replace the placeholders with the values from the TOML
            if let Some(link) = &config.link {
                for l in link.iter() {
                    match l.0.as_str() {
                        "arch" => contents = contents.replace("${ARCH}", l.1),
                        "load-address" => contents = contents.replace("${LOAD-ADDRESS}", l.1),
                        "script" => {} // do nothing for the script option
                        _ => eprintln!("ignoring unknown option '{} = {}'", l.0, l.1),
                    }
                }
            }

            // construct the path to the target directory
            let path = crate::target_dir()
                .join(target)
                .join(profile.to_string().to_lowercase())
                .display()
                .to_string();

            // make sure the target directory exists
            create_dir_all(&path).map_err(|e| format!("could not create `{path}`: {e}"))?;

            // everything is setup, now create the linker script
            // in the target directory
            File::create(format!("{path}/kernel.ld"))
                .and_then(|mut file| file.write_all(contents.as_bytes()))
                .map_err(|e| format!("could not write `{path}/kernel.ld`: {e}"))?;

            // pass the script path to the rustflags
            rustflags.push(format!("-Clink-args=-T{path}/kernel.ld"));
        }
    }
    Ok(())
}

fn apply_qemu_config(cmd: &mut Command, config: &Configuration) {
    if let Some(config) = &config.qemu {
        if let Some(machine) = &config.machine {
            cmd.arg("-M");
            cmd.arg(machine);
        }
        if let Some(dtb) = &config.dtb {
            cmd.arg("-dtb");
            cmd.arg(dtb);
        }
    }
}

fn apply_rustflags(cmd: &mut Command, rustflags: &[String]) {
    // pass the collected rustflags
    // !! this overrides the build.rustflags from the target Cargo.toml !!
    if !rustflags.is_empty() {
        // A TOML array, not a joined string: cargo whitespace-splits a
        // string-valued build.rustflags, which would shear any flag
        // carrying a path with a space (-Clink-args=-T<path>/kernel.ld).
        let flags: Vec<String> = rustflags.iter().map(|f| toml_basic_string(f)).collect();
        cmd.arg("--config");
        cmd.arg(format!("build.rustflags=[{}]", flags.join(",")));
    }
}

/// Render `s` as a TOML basic string, escaping the two characters TOML
/// gives meaning inside one.
fn toml_basic_string(s: &str) -> String {
    format!("\"{}\"", s.replace('\\', "\\\\").replace('"', "\\\""))
}

pub fn apply_to_clippy_step(cmd: &mut Command, config: &Configuration) {
    let mut rustflags: Vec<String> = Vec::new();
    apply_platform_config(cmd, &mut rustflags, config);
    apply_rustflags(cmd, &rustflags);
}

pub fn apply_to_build_step(
    cmd: &mut Command,
    config: &Configuration,
    target: &str,
    profile: &Profile,
    workspace_path: &str,
) -> crate::Result<()> {
    let mut rustflags: Vec<String> = Vec::new();
    apply_build(cmd, &mut rustflags, config);
    apply_platform_config(cmd, &mut rustflags, config);
    apply_link(&mut rustflags, config, target, profile, workspace_path)?;
    apply_rustflags(cmd, &rustflags);
    Ok(())
}

pub fn apply_to_qemu_step(cmd: &mut Command, config: &Configuration) {
    apply_qemu_config(cmd, config);
}
