use std::fmt::Write;
use rand::SeedableRng;
use rand_chacha::ChaCha8Rng;
use rand::Rng;
use rayon::prelude::*;
use indicatif::{ProgressBar, ProgressStyle};
use colored::*;

use crate::photon::Bit;

#[derive(Debug, Clone)]
pub struct LdpcConfig {
    pub enabled: bool,
    pub max_iterations: usize,
    pub seed: u64,
}

impl Default for LdpcConfig {
    fn default() -> Self {
        LdpcConfig {
            enabled: true,
            max_iterations: 10,
            seed: 99999,
        }
    }
}

#[derive(Debug, Clone)]
pub struct AmplificationConfig {
    pub compression_ratio: f64,
    pub seed: u64,
}

impl Default for AmplificationConfig {
    fn default() -> Self {
        AmplificationConfig {
            compression_ratio: 0.5,
            seed: 77777,
        }
    }
}

#[derive(Debug, Clone)]
pub struct PrivacyAmplificationStats {
    pub sifted_key_length: usize,
    pub corrected_key_length: usize,
    pub corrected_errors: usize,
    pub toeplitz_input_bits: usize,
    pub toeplitz_output_bits: usize,
    pub toeplitz_diagonal_bits: usize,
    pub final_key_bytes: usize,
}

pub fn simulate_ldpc_correction(
    alice_bits: &[Bit],
    bob_bits: &mut [Bit],
    config: &LdpcConfig,
) -> usize {
    if !config.enabled {
        let mut errors = 0usize;
        for i in 0..alice_bits.len().min(bob_bits.len()) {
            if alice_bits[i] != bob_bits[i] {
                errors += 1;
            }
        }
        return errors;
    }

    let len = alice_bits.len().min(bob_bits.len());
    let mut total_corrected = 0usize;
    let pb = ProgressBar::new(len as u64);
    pb.set_style(
        ProgressStyle::with_template("{spinner:.magenta} [{elapsed_precise}] [{wide_bar:.magenta/white} {pos}/{len} corrected ({eta}")
            .unwrap()
            .progress_chars("=>-"),
    );

    let chunk_size = (len / rayon::current_num_threads()).max(4096);
    let num_chunks = (len + chunk_size - 1) / chunk_size;

    let corrections: Vec<Vec<(usize, Bit)>> = (0..num_chunks)
        .into_par_iter()
        .map(|chunk_idx| {
            let mut rng = ChaCha8Rng::seed_from_u64(config.seed.wrapping_add(chunk_idx as u64 * 0x9E3779B97F4A7C15));
            let start = chunk_idx * chunk_size;
            let end = (start + chunk_size).min(len);

            let syndrome_bits: Vec<bool> = (0..end - start)
                .map(|_| rng.r#gen::<bool>())
                .collect();

            let mut corrections = Vec::new();
            for i in start..end {
                if alice_bits[i] != bob_bits[i] {
                    let local_idx = i - start;
                    if syndrome_bits[local_idx] || rng.r#gen::<f64>() < 0.85 {
                        corrections.push((i, alice_bits[i]));
                    }
                }
            }
            corrections
        })
        .collect();

    let mut applied = 0usize;
    for chunk_corrections in corrections {
        for (idx, correct_bit) in chunk_corrections {
            if bob_bits[idx] != correct_bit {
                bob_bits[idx] = correct_bit;
                total_corrected += 1;
            }
            applied += 1;
            if applied % 10000 == 0 {
                pb.inc(10000);
            }
        }
    }
    pb.finish_and_clear();

    total_corrected
}

pub struct ToeplitzMatrix {
    pub m: usize,
    pub n: usize,
    pub diagonal: Vec<u64>,
    pub diagonal_len: usize,
}

impl ToeplitzMatrix {
    pub fn generate(m: usize, n: usize, seed: u64) -> Self {
        let diagonal_len = m + n - 1;
        let num_words = (diagonal_len + 63) / 64;

        let pb = ProgressBar::new(num_words as u64);
        pb.set_style(
            ProgressStyle::with_template("{spinner:.bright_magenta} [{elapsed_precise}] [{wide_bar:.bright_magenta/white} {pos}/{len} Toeplitz words ({eta}")
                .unwrap()
                .progress_chars("=>-"),
        );

        let chunk_size = (num_words / rayon::current_num_threads()).max(128);
        let num_chunks = (num_words + chunk_size - 1) / chunk_size;

        let diagonal: Vec<Vec<u64>> = (0..num_chunks)
            .into_par_iter()
            .map(|chunk_idx| {
                let mut rng = ChaCha8Rng::seed_from_u64(seed.wrapping_add(chunk_idx as u64 * 0x9E3779B97F4A7C15));
                let start = chunk_idx * chunk_size;
                let end = (start + chunk_size).min(num_words);
                let chunk_len = end - start;
                let mut chunk = Vec::with_capacity(chunk_len);
                for _ in 0..chunk_len {
                    chunk.push(rng.r#gen::<u64>());
                }
                pb.inc(chunk_len as u64);
                chunk
            })
            .collect();

        pb.finish_with_message("Toeplitz matrix generated");

        let mut all_diagonal = Vec::with_capacity(num_words);
        for mut chunk in diagonal {
            all_diagonal.append(&mut chunk);
        }

        ToeplitzMatrix {
            m,
            n,
            diagonal: all_diagonal,
            diagonal_len,
        }
    }

    #[inline(always)]
    fn get_diagonal_bit(&self, k: isize) -> bool {
        let idx = k + (self.n as isize - 1);
        if idx < 0 || idx as usize >= self.diagonal_len {
            return false;
        }
        let idx_u = idx as usize;
        let word_idx = idx_u >> 6;
        let bit_idx = idx_u & 63;
        (self.diagonal[word_idx] & (1u64 << bit_idx)) != 0
    }

    pub fn multiply_gf2(&self, input: &[u64], input_len_bits: usize) -> Vec<u64> {
        let m = self.m;
        let n = self.n.min(input_len_bits);
        let output_words = (m + 63) / 64;
        let diag_total = self.diagonal_len;
        let diag_offset = self.n - 1;

        let pb = ProgressBar::new(diag_total as u64);
        pb.set_style(
            ProgressStyle::with_template("{spinner:.bright_green} [{elapsed_precise}] [{wide_bar:.bright_green/white} {pos}/{len} diagonal bits ({eta}")
                .unwrap()
                .progress_chars("=>-"),
        );

        let num_threads = rayon::current_num_threads();
        let diag_words = self.diagonal.len();
        let chunk_dw = (diag_words / num_threads).max(8);
        let num_chunks = (diag_words + chunk_dw - 1) / chunk_dw;

        let outputs: Vec<Vec<u64>> = (0..num_chunks)
            .into_par_iter()
            .map(|chunk_idx| {
                let start_dw = chunk_idx * chunk_dw;
                let end_dw = (start_dw + chunk_dw).min(diag_words);
                let mut out = vec![0u64; output_words];

                for dw in start_dw..end_dw {
                    let dword = self.diagonal[dw];
                    if dword == 0 {
                        pb.inc(64);
                        continue;
                    }
                    for b in 0..64u32 {
                        if (dword & (1u64 << b)) == 0 {
                            continue;
                        }
                        let diag_idx = dw * 64 + b as usize;
                        let shift = diag_idx as isize - diag_offset as isize;

                        if shift >= 0 {
                            let shift_right = shift as usize;
                            if shift_right < n {
                                self.apply_shift_right_xor(&mut out, input, shift_right, n, m);
                            }
                        } else {
                            let shift_left = (-shift) as usize;
                            if shift_left < m {
                                self.apply_shift_left_xor(&mut out, input, shift_left, n, m);
                            }
                        }
                    }
                    pb.inc(64);
                }
                out
            })
            .collect();

        pb.finish_with_message("Toeplitz hash complete");

        let mut final_output = vec![0u64; output_words];
        for c in &outputs {
            for (i, w) in c.iter().enumerate() {
                final_output[i] ^= w;
            }
        }
        final_output
    }

    #[inline(always)]
    fn apply_shift_right_xor(&self, out: &mut [u64], input: &[u64], shift_right: usize, n: usize, m: usize) {
        let input_words = input.len();
        let output_words = out.len();
        let start_word = shift_right >> 6;
        let bit_shift = shift_right & 63;

        if bit_shift == 0 {
            for i in 0..output_words {
                let src = start_word + i;
                if src < input_words && i * 64 < m {
                    let mut w = input[src];
                    if src == input_words - 1 {
                        let rem = n - src * 64;
                        if rem < 64 {
                            w &= (1u64 << rem) - 1;
                        }
                    }
                    out[i] ^= w;
                }
            }
        } else {
            let inv_shift = 64 - bit_shift;
            for i in 0..output_words {
                let src0 = start_word + i;
                let src1 = src0 + 1;
                let mut w = 0u64;
                if src0 < input_words {
                    let mut w0 = input[src0];
                    if src0 == input_words - 1 {
                        let rem = n - src0 * 64;
                        if rem < 64 {
                            w0 &= (1u64 << rem) - 1;
                        }
                    }
                    w |= w0 >> bit_shift;
                }
                if src1 < input_words {
                    let mut w1 = input[src1];
                    if src1 == input_words - 1 {
                        let rem = n - src1 * 64;
                        if rem < 64 {
                            w1 &= (1u64 << rem) - 1;
                        }
                    }
                    w |= w1 << inv_shift;
                }
                let remaining = m - i * 64;
                if remaining < 64 {
                    w &= (1u64 << remaining) - 1;
                }
                out[i] ^= w;
            }
        }
    }

    #[inline(always)]
    fn apply_shift_left_xor(&self, out: &mut [u64], input: &[u64], shift_left: usize, _n: usize, m: usize) {
        let output_words = out.len();
        let input_words = input.len();
        let start_word = shift_left >> 6;
        let bit_shift = shift_left & 63;

        if bit_shift == 0 {
            for i in start_word..output_words {
                let src = i - start_word;
                if src < input_words {
                    out[i] ^= input[src];
                }
            }
        } else {
            let inv_shift = 64 - bit_shift;
            if start_word < output_words {
                let mut w0 = input[0] >> bit_shift;
                let remaining = m - start_word * 64;
                if remaining < 64 {
                    w0 &= (1u64 << remaining) - 1;
                }
                out[start_word] ^= w0;
            }
            for i in (start_word + 1)..output_words {
                let src0 = i - start_word - 1;
                let src1 = src0 + 1;
                let mut w = 0u64;
                if src0 < input_words {
                    w |= input[src0] << inv_shift;
                }
                if src1 < input_words {
                    w |= input[src1] >> bit_shift;
                }
                let remaining = m - i * 64;
                if remaining < 64 {
                    w &= (1u64 << remaining) - 1;
                }
                out[i] ^= w;
            }
        }
    }
}

pub fn bits_to_packed_words(bits: &[Bit]) -> (Vec<u64>, usize) {
    let len = bits.len();
    let num_words = (len + 63) / 64;
    let mut packed = vec![0u64; num_words];

    let chunk_size = (len / rayon::current_num_threads()).max(4096);
    let num_chunks = (len + chunk_size - 1) / chunk_size;

    let chunks: Vec<Vec<(usize, u64)>> = (0..num_chunks)
        .into_par_iter()
        .map(|chunk_idx| {
            let start = chunk_idx * chunk_size;
            let end = (start + chunk_size).min(len);
            let mut updates = Vec::new();
            let mut word: u64 = 0;
            let mut current_word_idx = usize::MAX;

            for i in start..end {
                let word_idx = i / 64;
                let bit_idx = i % 64;
                if word_idx != current_word_idx {
                    if current_word_idx != usize::MAX {
                        updates.push((current_word_idx, word));
                    }
                    current_word_idx = word_idx;
                    word = 0;
                }
                match bits[i] {
                    Bit::One => word |= 1u64 << bit_idx,
                    Bit::Zero => {}
                }
            }
            if current_word_idx != usize::MAX {
                updates.push((current_word_idx, word));
            }
            updates
        })
        .collect();

    for chunk in chunks {
        for (word_idx, word) in chunk {
            packed[word_idx] |= word;
        }
    }

    (packed, len)
}

pub fn packed_words_to_hex(words: &[u64], output_bits: usize) -> String {
    let mut hex = String::with_capacity((output_bits + 3) / 4 + 2);
    let total_bits = output_bits.min(words.len() * 64);

    for bit_idx in (0..total_bits).step_by(4) {
        let word_idx = bit_idx / 64;
        let local_bit = bit_idx % 64;
        let bits_left = total_bits - bit_idx;
        let bits_to_take = bits_left.min(4);

        let mut nibble: u8 = 0;
        for b in 0..bits_to_take {
            if local_bit + b < 64 && word_idx < words.len() {
                if (words[word_idx] & (1u64 << (local_bit + b))) != 0 {
                    nibble |= 1u8 << b;
                }
            }
        }
        let _ = write!(hex, "{:01x}", nibble);
    }
    hex
}

pub fn run_privacy_amplification_pipeline(
    alice_sifted: &[Bit],
    bob_sifted: &[Bit],
    ldpc_config: &LdpcConfig,
    amp_config: &AmplificationConfig,
) -> (String, String, PrivacyAmplificationStats, usize) {
    let sifted_len = alice_sifted.len().min(bob_sifted.len());
    println!("  {}", "─".repeat(50).magenta());

    println!("\n{}", "  Phase 6a: LDPC Error Correction".magenta().bold());
    let ldpc_start = std::time::Instant::now();
    let mut bob_corrected: Vec<Bit> = bob_sifted[..sifted_len].to_vec();
    let corrected = simulate_ldpc_correction(&alice_sifted[..sifted_len], &mut bob_corrected, ldpc_config);

    let mut remaining_errors = 0usize;
    for i in 0..sifted_len {
        if alice_sifted[i] != bob_corrected[i] {
            remaining_errors += 1;
        }
    }
    println!("    Corrected errors:   {}", corrected.to_string().white().bold());
    println!("    Remaining errors:   {}", remaining_errors.to_string().white().bold());
    println!("    LDPC time:          {:.2?}", ldpc_start.elapsed());

    println!("\n{}", "  Phase 6b: Toeplitz Universal Hash (Privacy Amplification)".bright_magenta().bold());
    let amp_start = std::time::Instant::now();

    let input_bits = sifted_len;
    let output_bits = (input_bits as f64 * amp_config.compression_ratio) as usize;
    let output_bits = output_bits.max(256).min(input_bits);

    println!("    Input key length:   {} bits", format_num(input_bits).white().bold());
    println!("    Output key length:  {} bits ({} bytes)",
        format_num(output_bits).white().bold(),
        format_num((output_bits + 7) / 8).white().bold()
    );
    println!("    Compression ratio:  {:.1}%", amp_config.compression_ratio * 100.0);
    println!();

    println!("    Generating Toeplitz matrix ({}×{})...", format_num(output_bits), format_num(input_bits));
    let toeplitz = ToeplitzMatrix::generate(output_bits, input_bits, amp_config.seed);
    println!("    Diagonal entries:   {} bits ({} MB)",
        format_num(toeplitz.diagonal_len).white().bold(),
        format!("{:.2}", toeplitz.diagonal.len() as f64 * 8.0 / 1_048_576.0).white().bold()
    );

    println!();
    println!("    Packing key bits for GF(2) multiplication...");
    let (alice_packed, _) = bits_to_packed_words(&alice_sifted[..sifted_len]);
    let (bob_packed, _) = bits_to_packed_words(&bob_corrected);

    println!();
    println!("    Alice: Toeplitz × Key vector (GF(2) XOR multiplication)...");
    let alice_final_packed = toeplitz.multiply_gf2(&alice_packed, sifted_len);
    let alice_final_hex = packed_words_to_hex(&alice_final_packed, output_bits);

    println!();
    println!("    Bob:   Toeplitz × Key vector (GF(2) XOR multiplication)...");
    let bob_final_packed = toeplitz.multiply_gf2(&bob_packed, sifted_len);
    let bob_final_hex = packed_words_to_hex(&bob_final_packed, output_bits);

    let amp_duration = amp_start.elapsed();
    println!("    Amplification time: {:.2?}", amp_duration);

    let stats = PrivacyAmplificationStats {
        sifted_key_length: sifted_len,
        corrected_key_length: sifted_len,
        corrected_errors: corrected,
        toeplitz_input_bits: input_bits,
        toeplitz_output_bits: output_bits,
        toeplitz_diagonal_bits: toeplitz.diagonal_len,
        final_key_bytes: (output_bits + 7) / 8,
    };

    (alice_final_hex, bob_final_hex, stats, remaining_errors)
}

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

pub fn print_final_keys(alice_hex: &str, bob_hex: &str, stats: &PrivacyAmplificationStats) {
    println!("\n{}", "╔══════════════════════════════════════════════════════════════╗".bright_green().bold());
    println!("{}", "║       FINAL QUANTUM KEY — PRIVACY AMPLIFICATION COMPLETE     ║".bright_green().bold());
    println!("{}", "╚══════════════════════════════════════════════════════════════╝".bright_green().bold());

    println!("\n{}", "  ═══ Key Statistics ═══".bright_white().bold());
    println!("    Sifted key bits:       {}", format_num(stats.sifted_key_length));
    println!("    LDPC corrected bits:   {}", format_num(stats.corrected_errors));
    println!("    Toeplitz diagonal:     {} bits", format_num(stats.toeplitz_diagonal_bits));
    println!("    Input bits hashed:     {}", format_num(stats.toeplitz_input_bits));
    println!("    Output bits (final):   {} ({} bytes)",
        format_num(stats.toeplitz_output_bits),
        format_num(stats.final_key_bytes)
    );

    println!("\n{}", "  ═══ Alice's Final Quantum Key Stream ═══".cyan().bold());
    println!("  {}", "─".repeat(60).cyan());
    print_hex_wrapped(alice_hex, "cyan");

    println!("\n{}", "  ═══ Bob's Final Quantum Key Stream ═══".yellow().bold());
    println!("  {}", "─".repeat(60).yellow());
    print_hex_wrapped(bob_hex, "yellow");

    let keys_match = alice_hex == bob_hex;
    println!("\n{}", "  ═══ Key Verification ═══".bright_white().bold());
    if keys_match {
        println!("  {} Keys are IDENTICAL — Quantum key agreement SUCCESS", "✓".green().bold().blink());
    } else {
        println!("  {} Keys MISMATCH — Errors remain after LDPC", "✗".red().bold().blink());
        let mut diff = 0usize;
        let min_len = alice_hex.len().min(bob_hex.len());
        for i in 0..min_len {
            if alice_hex.as_bytes()[i] != bob_hex.as_bytes()[i] {
                diff += 1;
            }
        }
        println!("    Hex nibbles differing: {}", diff);
    }
    println!();
}

fn print_hex_wrapped(hex: &str, color: &str) {
    let bytes_per_line = 32;
    let chars_per_line = bytes_per_line * 2;

    for (line_idx, chunk) in hex.as_bytes().chunks(chars_per_line).enumerate() {
        let offset = format!("{:08x}", line_idx * bytes_per_line);
        let chunk_str = std::str::from_utf8(chunk).unwrap_or("");

        let mut spaced = String::with_capacity(chunk_str.len() + chunk_str.len() / 2);
        for (i, c) in chunk_str.chars().enumerate() {
            if i > 0 && i % 2 == 0 {
                spaced.push(' ');
            }
            spaced.push(c);
        }

        match color {
            "cyan" => {
                println!("    {}  {}", offset.cyan(), spaced.cyan());
            }
            "yellow" => {
                println!("    {}  {}", offset.yellow(), spaced.yellow());
            }
            _ => {
                println!("    {}  {}", offset, spaced);
            }
        }
    }
}
