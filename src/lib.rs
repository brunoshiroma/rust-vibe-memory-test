use std::time::Duration;
#[cfg(not(target_arch = "wasm32"))]
use std::time::Instant;

/// Minimum wall-clock duration of a single timed measurement. A short run over
/// a small working set finishes in a few microseconds, which is below the noise
/// floor of the system timer, so every operation repeats until it has been
/// timed for at least this long.
pub const MIN_MEASURE_SECONDS: f64 = 0.05;

#[derive(Clone, Debug)]
pub struct BenchmarkConfig {
    pub sizes_bytes: Vec<usize>,
    /// Lower bound on the number of timed passes per measurement. Operations
    /// keep repeating past this value until `min_measure_seconds` is reached,
    /// so this is a floor rather than an exact count.
    pub iterations: usize,
    /// Minimum timed duration per measurement. `0.0` times exactly
    /// `iterations` passes.
    pub min_measure_seconds: f64,
}

impl Default for BenchmarkConfig {
    fn default() -> Self {
        Self {
            sizes_bytes: vec![
                4 * 1024,
                32 * 1024,
                256 * 1024,
                2 * 1024 * 1024,
                16 * 1024 * 1024,
            ],
            iterations: 20,
            min_measure_seconds: MIN_MEASURE_SECONDS,
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Operation {
    Read,
    Write,
    Copy,
    PointerChase,
}

impl Operation {
    pub fn name(self) -> &'static str {
        match self {
            Self::Read => "read",
            Self::Write => "write",
            Self::Copy => "copy",
            Self::PointerChase => "pointer-chase",
        }
    }
}

#[derive(Clone, Debug)]
pub struct Measurement {
    pub operation: Operation,
    pub size_bytes: usize,
    pub iterations: usize,
    pub elapsed_seconds: f64,
    pub bytes_per_second: f64,
    pub nanoseconds_per_element: f64,
}

struct Timer {
    #[cfg(not(target_arch = "wasm32"))]
    started: Instant,
    #[cfg(target_arch = "wasm32")]
    started_millis: f64,
}

impl Timer {
    fn start() -> Self {
        #[cfg(not(target_arch = "wasm32"))]
        {
            Self {
                started: Instant::now(),
            }
        }
        #[cfg(target_arch = "wasm32")]
        {
            Self {
                started_millis: wasm::performance_now(),
            }
        }
    }

    fn elapsed(&self) -> Duration {
        #[cfg(not(target_arch = "wasm32"))]
        {
            self.started.elapsed()
        }
        #[cfg(target_arch = "wasm32")]
        {
            Duration::from_secs_f64(
                ((wasm::performance_now() - self.started_millis) / 1000.0).max(0.0),
            )
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum BenchmarkError {
    NoSizes,
    ZeroIterations,
    InvalidSize(usize),
}

impl std::fmt::Display for BenchmarkError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::NoSizes => write!(f, "at least one working-set size is required"),
            Self::ZeroIterations => write!(f, "iterations must be greater than zero"),
            Self::InvalidSize(size) => {
                write!(
                    f,
                    "working-set size {size} must be at least 8 and divisible by 8"
                )
            }
        }
    }
}

impl std::error::Error for BenchmarkError {}

const CACHE_LINE_BYTES: usize = 64;
const PAGE_BYTES: usize = 4096;
const WORDS_PER_CACHE_LINE: usize = CACHE_LINE_BYTES / std::mem::size_of::<u64>();
const WORDS_PER_PAGE: usize = PAGE_BYTES / std::mem::size_of::<u64>();

/// Source and destination for one working-set size, carved out of a single
/// allocation so their relative offset is exact and independent of the
/// allocator.
///
/// The two halves are deliberately placed at the *same* offset within their
/// page. `memcpy` streams the source and the destination in lockstep, and when
/// the two disagree modulo 4096 every store and its paired load differ in the
/// low address bits, so the memory system cannot disambiguate the streams and
/// falls back to a much slower path. Letting the allocator hand out two
/// back-to-back allocations instead produces a distance of `size + 16` bytes,
/// which lands on a different page offset for every working set that is a
/// multiple of 4 KiB. That one misaligned case measured about 20x slower below
/// 64 KiB and 6x slower from 1 MiB upwards.
struct WorkingSet {
    buffer: Vec<u64>,
    source_start: usize,
    destination_start: usize,
    len: usize,
}

impl WorkingSet {
    fn new(len: usize) -> Self {
        // One page of slack at minimum, then round the distance between the two
        // halves up to a whole number of pages.
        let slack = WORDS_PER_PAGE + (WORDS_PER_PAGE - len % WORDS_PER_PAGE) % WORDS_PER_PAGE;
        let capacity = len
            .saturating_mul(2)
            .saturating_add(slack + WORDS_PER_CACHE_LINE);
        let buffer = vec![0_u64; capacity];
        let base = buffer.as_ptr() as usize;
        let source_start = (CACHE_LINE_BYTES - base % CACHE_LINE_BYTES) % CACHE_LINE_BYTES
            / std::mem::size_of::<u64>();
        let destination_start = source_start + len + slack;
        Self {
            buffer,
            source_start,
            destination_start,
            len,
        }
    }

    /// Byte distance between the two halves. Always a non-zero multiple of
    /// [`PAGE_BYTES`], so they share a page offset.
    #[cfg(test)]
    fn separation_bytes(&self) -> usize {
        (self.destination_start - self.source_start) * std::mem::size_of::<u64>()
    }

    fn source(&self) -> &[u64] {
        &self.buffer[self.source_start..self.source_start + self.len]
    }

    fn source_mut(&mut self) -> &mut [u64] {
        let start = self.source_start;
        &mut self.buffer[start..start + self.len]
    }

    fn destination_mut(&mut self) -> &mut [u64] {
        let start = self.destination_start;
        &mut self.buffer[start..start + self.len]
    }

    fn fill_source(&mut self) {
        for (index, value) in self.source_mut().iter_mut().enumerate() {
            *value = index as u64;
        }
    }

    fn copy(&mut self) {
        let len = std::hint::black_box(self.len);
        let (source_start, destination_start) = (self.source_start, self.destination_start);
        let (head, tail) = self.buffer.split_at_mut(destination_start);
        tail[..len].copy_from_slice(&head[source_start..source_start + len]);
    }
}

pub fn run_suite(config: &BenchmarkConfig) -> Result<Vec<Measurement>, BenchmarkError> {
    if config.sizes_bytes.is_empty() {
        return Err(BenchmarkError::NoSizes);
    }
    if config.iterations == 0 {
        return Err(BenchmarkError::ZeroIterations);
    }

    let mut measurements = Vec::with_capacity(config.sizes_bytes.len() * 4);
    for &size_bytes in &config.sizes_bytes {
        if size_bytes < std::mem::size_of::<u64>() || !size_bytes.is_multiple_of(8) {
            return Err(BenchmarkError::InvalidSize(size_bytes));
        }
        let words = size_bytes / std::mem::size_of::<u64>();
        let mut working_set = WorkingSet::new(words);
        working_set.fill_source();

        let (passes, elapsed) = timed(config, |pass| {
            let checksum = std::hint::black_box(working_set.source())
                .iter()
                .copied()
                .fold(0_u64, u64::wrapping_add);
            std::hint::black_box(checksum);
            std::hint::black_box(pass);
        });
        measurements.push(measurement(
            Operation::Read,
            size_bytes,
            words,
            passes,
            elapsed,
            size_bytes,
        ));

        let (passes, elapsed) = timed(config, |pass| {
            working_set.destination_mut().fill(pass as u64);
            std::hint::black_box(&working_set);
        });
        measurements.push(measurement(
            Operation::Write,
            size_bytes,
            words,
            passes,
            elapsed,
            size_bytes,
        ));

        let (passes, elapsed) = timed(config, |pass| {
            working_set.copy();
            std::hint::black_box(&working_set);
            std::hint::black_box(pass);
        });
        measurements.push(measurement(
            Operation::Copy,
            size_bytes,
            words,
            passes,
            elapsed,
            size_bytes.saturating_mul(2),
        ));

        let next = make_pointer_chase_ring(words);
        let mut index = 0;
        let (passes, elapsed) = timed(config, |_| {
            for _ in 0..words {
                index = std::hint::black_box(next[index]);
            }
        });
        std::hint::black_box(index);
        measurements.push(measurement(
            Operation::PointerChase,
            size_bytes,
            words,
            passes,
            elapsed,
            size_bytes,
        ));
    }
    Ok(measurements)
}

/// Runs `operation` once untimed to warm caches, branch predictors and the TLB,
/// then repeats it until both `iterations` passes and `min_measure_seconds` of
/// wall-clock time have been recorded. Returns the number of timed passes and
/// the time they took.
fn timed<F: FnMut(usize)>(config: &BenchmarkConfig, mut operation: F) -> (usize, Duration) {
    operation(0);

    let start = Timer::start();
    let mut passes = 0;
    loop {
        operation(passes);
        passes += 1;
        if passes >= config.iterations
            && start.elapsed().as_secs_f64() >= config.min_measure_seconds
        {
            return (passes, start.elapsed());
        }
    }
}

fn measurement(
    operation: Operation,
    size_bytes: usize,
    elements_per_iteration: usize,
    iterations: usize,
    elapsed: std::time::Duration,
    transferred_bytes_per_iteration: usize,
) -> Measurement {
    let elapsed_seconds = elapsed.as_secs_f64().max(1e-9);
    let elapsed_for_division = elapsed_seconds.max(f64::MIN_POSITIVE);
    let total_elements = elements_per_iteration.saturating_mul(iterations);
    let total_bytes = transferred_bytes_per_iteration.saturating_mul(iterations);

    Measurement {
        operation,
        size_bytes,
        iterations,
        elapsed_seconds,
        bytes_per_second: total_bytes as f64 / elapsed_for_division,
        nanoseconds_per_element: elapsed.as_nanos() as f64 / total_elements.max(1) as f64,
    }
}

fn make_pointer_chase_ring(len: usize) -> Vec<usize> {
    let mut order = (0..len).collect::<Vec<_>>();
    let mut state = 0x9e37_79b9_7f4a_7c15_u64;
    for index in (1..len).rev() {
        state ^= state << 13;
        state ^= state >> 7;
        state ^= state << 17;
        order.swap(index, (state as usize) % (index + 1));
    }

    let mut next = vec![0; len];
    for pair in order.windows(2) {
        next[pair[0]] = pair[1];
    }
    next[*order.last().expect("working set is non-empty")] = order[0];
    next
}

#[cfg(target_arch = "wasm32")]
mod wasm {
    use super::{BenchmarkConfig, run_suite};
    use wasm_bindgen::prelude::*;

    #[wasm_bindgen]
    extern "C" {
        #[wasm_bindgen(js_namespace = performance, js_name = now)]
        pub(super) fn performance_now() -> f64;
    }

    #[wasm_bindgen]
    pub fn run_memory_benchmark(size_bytes: u32, iterations: u32) -> Result<String, JsValue> {
        let measurements = run_suite(&BenchmarkConfig {
            sizes_bytes: vec![size_bytes as usize],
            iterations: iterations as usize,
            ..BenchmarkConfig::default()
        })
        .map_err(|error| JsValue::from_str(&error.to_string()))?;

        let results = measurements
            .iter()
            .map(|result| {
                format!(
                    "{{\"operation\":\"{}\",\"size_bytes\":{},\"iterations\":{},\"elapsed_seconds\":{},\"bytes_per_second\":{},\"nanoseconds_per_element\":{}}}",
                    result.operation.name(),
                    result.size_bytes,
                    result.iterations,
                    result.elapsed_seconds,
                    result.bytes_per_second,
                    result.nanoseconds_per_element
                )
            })
            .collect::<Vec<_>>()
            .join(",");

        Ok(format!("[{results}]"))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn working_set_halves_share_a_page_offset() {
        // A separation that is not a multiple of 4096 bytes is what made `copy`
        // run up to 20x slower, so pin the invariant down for page-aligned and
        // unaligned working-set lengths alike.
        for len in [1, 2, 8, 511, 512, 513, 4096, 32_768, 100_000] {
            let mut working_set = WorkingSet::new(len);
            let source = working_set.source().as_ptr() as usize;
            let destination = working_set.destination_mut().as_mut_ptr() as usize;

            assert_eq!(
                working_set.separation_bytes() % PAGE_BYTES,
                0,
                "len {len} gives a separation of {} bytes",
                working_set.separation_bytes()
            );
            assert_eq!(
                source % PAGE_BYTES,
                destination % PAGE_BYTES,
                "len {len} splits the halves across page offsets"
            );
            assert_eq!(
                source % CACHE_LINE_BYTES,
                0,
                "len {len} source is not cache-line aligned"
            );
            assert_eq!(
                destination % CACHE_LINE_BYTES,
                0,
                "len {len} destination is not cache-line aligned"
            );
            assert!(
                destination >= source + len * std::mem::size_of::<u64>(),
                "len {len} overlaps"
            );
        }
    }

    #[test]
    fn working_set_copy_reproduces_the_source() {
        let mut working_set = WorkingSet::new(997);
        working_set.fill_source();
        let expected = working_set.source().to_vec();

        working_set.copy();

        assert_eq!(working_set.source(), expected.as_slice());
        assert_eq!(working_set.destination_mut(), expected.as_slice());
    }

    #[test]
    fn timed_keeps_repeating_until_the_minimum_duration() {
        let config = BenchmarkConfig {
            sizes_bytes: vec![4096],
            iterations: 1,
            min_measure_seconds: 0.05,
        };
        let mut calls = 0;

        let (passes, elapsed) = timed(&config, |_| calls += 1);

        // One untimed warm-up plus at least `iterations` timed passes.
        assert_eq!(calls, passes + 1);
        assert!(passes >= config.iterations);
        assert!(elapsed.as_secs_f64() >= config.min_measure_seconds);
    }

    #[test]
    fn timed_stops_at_the_iteration_floor_when_no_minimum_is_set() {
        let config = BenchmarkConfig {
            sizes_bytes: vec![4096],
            iterations: 3,
            min_measure_seconds: 0.0,
        };

        let (passes, _) = timed(&config, |_| {});

        assert_eq!(passes, 3);
    }

    #[test]
    fn suite_measures_all_operations_for_each_size() {
        let results = run_suite(&BenchmarkConfig {
            sizes_bytes: vec![8, 64],
            iterations: 2,
            min_measure_seconds: 0.0,
        })
        .unwrap();

        assert_eq!(results.len(), 8);
        assert_eq!(
            results
                .iter()
                .map(|item| item.operation)
                .collect::<Vec<_>>(),
            vec![
                Operation::Read,
                Operation::Write,
                Operation::Copy,
                Operation::PointerChase,
                Operation::Read,
                Operation::Write,
                Operation::Copy,
                Operation::PointerChase,
            ]
        );
        assert!(results.iter().all(|item| item.bytes_per_second.is_finite()));
    }

    #[test]
    fn rejects_empty_and_invalid_configurations() {
        assert_eq!(
            run_suite(&BenchmarkConfig {
                sizes_bytes: vec![],
                iterations: 1,
                ..BenchmarkConfig::default()
            })
            .unwrap_err(),
            BenchmarkError::NoSizes
        );
        assert_eq!(
            run_suite(&BenchmarkConfig {
                sizes_bytes: vec![16],
                iterations: 0,
                ..BenchmarkConfig::default()
            })
            .unwrap_err(),
            BenchmarkError::ZeroIterations
        );
        assert_eq!(
            run_suite(&BenchmarkConfig {
                sizes_bytes: vec![7],
                iterations: 1,
                ..BenchmarkConfig::default()
            })
            .unwrap_err(),
            BenchmarkError::InvalidSize(7)
        );
    }

    #[test]
    fn pointer_chase_ring_visits_every_entry_once() {
        let ring = make_pointer_chase_ring(17);
        let mut visited = vec![false; ring.len()];
        let mut index = 0;

        for _ in 0..ring.len() {
            assert!(!visited[index]);
            visited[index] = true;
            index = ring[index];
        }

        assert_eq!(index, 0);
        assert!(visited.into_iter().all(|item| item));
    }
}
