use colored::*;
use rayon::prelude::*;
use crate::photon::Bit;
use crate::channel::SiftedKey;

pub const QBER_THRESHOLD: f64 = 0.11;

#[derive(Debug, Clone)]
pub struct QberResult {
    pub qber: f64,
    pub error_count: usize,
    pub total_compared: usize,
    pub threshold_exceeded: bool,
    pub sample_size: usize,
}

pub fn calculate_qber_parallel(
    alice_bits: &[Bit],
    bob_bits: &[Bit],
    sample_fraction: f64,
) -> QberResult {
    let sample_size = (alice_bits.len() as f64 * sample_fraction) as usize;
    let sample_size = sample_size.max(100).min(alice_bits.len());

    let chunk_size = (sample_size / rayon::current_num_threads()).max(128);
    let num_chunks = (sample_size + chunk_size - 1) / chunk_size;

    let errors: Vec<usize> = (0..num_chunks)
        .into_par_iter()
        .map(|chunk_idx| {
            let start = chunk_idx * chunk_size;
            let end = (start + chunk_size).min(sample_size);
            let mut local_errors = 0usize;

            for i in start..end {
                if alice_bits[i] != bob_bits[i] {
                    local_errors += 1;
                }
            }

            local_errors
        })
        .collect();

    let total_errors: usize = errors.iter().sum();
    let qber = if sample_size > 0 {
        total_errors as f64 / sample_size as f64
    } else {
        0.0
    };

    QberResult {
        qber,
        error_count: total_errors,
        total_compared: sample_size,
        threshold_exceeded: qber > QBER_THRESHOLD,
        sample_size,
    }
}

pub fn calculate_qber_full(
    alice_bits: &[Bit],
    bob_bits: &[Bit],
) -> QberResult {
    let chunk_size = (alice_bits.len() / rayon::current_num_threads()).max(1024);
    let num_chunks = (alice_bits.len() + chunk_size - 1) / chunk_size;

    let errors: Vec<usize> = (0..num_chunks)
        .into_par_iter()
        .map(|chunk_idx| {
            let start = chunk_idx * chunk_size;
            let end = (start + chunk_size).min(alice_bits.len());
            let mut local_errors = 0usize;

            for i in start..end {
                if alice_bits[i] != bob_bits[i] {
                    local_errors += 1;
                }
            }

            local_errors
        })
        .collect();

    let total_errors: usize = errors.iter().sum();
    let qber = if !alice_bits.is_empty() {
        total_errors as f64 / alice_bits.len() as f64
    } else {
        0.0
    };

    QberResult {
        qber,
        error_count: total_errors,
        total_compared: alice_bits.len(),
        threshold_exceeded: qber > QBER_THRESHOLD,
        sample_size: alice_bits.len(),
    }
}

pub fn calculate_qber_from_sifted(sifted_key: &SiftedKey) -> QberResult {
    calculate_qber_full(&sifted_key.alice_bits, &sifted_key.bob_bits)
}

pub fn print_qber_report(result: &QberResult) {
    println!("\n{}", "═══════════════════════════════════════════════════".cyan().bold());
    println!("{}", "           QKD NETWORK SIMULATION REPORT           ".cyan().bold());
    println!("{}", "═══════════════════════════════════════════════════".cyan().bold());

    println!("\n{}", "  QUANTUM BIT ERROR RATE (QBER)".yellow().bold());
    println!("  {}", "─".repeat(40).yellow());

    let qber_percent = result.qber * 100.0;
    let threshold_percent = QBER_THRESHOLD * 100.0;

    println!("    Total bits compared:  {}", format!("{}", result.total_compared).white().bold());
    println!("    Error count:          {}", format!("{}", result.error_count).white().bold());
    println!("    Sample size:          {}", format!("{}", result.sample_size).white().bold());

    if result.threshold_exceeded {
        println!("\n    {}: {:.4}% {}",
            "QBER".red().bold(),
            qber_percent,
            "(EXCEEDS THRESHOLD!)".red().bold().blink()
        );
        println!("    {}: {:.2}%", "Threshold".yellow(), threshold_percent);
        println!("\n{}", "  ⚠  SECURITY ALERT  ⚠".red().bold().blink());
        println!("  {}", "━".repeat(40).red());
        println!("  {}", "EAVESDROPPING DETECTED!".red().bold());
        println!("  {}", "The quantum channel has been compromised.".red());
        println!("  {}", "Aborting key negotiation immediately.".red().bold());
        println!("  {}", "━".repeat(40).red());
    } else {
        println!("\n    {}: {:.4}% {}",
            "QBER".green().bold(),
            qber_percent,
            "(✓ Within safe limits)".green()
        );
        println!("    {}: {:.2}%", "Threshold".yellow(), threshold_percent);
        println!("\n  {}", "✓ Channel is secure - Proceeding with key agreement".green().bold());
    }

    println!("\n{}", "═══════════════════════════════════════════════════".cyan().bold());
}

pub fn check_and_alert(result: &QberResult) -> bool {
    print_qber_report(result);
    !result.threshold_exceeded
}
