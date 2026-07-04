# Tessera

Tessera is a compact embeddable scripting language implemented in Rust. It has
a real language pipeline: lexer, parser, optimizer, compiler, binary module
serializer, module loader, semantic verifier, and register-based virtual
machine. Host applications can run source text directly or precompile source to
`.tmod` bytecode modules and load those modules later.

The project is dependency-free in the core library. The command-line tool is
included for development workflows such as formatting source, compiling modules,
disassembling bytecode, inspecting module structure, validating project
manifests, and viewing standard-library documentation.

## Useful Commands

```bash
cargo run -- run examples/hello.tsr
cargo run -- build examples/hello.tsr -o hello.tmod
cargo run -- exec hello.tmod
cargo run -- dis hello.tmod
cargo run -- inspect hello.tmod
cargo run -- manifest Tessera.toml
cargo run -- doc range
```

## Library Entry Points

- `compile_source` lowers Tessera source text into an in-memory module.
- `compile_to_bytes` serializes compiled modules into the `.tmod` format.
- `load_module` decodes and verifies untrusted module bytes.
- `run_source` compiles and executes source text.
- `run_module_bytes` loads, verifies, and executes untrusted bytecode.
- `inspect_module` produces a structured report for tooling.
- `parse_manifest` validates `Tessera.toml` project metadata.

## Fuzzing

The fuzz targets live under `fuzz/fuzz_targets`:

- `source_fuzzer` exercises UTF-8 source parsing, optimization, compilation,
  verification, and VM execution.
- `module_fuzzer` exercises the binary module loader, verifier, and VM over
  arbitrary bytes.

The ClusterFuzzLite build script is `.clusterfuzzlite/build.sh`. It uses Cargo
in locked offline mode and links the harness binaries with the libFuzzer engine
provided by the runner. The `libfuzzer-sys` macro used by the harnesses is a
small local crate under `fuzz/libfuzzer-sys`, so no crates are downloaded during
the build.
