//! Benchmark case validation - verifies every benchmark case compiles without errors.
//!
//! WHAT: Parses both `cases.txt` and `frontend-cases.txt`, runs each case through
//! the compiler, and fails if any diagnostic error is produced.
//! WHY: Guards against the blindspot where `moth check` always exits with code 0
//! even on compilation errors, so benchmark timing data could measure diagnostic
//! speed rather than successful compilation.

use std::path::PathBuf;
use std::process::Command;

use crate::case_parser::{BenchmarkCase, parse_cases};
use crate::frontend_bench::run_one_frontend_case;

const BENCHMARK_CASES_PATH: &str = "benchmarks/cases.txt";
const FRONTEND_CASES_PATH: &str = "benchmarks/frontend-cases.txt";

/// Validate all benchmark cases compile without errors.
///
/// Checks CLI benchmark cases by running `moth check --terse <args>` and
/// inspecting the output for `errors=0`. Checks frontend benchmark cases
/// through the in-process compiler API.
///
/// # Returns
///
/// Ok(()) if every case compiles cleanly, or an error message listing all
/// failures.
pub fn validate_all_benchmarks() -> Result<(), String> {
    let mut failures: Vec<String> = Vec::new();

    validate_cli_cases(&mut failures)?;
    validate_frontend_cases(&mut failures)?;

    if failures.is_empty() {
        println!("All benchmark cases compile without errors.");
        Ok(())
    } else {
        let mut msg = String::new();
        msg.push_str(&format!(
            "{} benchmark case(s) failed to compile:\n",
            failures.len()
        ));
        for f in &failures {
            msg.push_str(&format!("  - {}\n", f));
        }
        Err(msg.trim_end().to_string())
    }
}

fn validate_cli_cases(failures: &mut Vec<String>) -> Result<(), String> {
    let cases = load_cli_cases()?;
    if cases.is_empty() {
        println!("  CLI cases: none found");
        return Ok(());
    }

    println!("Validating {} CLI benchmark cases...", cases.len());
    for case in &cases {
        print!("  {} ... ", case.name);
        match check_cli_case(case) {
            Ok(()) => println!("ok"),
            Err(msg) => {
                println!("FAIL");
                failures.push(format!("{}: {}", case.name, msg));
            }
        }
    }

    Ok(())
}

fn validate_frontend_cases(failures: &mut Vec<String>) -> Result<(), String> {
    let cases = load_frontend_cases()?;
    if cases.is_empty() {
        println!("  Frontend cases: none found");
        return Ok(());
    }

    println!("Validating {} frontend benchmark cases...", cases.len());
    for case in &cases {
        print!("  {} ... ", case.name);
        match check_frontend_case(case) {
            Ok(()) => println!("ok"),
            Err(msg) => {
                println!("FAIL");
                failures.push(format!("{}: {}", case.name, msg));
            }
        }
    }

    Ok(())
}

fn load_cli_cases() -> Result<Vec<BenchmarkCase>, String> {
    let cases_path = PathBuf::from(BENCHMARK_CASES_PATH);
    parse_cases(&cases_path)
}

fn load_frontend_cases() -> Result<Vec<BenchmarkCase>, String> {
    let cases_path = PathBuf::from(FRONTEND_CASES_PATH);
    parse_cases(&cases_path)
}

/// Check a single CLI case by running `moth check --terse <args>` and
/// inspecting the output for compilation errors.
fn check_cli_case(case: &BenchmarkCase) -> Result<(), String> {
    let output = Command::new("cargo")
        .arg("run")
        .arg("--quiet")
        .arg("--")
        .arg("check")
        .arg("--terse")
        .args(&case.args)
        .output()
        .map_err(|e| format!("Failed to run moth check: {}", e))?;

    let stdout = String::from_utf8_lossy(&output.stdout);
    let stderr = String::from_utf8_lossy(&output.stderr);

    let combined = format!("{}{}", stdout, stderr);

    // Check for the machine-readable error summary.
    if combined.contains("errors=") {
        // Extract the error count from the summary line.
        if let Some(errors_line) = combined.lines().find(|l| l.contains("errors=")) {
            let parts: Vec<&str> = errors_line.split(',').collect();
            if let Some(errors_part) = parts.first() {
                let count_str = errors_part.trim().strip_prefix("errors=").unwrap_or("0");
                if let Ok(count) = count_str.trim().parse::<u32>() {
                    if count > 0 {
                        // Return the terse diagnostic output as the error message.
                        let diagnostic_lines: Vec<&str> = combined
                            .lines()
                            .filter(|l| l.starts_with("E|"))
                            .collect();
                        let msg = if diagnostic_lines.is_empty() {
                            format!("{} error(s) found", count)
                        } else {
                            diagnostic_lines.join("; ")
                        };
                        return Err(msg);
                    }
                }
            }
        }
    }

    // Also check for non-zero exit code as a fallback.
    if !output.status.success() {
        return Err(format!("moth check exited with code {:?}", output.status.code()));
    }

    Ok(())
}

/// Check a single frontend case using the in-process compiler API.
fn check_frontend_case(case: &BenchmarkCase) -> Result<(), String> {
    run_one_frontend_case(case).map(|_| ())
}
