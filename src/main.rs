mod photon;
mod pipeline;
mod qber;

use std::time::Instant;
use colored::*;
use clap::Parser;

use pipeline::{PipelineConfig, run_pipeline, compute_qber_from_sifted, CHANNEL_CAPACITY, BATCH_SIZE};
use qber::check_and_alert;
use photon::Bit;

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

    #[arg(short = 'p', long = "interception-prob", default_value_t = 0.5)]
    interception_prob: f64,

    #[arg(short = 's', long = "seed", default_value_t = 42)]
    seed: u64,

    #[arg(long = "no-eve", default_value_t = false)]
    no_eve: bool,

    #[arg(short = 'c', long = "channel-capacity", default_value_t = CHANNEL_CAPACITY)]
    channel_capacity: usize,

    #[arg(short = 'b', long = "batch-size", default_value_t = BATCH_SIZE)]
    batch_size: usize,
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
    println!("{}", "     BB84 Protocol · Bounded Channel Backpressure Pipeline".yellow());
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
    println!("{}", "  Pipeline Configuration:".magenta().bold());
    println!("  {}", "─".repeat(50).magenta());
    println!("    Channel capacity:        {} batches", cli.channel_capacity.to_string().white().bold());
    println!("    Batch size:              {} photons/batch", format_num(cli.batch_size).white().bold());
    println!("    Max in-flight memory:    ~{:.1} MB",
        (cli.channel_capacity * 2 * cli.batch_size * std::mem::size_of::<photon::Photon>()) as f64 / 1_048_576.0
    );
    println!("    Backpressure:            {}",
        "sync_channel BLOCKING".bright_yellow().bold()
    );
    println!();
}

fn main() {
    let cli = Cli::parse();
    let eve_enabled = !cli.no_eve;

    print_header();
    print_config(&cli);

    println!("{}", "  Launching bounded streaming pipeline...".cyan().bold());
    println!("  {}", "━".repeat(50).cyan());
    println!();

    let total_start = Instant::now();

    let pipeline_config = PipelineConfig {
        num_photons: cli.num_photons,
        attenuation_prob: cli.attenuation,
        eve_enabled,
        interception_prob: cli.interception_prob,
        alice_seed: cli.seed,
        bob_seed: cli.seed.wrapping_add(12345),
        eve_seed: cli.seed.wrapping_add(67890),
    };

    let stats = run_pipeline(&pipeline_config);

    println!();
    println!("{}", "  Phase 1: Alice → Photon Generation".cyan().bold());
    println!("    Generated:   {} photons", format_num(stats.total_generated).white().bold());
    println!("    Lost in fiber: {} ({:.2}%)",
        format_num(stats.total_lost_fiber).white().bold(),
        stats.total_lost_fiber as f64 / stats.total_generated as f64 * 100.0
    );
    println!("    Throughput:  {:.2} M photons/sec",
        stats.total_generated as f64 / stats.alice_elapsed.as_secs_f64() / 1_000_000.0
    );
    println!();

    if eve_enabled {
        println!("{}", "  Phase 2: Eve → Intercept & Resend".red().bold());
        println!("    Intercepted: {} photons", format_num(stats.total_intercepted).white().bold());
        println!("    Rate:        {:.2}%",
            stats.total_intercepted as f64 / stats.total_generated as f64 * 100.0
        );
        println!("    Throughput:  {:.2} M photons/sec",
            stats.total_generated as f64 / stats.eve_elapsed.as_secs_f64() / 1_000_000.0
        );
    } else {
        println!("{}", "  Phase 2: No eavesdropper (secure channel)".green().bold());
    }
    println!();

    println!("{}", "  Phase 3: Bob → Measurement".yellow().bold());
    println!("    Measured:    {} photons", format_num(stats.total_received).white().bold());
    println!("    Lost:        {}", format_num(stats.total_lost_total).white().bold());
    println!("    Throughput:  {:.2} M measurements/sec",
        stats.total_generated as f64 / stats.bob_elapsed.as_secs_f64() / 1_000_000.0
    );
    println!();

    println!("{}", "  Phase 4: Basis Reconciliation → Sifted Key".blue().bold());
    println!("    Sifted key:  {} bits", format_num(stats.sifted_key_length).white().bold());
    println!("    Efficiency:  {:.2}%",
        stats.sifted_key_length as f64 / stats.total_generated as f64 * 100.0
    );
    println!();

    println!("{}", "  Phase 5: QBER Calculation & Security Check".magenta().bold());
    let qber_result = compute_qber_from_sifted(&stats.sifted_key_alice, &stats.sifted_key_bob);
    let secure = check_and_alert(&qber_result);

    let total_duration = total_start.elapsed();

    println!("\n{}", "  Performance Summary:".cyan().bold());
    println!("  {}", "─".repeat(50).cyan());
    println!("    Total time:    {:.2?}", total_duration);
    println!("    Avg throughput: {:.2} M photons/sec",
        cli.num_photons as f64 / total_duration.as_secs_f64() / 1_000_000.0
    );
    println!("    Threads used:  {}", rayon::current_num_threads());
    println!("    Pipeline:      {}",
        "sync_channel bounded (backpressure)".bright_green().bold()
    );
    println!();

    if !secure {
        println!("\n{}", "  KEY NEGOTIATION ABORTED - Channel compromised!".red().bold().blink());
        std::process::exit(1);
    } else {
        println!("\n{}", "  KEY AGREEMENT COMPLETE - Secure key established!".green().bold());
        println!("\n  Final key preview (first 64 bits):");
        print!("    Alice: ");
        for i in 0..stats.sifted_key_length.min(64) {
            match stats.sifted_key_alice[i] {
                Bit::Zero => print!("{}", "0".bright_black()),
                Bit::One => print!("{}", "1".white().bold()),
            }
        }
        println!();
        print!("    Bob:   ");
        for i in 0..stats.sifted_key_length.min(64) {
            match stats.sifted_key_bob[i] {
                Bit::Zero => print!("{}", "0".bright_black()),
                Bit::One => print!("{}", "1".white().bold()),
            }
        }
        println!();
    }
}
