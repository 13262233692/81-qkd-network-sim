use rand::SeedableRng;
use rand_chacha::ChaCha8Rng;
use rand::Rng;
use rayon::prelude::*;
use indicatif::{ProgressBar, ProgressStyle};
use crate::photon::{Basis, Bit, Photon, measure_photon_fast};

#[derive(Debug, Clone)]
pub struct BobConfig {
    pub seed: u64,
}

impl Default for BobConfig {
    fn default() -> Self {
        BobConfig {
            seed: 12345,
        }
    }
}

#[derive(Debug, Clone)]
pub struct BobResults {
    pub measurement_bases: Vec<Basis>,
    pub measured_bits: Vec<Bit>,
    pub received_count: usize,
    pub lost_count: usize,
}

impl BobResults {
    pub fn len(&self) -> usize {
        self.measured_bits.len()
    }
}

pub fn measure_photons_parallel(
    photons: &[Photon],
    config: &BobConfig,
) -> BobResults {
    let pb = ProgressBar::new(photons.len() as u64);
    pb.set_style(
        ProgressStyle::with_template("{spinner:.yellow} [{elapsed_precise}] [{wide_bar:.yellow/blue} {pos}/{len} measurements ({eta}")
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

            let mut measurement_bases = Vec::with_capacity(chunk.len());
            let mut measured_bits = Vec::with_capacity(chunk.len());
            let mut received = 0usize;
            let mut lost = 0usize;

            for photon in chunk {
                let measurement_basis = if rng.r#gen::<bool>() { Basis::Rectilinear } else { Basis::Diagonal };
                measurement_bases.push(measurement_basis);

                if photon.lost {
                    measured_bits.push(Bit::Zero);
                    lost += 1;
                } else {
                    let random_bit = rng.r#gen::<bool>();
                    let measured_bit = measure_photon_fast(photon.basis, photon.bit, measurement_basis, random_bit);
                    measured_bits.push(measured_bit);
                    received += 1;
                }
            }

            if chunk_idx % 10 == 0 {
                pb.inc(chunk.len() as u64);
            }

            (measurement_bases, measured_bits, received, lost)
        })
        .collect();

    pb.finish_with_message("Bob's measurements complete");

    let mut all_bases = Vec::with_capacity(photons.len());
    let mut all_bits = Vec::with_capacity(photons.len());
    let mut total_received = 0usize;
    let mut total_lost = 0usize;

    for (mut bases, mut bits, received, lost) in results {
        all_bases.append(&mut bases);
        all_bits.append(&mut bits);
        total_received += received;
        total_lost += lost;
    }

    BobResults {
        measurement_bases: all_bases,
        measured_bits: all_bits,
        received_count: total_received,
        lost_count: total_lost,
    }
}

pub fn measure_photons_packed(
    packed_bases: &[u64],
    packed_bits: &[u64],
    lost_flags: &[bool],
    config: &BobConfig,
    total_photons: usize,
) -> (Vec<u64>, Vec<u64>, usize, usize) {
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
            let mut packed_meas_bases = vec![0u64; num_u64];
            let mut packed_meas_bits = vec![0u64; num_u64];
            let mut received = 0usize;
            let mut lost = 0usize;

            for i in 0..chunk_len {
                let global_idx = start + i;
                let word_idx = i / 64;
                let bit_idx = i % 64;
                let global_word_idx = global_idx / 64;
                let global_bit_idx = global_idx % 64;

                let photon_basis = (packed_bases[global_word_idx] & (1u64 << global_bit_idx)) != 0;
                let photon_bit = (packed_bits[global_word_idx] & (1u64 << global_bit_idx)) != 0;
                let photon_lost = lost_flags[global_idx];

                let meas_basis = rng.r#gen::<bool>();

                if meas_basis {
                    packed_meas_bases[word_idx] |= 1u64 << bit_idx;
                }

                if !photon_lost {
                    let random_bit = rng.r#gen::<bool>();
                    let result_bit = if photon_basis == meas_basis {
                        photon_bit
                    } else {
                        random_bit
                    };
                    if result_bit {
                        packed_meas_bits[word_idx] |= 1u64 << bit_idx;
                    }
                    received += 1;
                } else {
                    lost += 1;
                }
            }

            (packed_meas_bases, packed_meas_bits, received, lost)
        })
        .collect();

    let total_u64 = (total_photons + 63) / 64;
    let mut all_packed_meas_bases = Vec::with_capacity(total_u64);
    let mut all_packed_meas_bits = Vec::with_capacity(total_u64);
    let mut total_received = 0usize;
    let mut total_lost = 0usize;

    for (mut meas_bases, mut meas_bits, received, lost) in results {
        all_packed_meas_bases.append(&mut meas_bases);
        all_packed_meas_bits.append(&mut meas_bits);
        total_received += received;
        total_lost += lost;
    }

    (all_packed_meas_bases, all_packed_meas_bits, total_received, total_lost)
}
