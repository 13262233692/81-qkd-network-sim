mod photon;
mod alice;
mod bob;
mod channel;
mod eve;
mod qber;

use std::time::Instant;
use colored::*;
use clap::Parser;

use alice::{AliceConfig, generate_photons_parallel};
use bob::{BobConfig, measure_photons_parallel};
use eve::{EveConfig, intercept_and_resend_parallel};
use channel::{reconcile_bases_parallel};
use qber::{calculate_qber_from_sifted, check_and_alert};
use photon::{Photon, Bit};

fn format_num(n: usize) -> String {
    n.to_string()
        .as_bytes()
        .rchunks(3)
        .rev()
        .map(std::str::from_utf8)
        .collect::<Result<Vec<&str>, _>>()
        .unwrap()
        .join("_")
}

#[derive(Parser, Debug, Clone)]
#[command(name = "qkd-network-sim", version = "0.1.0", about = "Quantum Key Distribution Network Simulator")]
struct Cli {
    #[arg(short = 'n', long = "num-photons", default_value_t = 10_000_000)]
    num_photons: usize,

    #[arg(short = 'a', long = "attenuation", default_value_t = 0.01)]
    attenuation: f64,

    #[arg(short = 'e', long = "eve-enabled", default_value_t = true)]
    eve_enabled: bool,

    #[arg(short = 'p', long = "interception-prob", default_value_t = 0.5)]
    interception_prob: f64,

    #[arg(short = 's', long = "seed", default_value_t = 42)]
    seed: u64,

    #[arg(long = "no-eve", default_value_t = false)]
    no_eve: bool,
}

fn print_header() {
    println!("\n{}", "╔═══════════════════════════════════════════════════════════════╗".bright_cyan().bold());
    println!("{}", "║        ██████╗ ██╗  ██╗██████╗     ███╗   ██╗███████╗████████╗║".bright_cyan().bold());
    println!("{}", "║        ██╔══██╗██║ ██╔╝██╔══██╗    ████╗  ██║██╔════╝╚══██╔══╝║".bright_cyan().bold());
    println!("{}", "║        ██████╔╝█████╔╝ ██║  ██║    ██╔██╗ ██║█████╗     ██║   ║".bright_cyan().bold());
    println!("{}", "║        ██╔═══╝ ██╔═██╗ ██║  ██║    ██║╚██╗██║██╔══╝     ██║   ║".bright_cyan().bold());
    println!("{}", "║        ██║     ██║  ██╗██████╔╝    ██║ ╚████║███████╗   ██║   ║".bright_cyan().bold());
    println!("{}", "║        ╚═╝     ╚═╝  ╚═╝╚═════╝     ╚═╝  ╚═══╝╚══════╝   ╚═╝   ║".bright_cyan().bold());
    println!("{}", "╚═══════════════════════════════════════════════════════════════╝".bright_cyan().bold());
    println!("\n{}", "     Quantum Key Distribution Network Terminal Simulator".yellow().bold());
    println!("{}", "     BB84 Protocol with Intercept-Resend Eavesdropping".yellow());
    println!();
}

fn print_config(cli: &Cli) {
    println!("{}", "  Simulation Configuration:".blue().bold());
    println!("  {}", "─".repeat(50).blue());
    println!("    Photons to generate:     {}", format_num(cli.num_photons).white().bold());
    println!("    Fiber attenuation:       {:.2}%", cli.attenuation * 100.0);
    println!("    Eve interception:        {}", if cli.no_eve { "OFF".green().bold() } else { "ACTIVE".red().bold() });
    if !cli.no_eve {
        println!("    Interception prob:       {:.1}%", cli.interception_prob * 100.0);
    }
    println!("    Random seed:             {}", cli.seed);
    println!();
}

fn main() {
    let cli = Cli::parse();
    let cli = Cli {
        eve_enabled: !cli.no_eve,
        ..cli
    };

    print_header();
    print_config(&cli);

    let total_start = Instant::now();

    let alice_config = AliceConfig {
        num_photons: cli.num_photons,
        attenuation_prob: cli.attenuation,
        seed: cli.seed,
    };

    let bob_config = BobConfig {
        seed: cli.seed.wrapping_add(12345),
    };

    let eve_config = EveConfig {
        enabled: cli.eve_enabled,
        interception_prob: cli.interception_prob,
        seed: cli.seed.wrapping_add(67890),
    };

    println!("{}", "  Phase 1: Alice generates photon pulses...".cyan().bold());
    let gen_start = Instant::now();
    let generated = generate_photons_parallel(&alice_config);
    let gen_duration = gen_start.elapsed();

    println!("    Generated:   {} photons", format_num(generated.total_generated).white().bold());
    println!("    Lost in fiber: {} ({:.2}%)",
        format_num(generated.total_lost).white().bold(),
        generated.total_lost as f64 / generated.total_generated as f64 * 100.0
    );
    println!("    Throughput:  {:.2} M photons/sec",
        generated.total_generated as f64 / gen_duration.as_secs_f64() / 1_000_000.0
    );
    println!();

    let photons_for_bob: Vec<Photon>;

    if cli.eve_enabled {
        println!("{}", "  Phase 2: Eve intercepts and resends photons...".red().bold());
        let eve_start = Instant::now();
        let (modified, eve_results) = intercept_and_resend_parallel(&generated.photons, &eve_config);
        let eve_duration = eve_start.elapsed();
        photons_for_bob = modified;
        let eve_intercepted = eve_results.intercepted_count;

        println!("    Intercepted: {} photons", format_num(eve_intercepted).white().bold());
        println!("    Rate:        {:.2}%", eve_intercepted as f64 / generated.total_generated as f64 * 100.0);
        println!("    Throughput:  {:.2} M photons/sec",
            generated.total_generated as f64 / eve_duration.as_secs_f64() / 1_000_000.0
        );
    } else {
        println!("{}", "  Phase 2: No eavesdropper (secure channel)...".green().bold());
        photons_for_bob = generated.photons.clone();
    }
    println!();

    println!("{}", "  Phase 3: Bob measures photons with random bases...".yellow().bold());
    let measure_start = Instant::now();
    let bob_results = measure_photons_parallel(&photons_for_bob, &bob_config);
    let measure_duration = measure_start.elapsed();

    println!("    Measured:    {} photons", format_num(bob_results.received_count).white().bold());
    println!("    Lost:        {}", format_num(bob_results.lost_count).white().bold());
    println!("    Throughput:  {:.2} M measurements/sec",
        generated.total_generated as f64 / measure_duration.as_secs_f64() / 1_000_000.0
    );
    println!();

    println!("{}", "  Phase 4: Public channel - Basis reconciliation...".blue().bold());
    let recon_start = Instant::now();
    let lost_flags: Vec<bool> = generated.photons.iter().map(|p| p.lost).collect();
    let sifted_key = reconcile_bases_parallel(
        &generated.bases,
        &generated.bits,
        &bob_results.measurement_bases,
        &bob_results.measured_bits,
        &lost_flags,
    );
    let recon_duration = recon_start.elapsed();

    println!("    Sifted key:  {} bits", format_num(sifted_key.length).white().bold());
    println!("    Efficiency:  {:.2}%",
        sifted_key.length as f64 / generated.total_generated as f64 * 100.0
    );
    println!("    Throughput:  {:.2} M bits/sec",
        sifted_key.length as f64 / recon_duration.as_secs_f64() / 1_000_000.0
    );
    println!();

    println!("{}", "  Phase 5: QBER calculation and security check...".magenta().bold());
    let qber_start = Instant::now();
    let qber_result = calculate_qber_from_sifted(&sifted_key);
    let _qber_duration = qber_start.elapsed();

    let secure = check_and_alert(&qber_result);

    let total_duration = total_start.elapsed();

    println!("\n{}", "  Performance Summary:".cyan().bold());
    println!("  {}", "─".repeat(50).cyan());
    println!("    Total time:    {:.2?}", total_duration);
    println!("    Avg throughput: {:.2} M photons/sec",
        cli.num_photons as f64 / total_duration.as_secs_f64() / 1_000_000.0
    );
    println!("    Threads used:  {}", rayon::current_num_threads());
    println!();

    if !secure {
        println!("\n{}", "  KEY NEGOTIATION ABORTED - Channel compromised!".red().bold().blink());
        std::process::exit(1);
    } else {
        println!("\n{}", "  KEY AGREEMENT COMPLETE - Secure key established!".green().bold());
        println!("\n  Final key preview (first 64 bits):");
        print!("    Alice: ");
        for i in 0..sifted_key.length.min(64) {
            match sifted_key.alice_bits[i] {
                Bit::Zero => print!("{}", "0".bright_black()),
                Bit::One => print!("{}", "1".white().bold()),
            }
        }
        println!();
        print!("    Bob:   ");
        for i in 0..sifted_key.length.min(64) {
            match sifted_key.bob_bits[i] {
                Bit::Zero => print!("{}", "0".bright_black()),
                Bit::One => print!("{}", "1".white().bold()),
            }
        }
        println!();
    }
}
