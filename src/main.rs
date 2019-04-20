#![feature(rustc_private)]

extern crate miri;
extern crate rustc;
extern crate rustc_interface;
extern crate rustc_driver;
extern crate rustc_version;

use std::thread;
use std::fs;
use std::time::Duration;

type EvalError = String;

#[derive(Debug, Copy, Clone)]
pub struct MiriReturn {
    value: usize,
}

fn main() {

    let cache_key = 0;
    let file_path = "./src/game_script.rs";
    let stdlib_path = "target/miri-stdlib";

    setup::setup(stdlib_path).unwrap();

    'game_loop: loop {

        println!("--------- evaluating code!!!");

        let rust_code = fs::read_to_string(file_path).unwrap();
        let evaluation_result = compiler::eval_code(rust_code, cache_key);

        println!("speed is: {}", match evaluation_result {
            Ok(r) => format!("{}", r.value),
            Err(e) => format!("ERROR: {}", e),
        });

        thread::sleep(Duration::from_millis(500));
    }
}

mod compiler {

    use super::{MiriReturn, EvalError};
    use std::fs;
    use rustc_interface::interface;
    use rustc::hir::def_id::LOCAL_CRATE;

    struct MiriCompilerCalls {
        miri_config: miri::MiriConfig,
        eval_result: Option<MiriReturn>,
    }

    impl rustc_driver::Callbacks for MiriCompilerCalls {
        fn after_parsing(&mut self, _: &interface::Compiler) -> bool { true }

        fn after_analysis(&mut self, compiler: &interface::Compiler) -> bool {
            let global_compiler_context = compiler.global_ctxt().expect("couldn't get compiler context!");

            global_compiler_context.peek_mut().enter(|tcx| {
                let (entry_def_id, _) = tcx.entry_fn(LOCAL_CRATE).expect("no main function found!");
                miri::eval_main(tcx, entry_def_id, self.miri_config.clone())
            });

            true
        }
    }

    pub fn eval_code(rust_code: String, cache_id: usize) -> Result<MiriReturn, EvalError> {

        let filename = format!("autogen_{}.rs", cache_id);
        fs::write(filename.clone(), fixup_code(rust_code)).expect("autogen file panic!");

        let miri_args = Vec::new();
        let rustc_args = vec![
            String::from("rustc"),
            filename,
            String::from("--sysroot"),
            find_sysroot(),
            String::from("--"),
            String::from("-Zmiri-disable-validation"),
        ];

        println!("rustc args: {}", rustc_args.clone().join(" "));

        let miri_config = miri::MiriConfig { validate: false, args: miri_args, seed: None };
        let mut compiler = MiriCompilerCalls { miri_config, eval_result: None };

        rustc_driver::run_compiler(&rustc_args, &mut compiler, None, None).expect("run compiler panic!");

        Ok(compiler.eval_result.unwrap())
    }

    fn fixup_code(input: String) -> String {
        format!("fn main() {{\r\n{}\r\n}}", input.lines().map(|l| format!("\t{}", l)).collect::<Vec<_>>().join("\r\n"))
    }

    fn find_sysroot() -> String {
        if let Ok(sysroot) = std::env::var("MIRI_SYSROOT") {
            return sysroot;
        }

        // Taken from PR <https://github.com/Manishearth/rust-clippy/pull/911>.
        let home = option_env!("RUSTUP_HOME").or(option_env!("MULTIRUST_HOME"));
        let toolchain = option_env!("RUSTUP_TOOLCHAIN").or(option_env!("MULTIRUST_TOOLCHAIN"));
        match (home, toolchain) {
            (Some(home), Some(toolchain)) => format!("{}/toolchains/{}", home, toolchain),
            _ => {
                option_env!("RUST_SYSROOT")
                    .expect(
                        "could not find sysroot. Either set `MIRI_SYSROOT` at run-time, or at \
                         build-time specify `RUST_SYSROOT` env var or use rustup or multirust",
                    )
                    .to_owned()
            }
        }
    }
}

mod setup {

    use std::{env, path::{Path, PathBuf}, fs::{self, File}, process::Command};

    #[derive(Debug, Copy, Clone, PartialEq, Eq)]
    pub enum SetupError {
        FailedToInstallXargo,
        FailedToInstallRustSrc,
        FailedToRunXargo,
    }

    /// Performs the setup required to make `cargo miri` work: Getting a custom-built libstd. Then sets
    /// `MIRI_SYSROOT`. Skipped if `MIRI_SYSROOT` is already set, in which case we expect the user has
    /// done all this already.
    pub fn setup<I: Into<PathBuf>>(cache_dir: I) -> Result<(), SetupError> {

        use std::io::Write;

        if env::var("MIRI_SYSROOT").is_ok() {
            return Ok(());
        }

        // First install xargo
        let needs_xargo_install = match xargo_version() { None => true, Some(v) if v < (0, 3, 13) => true, _ => false };
        if needs_xargo_install {
            println!("Xargo is necessary to build libstd. Installing xargo: `cargo install xargo -f`");
            if !Command::new("cargo").args(&["install", "xargo", "-f"]).status().unwrap().success() {
                return Err(SetupError::FailedToInstallXargo)
            }
        }

        let xargo_version = xargo_version().unwrap();
        println!("OK xargo {}.{}.{} installed!", xargo_version.0, xargo_version.1, xargo_version.2);

        // Then, unless `XARGO_RUST_SRC` is set, we also need rust-src.
        // Let's see if it is already installed.
        if env::var("XARGO_RUST_SRC").is_err() {

            let sysroot = Command::new("rustc").args(&["--print", "sysroot"]).output().unwrap().stdout;
            let sysroot = std::str::from_utf8(&sysroot).unwrap();
            let src = Path::new(sysroot.trim_end_matches('\n')).join("lib").join("rustlib").join("src");

            if !src.exists() {
                println!("Installing rust-src component: `rustup component add rust-src`");
                if !Command::new("rustup").args(&["component", "add", "rust-src"]).status().unwrap().success() {
                    return Err(SetupError::FailedToInstallRustSrc);
                }
            }

            env::set_var("XARGO_RUST_SRC", &PathBuf::from(src));
        }

        // Next, we need our own libstd. We will do this work in whatever is a good cache dir for this platform.
        let cache_dir: PathBuf = cache_dir.into();
        let cache_dir: &Path = cache_dir.as_path();

        if !cache_dir.exists() {
            fs::create_dir_all(cache_dir).unwrap();
        }

        // The interesting bit: Xargo.toml
        File::create(cache_dir.join("Xargo.toml")).unwrap().write_all(include_bytes!("./XargoTemplate.toml")).unwrap();
        // The boring bits: a dummy project for xargo.
        File::create(cache_dir.join("Cargo.toml")).unwrap().write_all(include_bytes!("./CargoTemplate.toml")).unwrap();
        File::create(cache_dir.join("lib.rs")).unwrap();

        // Run xargo.
        let target = get_arg_flag_value("--target");
        let mut command = Command::new("xargo");
        command.arg("build").arg("-q")
            .current_dir(cache_dir)
            .env("RUSTFLAGS", miri::miri_default_args().join(" "))
            .env("XARGO_HOME", cache_dir.to_str().unwrap());

        if let Some(ref target) = target {
            command.arg("--target").arg(&target);
        }

        if !command.status().unwrap().success() {
            return Err(SetupError::FailedToRunXargo);
        }

        // That should be it! But we need to figure out where xargo built stuff.
        // Unfortunately, it puts things into a different directory when the
        // architecture matches the host.
        let is_host = match target {
            None => true,
            Some(target) => target == rustc_version::version_meta().unwrap().host,
        };

        let sysroot = if is_host { cache_dir.join("HOST") } else { PathBuf::from(cache_dir) };

        env::set_var("MIRI_SYSROOT", &sysroot);

        Ok(())
    }

    fn xargo_version() -> Option<(u32, u32, u32)> {

        let out = Command::new("xargo").arg("--version").output().ok()?;
        if !out.status.success() {
            return None;
        }

        // Parse output. The first line looks like "xargo 0.3.12 (b004f1c 2018-12-13)".
        let stderr = String::from_utf8(out.stderr)
            .expect("malformed `xargo --version` output: not UTF8");

        let line = stderr
            .lines().nth(0)
            .expect("malformed `xargo --version` output: not at least one line");

        let (name, version) = {
            let mut split = line.split(' ');
            (split.next().expect("malformed `xargo --version` output: empty"),
             split.next().expect("malformed `xargo --version` output: not at least two words"))
        };

        if name != "xargo" {
            panic!("malformed `xargo --version` output: application name is not `xargo`");
        }

        let mut version_pieces = version.split('.');

        let major = version_pieces.next()
            .expect("malformed `xargo --version` output: not a major version piece")
            .parse()
            .expect("malformed `xargo --version` output: major version is not an integer");
        let minor = version_pieces.next()
            .expect("malformed `xargo --version` output: not a minor version piece")
            .parse()
            .expect("malformed `xargo --version` output: minor version is not an integer");
        let patch = version_pieces.next()
            .expect("malformed `xargo --version` output: not a patch version piece")
            .parse()
            .expect("malformed `xargo --version` output: patch version is not an integer");

        if !version_pieces.next().is_none() {
            panic!("malformed `xargo --version` output: more than three pieces in version");
        }

        Some((major, minor, patch))
    }

    fn get_arg_flag_value(name: &str) -> Option<String> {
        // Stop searching at `--`.
        let mut args = std::env::args().take_while(|val| val != "--");
        loop {
            let arg = match args.next() {
                Some(arg) => arg,
                None => return None,
            };
            if !arg.starts_with(name) {
                continue;
            }
            // Strip leading `name`.
            let suffix = &arg[name.len()..];
            if suffix.is_empty() {
                // This argument is exactly `name`; the next one is the value.
                return args.next();
            } else if suffix.starts_with('=') {
                // This argument is `name=value`; get the value.
                // Strip leading `=`.
                return Some(suffix[1..].to_owned());
            }
        }
    }
}