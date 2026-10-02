# Memory and cache benchmark

Rust benchmark for comparing memory working sets on Linux, Windows, macOS, Android, iOS, and WebAssembly runtimes. It measures sequential reads, writes, copies, and dependent pointer-chase read latency. For the first three operations, `ns/element` is the average time per 64-bit element over the run; pointer-chase latency is serialized and is a better approximation of individual memory-access latency. Throughput is reported in decimal GB/s.

## Run natively

```sh
cargo run --release
cargo run --release -- --sizes 16KiB,32KiB,256KiB,2MiB,16MiB --iterations 50
cargo run --release -- --sizes 256MiB --threads 4
cargo test
```

Choose working-set sizes around the cache capacities you want to investigate. The program does not identify cache levels or guarantee that a size fits in a particular cache: cache topology and behavior depend on the processor and system. Run several times on an otherwise idle device and compare results.

`--iterations` is a lower bound, not an exact count. A short run over a small working set finishes in a few microseconds, which is below the noise floor of the system timer, so every operation first runs once untimed to warm caches and then repeats until it has been timed for at least 50 ms. The `iterations` column reports the passes that were actually timed. Set `min_measure_seconds` to `0.0` through the library API to time exactly `--iterations` passes.

Release builds target the generic architecture, so the vector width of `read` and `write` follows the baseline instruction set rather than the host CPU. Rebuild with `RUSTFLAGS="-C target-cpu=native"` to measure the widest vectors the local processor supports; on an AVX2 machine that is roughly 3x the read and write throughput of a default x86-64 build. The released binaries keep the portable baseline so they run on any machine of their architecture.

`copy` places its source and destination at the same offset within their page and keeps them cache-line aligned. Anything else makes the two lockstep streams disagree in the low address bits, which measured about 20x slower below 64 KiB and 6x slower from 1 MiB upwards.

## Threads and memory bandwidth

`--threads` gives each thread a private working set and drives them in lockstep, so the reported throughput is the aggregate. It defaults to 1.

One stream cannot saturate DRAM. A single core is limited by its own load/store throughput and prefetcher, so on a four-core machine a 256 MiB read measures roughly 27 GB/s on one thread and 89 GB/s on four. If you are trying to answer "how fast is this machine's memory", raise `--threads` until the curve flattens; if you are characterizing a cache hierarchy, keep it at 1 so each measurement is attributable to one core.

Each thread holds its own source and destination, so a run allocates `threads * 2 * size_bytes` bytes of buffers plus, for the pointer chase, `threads * size_bytes` more. Lower `--sizes` before raising `--threads` on a memory-constrained machine.

Threads are native-only; the WebAssembly build always runs single-threaded.

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
