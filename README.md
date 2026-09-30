# Memory and cache benchmark

Rust benchmark for comparing memory working sets on Linux, Windows, macOS, Android, iOS, and WebAssembly runtimes. It measures sequential reads, writes, copies, and dependent pointer-chase read latency. For the first three operations, `ns/element` is the average time per 64-bit element over the run; pointer-chase latency is serialized and is a better approximation of individual memory-access latency. Throughput is reported in decimal GB/s.

## Run natively

```sh
cargo run --release
cargo run --release -- --sizes 16KiB,32KiB,256KiB,2MiB,16MiB --iterations 50
cargo test
```

Choose working-set sizes around the cache capacities you want to investigate. The program does not identify cache levels or guarantee that a size fits in a particular cache: cache topology and behavior depend on the processor and system. Run several times on an otherwise idle device and compare results.

## WebAssembly

Install the `wasm32-unknown-unknown` Rust target and `wasm-pack`, then build:

```sh
wasm-pack build --target web
```

The generated package exports `run_memory_benchmark(size_bytes, iterations)`, which returns a JSON string containing the measurements. For example, import `run_memory_benchmark` from `pkg/memory_cache_bench.js` after initializing the generated module. Browser measurements are constrained by browser scheduling, available WebAssembly memory, and sandboxing; they do not provide direct access to physical cache topology.

## Android and iOS

The benchmark core is a Rust library and can be cross-compiled for Android and iOS Rust targets, then called from a native application through the platform's Rust FFI integration. Install the relevant Rust target and native SDK/toolchain first. The command-line executable is intended for desktop use; this repository does not include mobile app shells or platform-specific packaging.

## Desktop release binaries

Pushing any Git tag creates a GitHub release with packaged command-line binaries. The workflow builds Linux for x86, x64, ARM32, ARM64, and RISC-V 64-bit; Windows for x86, x64, and ARM64; and macOS for x64 and ARM64. Windows ARM32/RISC-V and macOS x86/ARM32/RISC-V are not included because Rust does not provide supported desktop targets for those combinations.

## Toolchain

`rust-toolchain.toml` tracks the latest stable Rust toolchain. Cross-compiling requires the target's Rust standard library and platform linker/SDK.
