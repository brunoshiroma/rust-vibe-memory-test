use memory_cache_bench::{BenchmarkConfig, Measurement, run_suite};
use std::process::ExitCode;

fn main() -> ExitCode {
    match run() {
        Ok(()) => ExitCode::SUCCESS,
        Err(error) => {
            eprintln!("error: {error}");
            ExitCode::FAILURE
        }
    }
}

fn run() -> Result<(), Box<dyn std::error::Error>> {
    let mut config = BenchmarkConfig::default();
    let mut args = std::env::args().skip(1);
    while let Some(argument) = args.next() {
        match argument.as_str() {
            "--sizes" => {
                let value = args
                    .next()
                    .ok_or("--sizes requires a comma-separated list")?;
                config.sizes_bytes = value
                    .split(',')
                    .map(parse_size)
                    .collect::<Result<Vec<_>, _>>()?;
            }
            "--iterations" => {
                config.iterations = args
                    .next()
                    .ok_or("--iterations requires a positive integer")?
                    .parse()?;
            }
            "--help" | "-h" => {
                print_help();
                return Ok(());
            }
            other => return Err(format!("unknown argument {other:?}; use --help").into()),
        }
    }

    let measurements = run_suite(&config)?;
    println!(
        "{:<16} {:>12} {:>12} {:>14} {:>14}",
        "operation", "working set", "iterations", "GB/s", "ns/element"
    );
    for result in &measurements {
        print_measurement(result);
    }
    Ok(())
}

fn print_measurement(result: &Measurement) {
    println!(
        "{:<16} {:>12} {:>12} {:>14.2} {:>14.2}",
        result.operation.name(),
        format_size(result.size_bytes),
        result.iterations,
        result.bytes_per_second / 1_000_000_000.0,
        result.nanoseconds_per_element
    );
}

fn parse_size(value: &str) -> Result<usize, Box<dyn std::error::Error>> {
    let value = value.trim();
    let split_at = value
        .find(|character: char| !character.is_ascii_digit())
        .unwrap_or(value.len());
    let (number, unit) = value.split_at(split_at);
    let number: usize = number.parse()?;
    let multiplier = match unit.trim().to_ascii_lowercase().as_str() {
        "" | "b" => 1,
        "k" | "kb" | "kib" => 1024,
        "m" | "mb" | "mib" => 1024 * 1024,
        "g" | "gb" | "gib" => 1024 * 1024 * 1024,
        _ => return Err(format!("unsupported size unit in {value:?}").into()),
    };
    number
        .checked_mul(multiplier)
        .ok_or_else(|| format!("size {value:?} is too large").into())
}

fn format_size(bytes: usize) -> String {
    if bytes.is_multiple_of(1024 * 1024) {
        format!("{} MiB", bytes / (1024 * 1024))
    } else if bytes.is_multiple_of(1024) {
        format!("{} KiB", bytes / 1024)
    } else {
        format!("{bytes} B")
    }
}

fn print_help() {
    println!(
        "Memory and cache benchmark\n\n\
         Usage: memory-cache-bench [--sizes SIZE[,SIZE...]] [--iterations COUNT]\n\n\
         Sizes accept bytes, KiB, MiB, or GiB (for example: 16KiB,256KiB,8MiB).\n\
         Defaults: 4KiB,32KiB,256KiB,2MiB,16MiB and 20 iterations.\n\n\
         COUNT is a lower bound: every operation repeats until it has been timed\n\
         for at least 50 ms, so small working sets stay measurable. The reported\n\
         iteration count is the number of passes that were actually timed."
    );
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_size_units_case_insensitively() {
        assert_eq!(parse_size(" 2MiB").unwrap(), 2 * 1024 * 1024);
        assert_eq!(parse_size("64k").unwrap(), 64 * 1024);
        assert!(parse_size("5XB").is_err());
    }
}
