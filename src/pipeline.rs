use std::sync::mpsc::{self, SyncSender, Receiver};
use std::thread;
use std::time::Instant;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::Arc;
use rand::SeedableRng;
use rand_chacha::ChaCha8Rng;
use rand::Rng;
use indicatif::{ProgressBar, ProgressStyle};
use colored::*;

use crate::photon::{Basis, Bit, Photon, measure_photon_fast};
use crate::qber::{QberResult, QBER_THRESHOLD};
use rayon::prelude::*;

pub const CHANNEL_CAPACITY: usize = 64;
pub const BATCH_SIZE: usize = 65_536;

#[derive(Debug, Clone)]
pub struct PhotonBatch {
    pub photons: Vec<Photon>,
    pub bases: Vec<Basis>,
    pub bits: Vec<Bit>,
    pub batch_index: usize,
    pub lost_in_batch: usize,
}

#[derive(Debug, Clone)]
pub struct MeasuredBatch {
    pub alice_bases: Vec<Basis>,
    pub alice_bits: Vec<Bit>,
    pub bob_bases: Vec<Basis>,
    pub bob_bits: Vec<Bit>,
    pub lost_flags: Vec<bool>,
    pub batch_index: usize,
    pub received_in_batch: usize,
    pub lost_in_batch: usize,
}

#[derive(Debug)]
pub struct PipelineStats {
    pub total_generated: usize,
    pub total_lost_fiber: usize,
    pub total_intercepted: usize,
    pub total_received: usize,
    pub total_lost_total: usize,
    pub sifted_key_alice: Vec<Bit>,
    pub sifted_key_bob: Vec<Bit>,
    pub sifted_key_length: usize,
    pub alice_elapsed: std::time::Duration,
    pub eve_elapsed: std::time::Duration,
    pub bob_elapsed: std::time::Duration,
    pub total_elapsed: std::time::Duration,
}

pub struct PipelineConfig {
    pub num_photons: usize,
    pub attenuation_prob: f64,
    pub eve_enabled: bool,
    pub interception_prob: f64,
    pub alice_seed: u64,
    pub bob_seed: u64,
    pub eve_seed: u64,
}

fn make_progress_bar(total: u64, style_color: &str, label: &str) -> ProgressBar {
    let pb = ProgressBar::new(total);
    let template = format!(
        "{{spinner:.{style_color}}} [{{elapsed_precise}}] [{{wide_bar:.{style_color}/blue}} {{pos}}/{{len}} {label} ({{eta}}"
    );
    pb.set_style(
        ProgressStyle::with_template(&template)
            .unwrap()
            .progress_chars("=>-"),
    );
    pb
}

fn alice_producer(
    config: &PipelineConfig,
    tx: SyncSender<PhotonBatch>,
    pb: &ProgressBar,
    total_generated: &AtomicUsize,
    total_lost: &AtomicUsize,
) {
    let num_batches = (config.num_photons + BATCH_SIZE - 1) / BATCH_SIZE;
    let mut rng = ChaCha8Rng::seed_from_u64(config.alice_seed);

    for batch_idx in 0..num_batches {
        let start = batch_idx * BATCH_SIZE;
        let end = (start + BATCH_SIZE).min(config.num_photons);
        let batch_len = end - start;

        let mut photons = Vec::with_capacity(batch_len);
        let mut bases = Vec::with_capacity(batch_len);
        let mut bits = Vec::with_capacity(batch_len);
        let mut lost_in_batch = 0usize;

        for _ in 0..batch_len {
            let basis = if rng.r#gen::<bool>() { Basis::Rectilinear } else { Basis::Diagonal };
            let bit = Bit::from(rng.r#gen::<bool>());
            let lost_photon = rng.r#gen::<f64>() < config.attenuation_prob;
            let mut photon = Photon::new(basis, bit);
            photon.lost = lost_photon;
            if lost_photon {
                lost_in_batch += 1;
            }

            photons.push(photon);
            bases.push(basis);
            bits.push(bit);
        }

        total_generated.fetch_add(batch_len, Ordering::Relaxed);
        total_lost.fetch_add(lost_in_batch, Ordering::Relaxed);
        pb.inc(batch_len as u64);

        if tx.send(PhotonBatch {
            photons,
            bases,
            bits,
            batch_index: batch_idx,
            lost_in_batch,
        }).is_err() {
            break;
        }
    }
}

fn eve_relay(
    config: &PipelineConfig,
    rx: Receiver<PhotonBatch>,
    tx: SyncSender<PhotonBatch>,
    pb: &ProgressBar,
    total_intercepted: &AtomicUsize,
) {
    let mut rng = ChaCha8Rng::seed_from_u64(config.eve_seed);

    while let Ok(batch) = rx.recv() {
        let mut modified_photons = Vec::with_capacity(batch.photons.len());
        let mut intercepted_in_batch = 0usize;

        for photon in &batch.photons {
            if photon.lost {
                modified_photons.push(*photon);
                continue;
            }

            if rng.r#gen::<f64>() < config.interception_prob {
                let eve_basis = if rng.r#gen::<bool>() { Basis::Rectilinear } else { Basis::Diagonal };
                let random_bit = rng.r#gen::<bool>();
                let measured_bit = measure_photon_fast(photon.basis, photon.bit, eve_basis, random_bit);
                let new_photon = Photon::new(eve_basis, measured_bit);
                modified_photons.push(new_photon);
                intercepted_in_batch += 1;
            } else {
                modified_photons.push(*photon);
            }
        }

        total_intercepted.fetch_add(intercepted_in_batch, Ordering::Relaxed);
        pb.inc(batch.photons.len() as u64);

        let modified_batch = PhotonBatch {
            photons: modified_photons,
            bases: batch.bases,
            bits: batch.bits,
            batch_index: batch.batch_index,
            lost_in_batch: batch.lost_in_batch,
        };

        if tx.send(modified_batch).is_err() {
            break;
        }
    }
}

fn bob_consumer(
    config: &PipelineConfig,
    rx: Receiver<PhotonBatch>,
    pb: &ProgressBar,
    total_received: &AtomicUsize,
    total_lost_total: &AtomicUsize,
) -> Vec<MeasuredBatch> {
    let mut rng = ChaCha8Rng::seed_from_u64(config.bob_seed);
    let mut measured_batches = Vec::new();

    while let Ok(batch) = rx.recv() {
        let mut bob_bases = Vec::with_capacity(batch.photons.len());
        let mut bob_bits = Vec::with_capacity(batch.photons.len());
        let mut lost_flags = Vec::with_capacity(batch.photons.len());
        let mut received_in_batch = 0usize;
        let mut lost_in_batch = 0usize;

        for photon in &batch.photons {
            let measurement_basis = if rng.r#gen::<bool>() { Basis::Rectilinear } else { Basis::Diagonal };
            bob_bases.push(measurement_basis);
            lost_flags.push(photon.lost);

            if photon.lost {
                bob_bits.push(Bit::Zero);
                lost_in_batch += 1;
            } else {
                let random_bit = rng.r#gen::<bool>();
                let measured_bit = measure_photon_fast(photon.basis, photon.bit, measurement_basis, random_bit);
                bob_bits.push(measured_bit);
                received_in_batch += 1;
            }
        }

        total_received.fetch_add(received_in_batch, Ordering::Relaxed);
        total_lost_total.fetch_add(lost_in_batch, Ordering::Relaxed);
        pb.inc(batch.photons.len() as u64);

        measured_batches.push(MeasuredBatch {
            alice_bases: batch.bases,
            alice_bits: batch.bits,
            bob_bases,
            bob_bits,
            lost_flags,
            batch_index: batch.batch_index,
            received_in_batch,
            lost_in_batch,
        });
    }

    measured_batches.sort_by_key(|b| b.batch_index);
    measured_batches
}

fn reconcile_batches(measured_batches: Vec<MeasuredBatch>, pb: &ProgressBar) -> (Vec<Bit>, Vec<Bit>, usize) {
    let mut alice_sifted = Vec::new();
    let mut bob_sifted = Vec::new();
    let mut total_sifted = 0usize;

    for batch in measured_batches {
        let len = batch.alice_bases.len();
        for i in 0..len {
            if !batch.lost_flags[i] && batch.alice_bases[i] == batch.bob_bases[i] {
                alice_sifted.push(batch.alice_bits[i]);
                bob_sifted.push(batch.bob_bits[i]);
                total_sifted += 1;
            }
        }
        pb.inc(len as u64);
    }

    (alice_sifted, bob_sifted, total_sifted)
}

pub fn run_pipeline(config: &PipelineConfig) -> PipelineStats {
    let total_start = Instant::now();

    let total_generated = Arc::new(AtomicUsize::new(0));
    let total_lost = Arc::new(AtomicUsize::new(0));
    let total_intercepted = Arc::new(AtomicUsize::new(0));
    let total_received = Arc::new(AtomicUsize::new(0));
    let total_lost_total = Arc::new(AtomicUsize::new(0));

    let pb_alice = make_progress_bar(config.num_photons as u64, "green", "photons");
    let pb_eve = make_progress_bar(config.num_photons as u64, "red", "intercepted");
    let pb_bob = make_progress_bar(config.num_photons as u64, "yellow", "measurements");
    let pb_recon = make_progress_bar(config.num_photons as u64, "blue", "sifted");

    pb_eve.set_draw_target(indicatif::ProgressDrawTarget::hidden());
    pb_bob.set_draw_target(indicatif::ProgressDrawTarget::hidden());
    pb_recon.set_draw_target(indicatif::ProgressDrawTarget::hidden());

    let alice_start = Instant::now();

    let (alice_tx, eve_rx) = mpsc::sync_channel(CHANNEL_CAPACITY);

    let alice_total_gen = Arc::clone(&total_generated);
    let alice_total_lost = Arc::clone(&total_lost);
    let alice_pb = pb_alice.clone();

    let alice_config = PipelineConfig {
        num_photons: config.num_photons,
        attenuation_prob: config.attenuation_prob,
        eve_enabled: config.eve_enabled,
        interception_prob: config.interception_prob,
        alice_seed: config.alice_seed,
        bob_seed: config.bob_seed,
        eve_seed: config.eve_seed,
    };

    let alice_handle = thread::Builder::new()
        .name("alice-producer".into())
        .spawn(move || {
            alice_producer(&alice_config, alice_tx, &alice_pb, &alice_total_gen, &alice_total_lost);
        })
        .unwrap();

    let eve_elapsed;
    let eve_intercepted_val;

    let bob_handle;

    if config.eve_enabled {
        let (eve_tx, bob_rx) = mpsc::sync_channel(CHANNEL_CAPACITY);

        let eve_total_intercepted = Arc::clone(&total_intercepted);
        let eve_pb = pb_eve.clone();
        let eve_config = PipelineConfig {
            num_photons: config.num_photons,
            attenuation_prob: config.attenuation_prob,
            eve_enabled: config.eve_enabled,
            interception_prob: config.interception_prob,
            alice_seed: config.alice_seed,
            bob_seed: config.bob_seed,
            eve_seed: config.eve_seed,
        };

        let eve_handle = thread::Builder::new()
            .name("eve-relay".into())
            .spawn(move || {
                eve_relay(&eve_config, eve_rx, eve_tx, &eve_pb, &eve_total_intercepted);
            })
            .unwrap();

        let bob_total_received = Arc::clone(&total_received);
        let bob_total_lost = Arc::clone(&total_lost_total);
        let bob_pb = pb_bob.clone();
        let bob_config = PipelineConfig {
            num_photons: config.num_photons,
            attenuation_prob: config.attenuation_prob,
            eve_enabled: config.eve_enabled,
            interception_prob: config.interception_prob,
            alice_seed: config.alice_seed,
            bob_seed: config.bob_seed,
            eve_seed: config.eve_seed,
        };

        bob_handle = thread::Builder::new()
            .name("bob-consumer".into())
            .spawn(move || {
                bob_consumer(&bob_config, bob_rx, &bob_pb, &bob_total_received, &bob_total_lost)
            })
            .unwrap();

        alice_handle.join().unwrap();
        let eve_start = Instant::now();
        eve_handle.join().unwrap();
        eve_elapsed = eve_start.elapsed();
        eve_intercepted_val = total_intercepted.load(Ordering::Relaxed);
    } else {
        let bob_total_received = Arc::clone(&total_received);
        let bob_total_lost = Arc::clone(&total_lost_total);
        let bob_pb = pb_bob.clone();
        let bob_config = PipelineConfig {
            num_photons: config.num_photons,
            attenuation_prob: config.attenuation_prob,
            eve_enabled: config.eve_enabled,
            interception_prob: config.interception_prob,
            alice_seed: config.alice_seed,
            bob_seed: config.bob_seed,
            eve_seed: config.eve_seed,
        };

        bob_handle = thread::Builder::new()
            .name("bob-consumer".into())
            .spawn(move || {
                bob_consumer(&bob_config, eve_rx, &bob_pb, &bob_total_received, &bob_total_lost)
            })
            .unwrap();

        alice_handle.join().unwrap();
        eve_elapsed = std::time::Duration::ZERO;
        eve_intercepted_val = 0;
    }

    let alice_elapsed = alice_start.elapsed();
    pb_alice.finish_with_message("Alice complete");
    pb_eve.finish_with_message("Eve complete");

    let bob_start = Instant::now();
    let measured_batches = bob_handle.join().unwrap();
    let bob_elapsed = bob_start.elapsed();
    pb_bob.finish_with_message("Bob complete");

    let recon_start = Instant::now();
    pb_recon.set_draw_target(indicatif::ProgressDrawTarget::stdout());
    let (sifted_alice, sifted_bob, sifted_length) = reconcile_batches(measured_batches, &pb_recon);
    let _recon_elapsed = recon_start.elapsed();
    pb_recon.finish_with_message("Reconciliation complete");

    let total_elapsed = total_start.elapsed();

    PipelineStats {
        total_generated: total_generated.load(Ordering::Relaxed),
        total_lost_fiber: total_lost.load(Ordering::Relaxed),
        total_intercepted: eve_intercepted_val,
        total_received: total_received.load(Ordering::Relaxed),
        total_lost_total: total_lost_total.load(Ordering::Relaxed),
        sifted_key_alice: sifted_alice,
        sifted_key_bob: sifted_bob,
        sifted_key_length: sifted_length,
        alice_elapsed,
        eve_elapsed,
        bob_elapsed,
        total_elapsed,
    }
}

pub fn compute_qber_from_sifted(alice_bits: &[Bit], bob_bits: &[Bit]) -> QberResult {
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

pub fn print_pipeline_report(_stats: &PipelineStats, qber_result: &QberResult) {
    println!("\n{}", "═══════════════════════════════════════════════════".cyan().bold());
    println!("{}", "           QKD NETWORK SIMULATION REPORT           ".cyan().bold());
    println!("{}", "═══════════════════════════════════════════════════".cyan().bold());

    println!("\n{}", "  PIPELINE THROUGHPUT (Bounded Backpressure)".yellow().bold());
    println!("  {}", "─".repeat(50).yellow());
    println!("    Channel capacity:  {} batches ({} photons/batch)",
        CHANNEL_CAPACITY.to_string().white().bold(),
        BATCH_SIZE.to_string().white().bold()
    );
    println!("    Max in-flight:    ~{:.0} MB",
        (CHANNEL_CAPACITY * 2 * BATCH_SIZE * std::mem::size_of::<Photon>()) as f64 / 1_048_576.0
    );

    println!("\n{}", "  QUANTUM BIT ERROR RATE (QBER)".yellow().bold());
    println!("  {}", "─".repeat(40).yellow());

    let qber_percent = qber_result.qber * 100.0;
    let threshold_percent = QBER_THRESHOLD * 100.0;

    println!("    Total bits compared:  {}", qber_result.total_compared.to_string().white().bold());
    println!("    Error count:          {}", qber_result.error_count.to_string().white().bold());

    if qber_result.threshold_exceeded {
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
