# miri-repl-test

The goal of this repository is to document how to use MIRI (https://github.com/rust-lang/miri)
as an "embedded compiler", so that applications like games can hot-reload code
(so it's quickly editable while building the game) and then (in a release build),
compile it with the normal rust compiler.

This repository compiles on `nightly-2019-04-20`, be sure to install the correct compiler:

```
rustup toolchain add nightly-2019-04-20
rustup override nightly
rustup install miri
```

Since the rustc driver needs a file, the "compiler" builds a cache with the code
wrapped in a "autogen_0.rs" file at runtime.

This is just a test if / how this would work and if it is possible to pass struct
between the interpreted code and the Rust code back and forth. So far it compiles
and