use rand::SeedableRng;
use rand_chacha::ChaCha8Rng;
use rand::Rng;
use rayon::prelude::*;
use indicatif::{ProgressBar, ProgressStyle};
use crate::photon::{Basis, Bit, Photon};

#[derive(Debug, Clone)]
pub struct AliceConfig {
    pub num_photons: usize,
    pub attenuation_prob: f64,
    pub seed: u64,
}

impl Default for AliceConfig {
    fn default() -> Self {
        AliceConfig {
            num_photons: 1_000_000,
            attenuation_prob: 0.01,
            seed: 42,
        }
    }
}

#[derive(Debug, Clone)]
pub struct GeneratedPhotons {
    pub photons: Vec<Photon>,
    pub bases: Vec<Basis>,
    pub bits: Vec<Bit>,
    pub total_lost: usize,
    pub total_generated: usize,
}

impl GeneratedPhotons {
    pub fn len(&self) -> usize {
        self.photons.len()
    }
}

pub fn generate_photons_parallel(config: &AliceConfig) -> GeneratedPhotons {
    let pb = ProgressBar::new(config.num_photons as u64);
    pb.set_style(
        ProgressStyle::with_template("{spinner:.green} [{elapsed_precise}] [{wide_bar:.cyan/blue} {pos}/{len} photons ({eta}")
            .unwrap()
            .progress_chars("=>-"),
    );

    let chunk_size = (config.num_photons / rayon::current_num_threads()).max(1024);
    let num_chunks = (config.num_photons + chunk_size - 1) / chunk_size;

    let results: Vec<_> = (0..num_chunks)
        .into_par_iter()
        .map(|chunk_idx| {
            let mut rng = ChaCha8Rng::seed_from_u64(config.seed.wrapping_add(chunk_idx as u64 * 0x9E3779B97F4A7C15));
            let start = chunk_idx * chunk_size;
            let end = (start + chunk_size).min(config.num_photons);
            let chunk_len = end - start;

            let mut photons = Vec::with_capacity(chunk_len);
            let mut bases = Vec::with_capacity(chunk_len);
            let mut bits = Vec::with_capacity(chunk_len);
            let lost = 0usize;

            for _ in 0..chunk_len {
                let basis = if rng.r#gen::<bool>() { Basis::Rectilinear } else { Basis::Diagonal };
                let bit = Bit::from(rng.r#gen::<bool>());
                let lost_photon = rng.r#gen::<f64>() < config.attenuation_prob;
                let mut photon = Photon::new(basis, bit);
                photon.lost = lost_photon;

                photons.push(photon);
                bases.push(basis);
                bits.push(bit);
            }

            if chunk_idx % 10 == 0 {
                pb.inc(chunk_len as u64);
            }

            (photons, bases, bits, lost)
        })
        .collect();

    pb.finish_with_message("Photon generation complete");

    let mut all_photons = Vec::with_capacity(config.num_photons);
    let mut all_bases = Vec::with_capacity(config.num_photons);
    let mut all_bits = Vec::with_capacity(config.num_photons);
    let mut total_lost = 0usize;

    for (mut photons, mut bases, mut bits, lost) in results {
        all_photons.append(&mut photons);
        all_bases.append(&mut bases);
        all_bits.append(&mut bits);
        total_lost += lost;
    }

    GeneratedPhotons {
        photons: all_photons,
        bases: all_bases,
        bits: all_bits,
        total_lost,
        total_generated: config.num_photons,
    }
}

pub fn generate_photons_packed(config: &AliceConfig) -> (Vec<u64>, Vec<u64>, Vec<bool>, usize) {
    let chunk_size = (config.num_photons / rayon::current_num_threads()).max(1024 * 64);
    let num_chunks = (config.num_photons + chunk_size - 1) / chunk_size;

    let results: Vec<_> = (0..num_chunks)
        .into_par_iter()
        .map(|chunk_idx| {
            let mut rng = ChaCha8Rng::seed_from_u64(config.seed.wrapping_add(chunk_idx as u64 * 0x9E3779B97F4A7C15));
            let start = chunk_idx * chunk_size;
            let end = (start + chunk_size).min(config.num_photons);
            let chunk_len = end - start;

            let num_u64 = (chunk_len + 63) / 64;
            let mut packed_bases = vec![0u64; num_u64];
            let mut packed_bits = vec![0u64; num_u64];
            let mut lost_flags = vec![false; chunk_len];
            let lost = 0usize;

            for i in 0..chunk_len {
                let word_idx = i / 64;
                let bit_idx = i % 64;

                let basis = rng.r#gen::<bool>();
                let bit = rng.r#gen::<bool>();
                let lost_photon = rng.r#gen::<f64>() < config.attenuation_prob;

                if basis {
                    packed_bases[word_idx] |= 1u64 << bit_idx;
                }
                if bit {
                    packed_bits[word_idx] |= 1u64 << bit_idx;
                }
                lost_flags[i] = lost_photon;
            }

            (packed_bases, packed_bits, lost_flags, lost)
        })
        .collect();

    let total_u64 = (config.num_photons + 63) / 64;
    let mut all_packed_bases = Vec::with_capacity(total_u64);
    let mut all_packed_bits = Vec::with_capacity(total_u64);
    let mut all_lost_flags = Vec::with_capacity(config.num_photons);
    let mut total_lost = 0usize;

    for (mut packed_bases, mut packed_bits, mut lost_flags, lost) in results {
        all_packed_bases.append(&mut packed_bases);
        all_packed_bits.append(&mut packed_bits);
        all_lost_flags.append(&mut lost_flags);
        total_lost += lost;
    }

    (all_packed_bases, all_packed_bits, all_lost_flags, total_lost)
}
