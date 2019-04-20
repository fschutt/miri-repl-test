#![feature(rustc_private)]

extern crate miri;
extern crate rustc;
extern crate rustc_interface;
extern crate rustc_driver;

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
        ];

        println!("rustc args: {}", rustc_args.clone().join(" "));

        let miri_config = miri::MiriConfig { validate: true, args: miri_args, seed: None };
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