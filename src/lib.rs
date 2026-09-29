use std::time::Instant;

#[derive(Clone, Debug)]
pub struct BenchmarkConfig {
    pub sizes_bytes: Vec<usize>,
    pub iterations: usize,
}

impl Default for BenchmarkConfig {
    fn default() -> Self {
        Self {
            sizes_bytes: vec![4 * 1024, 32 * 1024, 256 * 1024, 2 * 1024 * 1024, 16 * 1024 * 1024],
            iterations: 20,
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
                write!(f, "working-set size {size} must be at least 8 and divisible by 8")
            }
        }
    }
}

impl std::error::Error for BenchmarkError {}

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
        let mut source = vec![0_u64; words];
        let mut destination = vec![0_u64; words];

        for iteration in 0..config.iterations {
            for (index, value) in source.iter_mut().enumerate() {
                *value = std::hint::black_box((index as u64).wrapping_add(iteration as u64));
            }
        }

        let start = Instant::now();
        let mut checksum = 0_u64;
        for _ in 0..config.iterations {
            for &value in &source {
                checksum = checksum.wrapping_add(std::hint::black_box(value));
            }
        }
        std::hint::black_box(checksum);
        measurements.push(measurement(
            Operation::Read,
            size_bytes,
            words,
            config.iterations,
            start.elapsed(),
            size_bytes,
        ));

        let start = Instant::now();
        for iteration in 0..config.iterations {
            for (index, value) in destination.iter_mut().enumerate() {
                *value = std::hint::black_box((index as u64).wrapping_add(iteration as u64));
            }
            std::hint::black_box(&destination);
        }
        measurements.push(measurement(
            Operation::Write,
            size_bytes,
            words,
            config.iterations,
            start.elapsed(),
            size_bytes,
        ));

        let start = Instant::now();
        for _ in 0..config.iterations {
            destination.copy_from_slice(std::hint::black_box(&source));
            std::hint::black_box(&destination);
        }
        measurements.push(measurement(
            Operation::Copy,
            size_bytes,
            words,
            config.iterations,
            start.elapsed(),
            size_bytes.saturating_mul(2),
        ));

        let next = make_pointer_chase_ring(words);
        let start = Instant::now();
        let mut index = 0;
        for _ in 0..config.iterations.saturating_mul(words) {
            index = std::hint::black_box(next[index]);
        }
        std::hint::black_box(index);
        measurements.push(measurement(
            Operation::PointerChase,
            size_bytes,
            words,
            config.iterations,
            start.elapsed(),
            size_bytes,
        ));
    }
    Ok(measurements)
}

fn measurement(
    operation: Operation,
    size_bytes: usize,
    elements_per_iteration: usize,
    iterations: usize,
    elapsed: std::time::Duration,
    transferred_bytes_per_iteration: usize,
) -> Measurement {
    let elapsed_seconds = elapsed.as_secs_f64();
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
    let stride = (1..len)
        .find(|stride| gcd(*stride, len) == 1)
        .unwrap_or(0);
    let mut next = vec![0; len];
    let mut current = 0;
    for _ in 0..len {
        let following = (current + stride) % len;
        next[current] = following;
        current = following;
    }
    next
}

fn gcd(mut left: usize, mut right: usize) -> usize {
    while right != 0 {
        (left, right) = (right, left % right);
    }
    left
}

#[cfg(target_arch = "wasm32")]
mod wasm {
    use super::{BenchmarkConfig, run_suite};
    use wasm_bindgen::prelude::*;

    #[wasm_bindgen]
    pub fn run_memory_benchmark(size_bytes: u32, iterations: u32) -> Result<String, JsValue> {
        let measurements = run_suite(&BenchmarkConfig {
            sizes_bytes: vec![size_bytes as usize],
            iterations: iterations as usize,
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
    fn suite_measures_all_operations_for_each_size() {
        let results = run_suite(&BenchmarkConfig {
            sizes_bytes: vec![8, 64],
            iterations: 2,
        })
        .unwrap();

        assert_eq!(results.len(), 8);
        assert_eq!(
            results.iter().map(|item| item.operation).collect::<Vec<_>>(),
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
                iterations: 1
            })
            .unwrap_err(),
            BenchmarkError::NoSizes
        );
        assert_eq!(
            run_suite(&BenchmarkConfig {
                sizes_bytes: vec![16],
                iterations: 0
            })
            .unwrap_err(),
            BenchmarkError::ZeroIterations
        );
        assert_eq!(
            run_suite(&BenchmarkConfig {
                sizes_bytes: vec![7],
                iterations: 1
            })
            .unwrap_err(),
            BenchmarkError::InvalidSize(7)
        );
    }
}
