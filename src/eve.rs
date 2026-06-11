use rand::SeedableRng;
use rand_chacha::ChaCha8Rng;
use rand::Rng;
use rayon::prelude::*;
use indicatif::{ProgressBar, ProgressStyle};
use crate::photon::{Basis, Bit, Photon, measure_photon_fast};

#[derive(Debug, Clone)]
pub struct EveConfig {
    pub enabled: bool,
    pub interception_prob: f64,
    pub seed: u64,
}

impl Default for EveConfig {
    fn default() -> Self {
        EveConfig {
            enabled: true,
            interception_prob: 0.5,
            seed: 67890,
        }
    }
}

#[derive(Debug, Clone)]
pub struct EveResults {
    pub intercepted_count: usize,
    pub eavesdropped_bits: Vec<Bit>,
    pub eavesdropped_bases: Vec<Basis>,
    pub modified_photons: Vec<Photon>,
}

pub fn intercept_and_resend_parallel(
    photons: &[Photon],
    config: &EveConfig,
) -> (Vec<Photon>, EveResults) {
    if !config.enabled {
        return (
            photons.to_vec(),
            EveResults {
                intercepted_count: 0,
                eavesdropped_bits: Vec::new(),
                eavesdropped_bases: Vec::new(),
                modified_photons: Vec::new(),
            },
        );
    }

    let pb = ProgressBar::new(photons.len() as u64);
    pb.set_style(
        ProgressStyle::with_template("{spinner:.red} [{elapsed_precise}] [{wide_bar:.red/white} {pos}/{len} intercepted ({eta}")
            .unwrap()
            .progress_chars("=>-"),
    );

    let chunk_size = (photons.len() / rayon::current_num_threads()).max(1024);
    let num_chunks = (photons.len() + chunk_size - 1) / chunk_size;

    let results: Vec<_> = (0..num_chunks)
        .into_par_iter()
        .map(|chunk_idx| {
            let mut rng = ChaCha8Rng::seed_from_u64(config.seed.wrapping_add(chunk_idx as u64 * 0x9E3779B97F4A7C15));
            let start = chunk_idx * chunk_size;
            let end = (start + chunk_size).min(photons.len());
            let chunk = &photons[start..end];

            let mut modified_photons = Vec::with_capacity(chunk.len());
            let mut eavesdropped_bits = Vec::new();
            let mut eavesdropped_bases = Vec::new();
            let mut intercepted = 0usize;

            for photon in chunk {
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

                    eavesdropped_bits.push(measured_bit);
                    eavesdropped_bases.push(eve_basis);
                    intercepted += 1;
                } else {
                    modified_photons.push(*photon);
                }
            }

            if chunk_idx % 10 == 0 {
                pb.inc(chunk.len() as u64);
            }

            (modified_photons, eavesdropped_bits, eavesdropped_bases, intercepted)
        })
        .collect();

    pb.finish_with_message("Eve's interception complete");

    let mut all_modified = Vec::with_capacity(photons.len());
    let mut all_bits = Vec::new();
    let mut all_bases = Vec::new();
    let mut total_intercepted = 0usize;

    for (mut photons, mut bits, mut bases, intercepted) in results {
        all_modified.append(&mut photons);
        all_bits.append(&mut bits);
        all_bases.append(&mut bases);
        total_intercepted += intercepted;
    }

    (
        all_modified,
        EveResults {
            intercepted_count: total_intercepted,
            eavesdropped_bits: all_bits,
            eavesdropped_bases: all_bases,
            modified_photons: Vec::new(),
        },
    )
}

pub fn intercept_and_resend_packed(
    packed_bases: &[u64],
    packed_bits: &[u64],
    lost_flags: &[bool],
    config: &EveConfig,
    total_photons: usize,
) -> (Vec<u64>, Vec<u64>, usize) {
    if !config.enabled {
        return (packed_bases.to_vec(), packed_bits.to_vec(), 0);
    }

    let chunk_size = (total_photons / rayon::current_num_threads()).max(1024 * 64);
    let num_chunks = (total_photons + chunk_size - 1) / chunk_size;

    let results: Vec<_> = (0..num_chunks)
        .into_par_iter()
        .map(|chunk_idx| {
            let mut rng = ChaCha8Rng::seed_from_u64(config.seed.wrapping_add(chunk_idx as u64 * 0x9E3779B97F4A7C15));
            let start = chunk_idx * chunk_size;
            let end = (start + chunk_size).min(total_photons);
            let chunk_len = end - start;

            let num_u64 = (chunk_len + 63) / 64;
            let mut new_packed_bases = vec![0u64; num_u64];
            let mut new_packed_bits = vec![0u64; num_u64];
            let mut intercepted = 0usize;

            for i in 0..chunk_len {
                let global_idx = start + i;
                let word_idx = i / 64;
                let bit_idx = i % 64;
                let global_word_idx = global_idx / 64;
                let global_bit_idx = global_idx % 64;

                if lost_flags[global_idx] {
                    let basis = (packed_bases[global_word_idx] & (1u64 << global_bit_idx)) != 0;
                    let bit = (packed_bits[global_word_idx] & (1u64 << global_bit_idx)) != 0;
                    if basis {
                        new_packed_bases[word_idx] |= 1u64 << bit_idx;
                    }
                    if bit {
                        new_packed_bits[word_idx] |= 1u64 << bit_idx;
                    }
                    continue;
                }

                if rng.r#gen::<f64>() < config.interception_prob {
                    let photon_basis = (packed_bases[global_word_idx] & (1u64 << global_bit_idx)) != 0;
                    let photon_bit = (packed_bits[global_word_idx] & (1u64 << global_bit_idx)) != 0;

                    let eve_basis = rng.r#gen::<bool>();
                    let random_bit = rng.r#gen::<bool>();
                    let measured_bit = if photon_basis == eve_basis {
                        photon_bit
                    } else {
                        random_bit
                    };

                    if eve_basis {
                        new_packed_bases[word_idx] |= 1u64 << bit_idx;
                    }
                    if measured_bit {
                        new_packed_bits[word_idx] |= 1u64 << bit_idx;
                    }
                    intercepted += 1;
                } else {
                    let basis = (packed_bases[global_word_idx] & (1u64 << global_bit_idx)) != 0;
                    let bit = (packed_bits[global_word_idx] & (1u64 << global_bit_idx)) != 0;
                    if basis {
                        new_packed_bases[word_idx] |= 1u64 << bit_idx;
                    }
                    if bit {
                        new_packed_bits[word_idx] |= 1u64 << bit_idx;
                    }
                }
            }

            (new_packed_bases, new_packed_bits, intercepted)
        })
        .collect();

    let total_u64 = (total_photons + 63) / 64;
    let mut all_new_bases = Vec::with_capacity(total_u64);
    let mut all_new_bits = Vec::with_capacity(total_u64);
    let mut total_intercepted = 0usize;

    for (mut bases, mut bits, intercepted) in results {
        all_new_bases.append(&mut bases);
        all_new_bits.append(&mut bits);
        total_intercepted += intercepted;
    }

    (all_new_bases, all_new_bits, total_intercepted)
}
