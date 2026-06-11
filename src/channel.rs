use rayon::prelude::*;
use indicatif::{ProgressBar, ProgressStyle};
use crate::photon::{Basis, Bit};

#[derive(Debug, Clone)]
pub struct SiftedKey {
    pub alice_bits: Vec<Bit>,
    pub bob_bits: Vec<Bit>,
    pub indices: Vec<usize>,
    pub length: usize,
}

impl SiftedKey {
    pub fn len(&self) -> usize {
        self.length
    }

    pub fn is_empty(&self) -> bool {
        self.length == 0
    }
}

pub fn reconcile_bases_parallel(
    alice_bases: &[Basis],
    alice_bits: &[Bit],
    bob_bases: &[Basis],
    bob_bits: &[Bit],
    lost_flags: &[bool],
) -> SiftedKey {
    let pb = ProgressBar::new(alice_bases.len() as u64);
    pb.set_style(
        ProgressStyle::with_template("{spinner:.blue} [{elapsed_precise}] [{wide_bar:.blue/white} {pos}/{len} sifted ({eta}")
            .unwrap()
            .progress_chars("=>-"),
    );

    let chunk_size = (alice_bases.len() / rayon::current_num_threads()).max(1024);
    let num_chunks = (alice_bases.len() + chunk_size - 1) / chunk_size;

    let results: Vec<_> = (0..num_chunks)
        .into_par_iter()
        .map(|chunk_idx| {
            let start = chunk_idx * chunk_size;
            let end = (start + chunk_size).min(alice_bases.len());

            let mut alice_sifted = Vec::new();
            let mut bob_sifted = Vec::new();
            let mut indices = Vec::new();

            for i in start..end {
                if !lost_flags[i] && alice_bases[i] == bob_bases[i] {
                    alice_sifted.push(alice_bits[i]);
                    bob_sifted.push(bob_bits[i]);
                    indices.push(i);
                }
            }

            if chunk_idx % 10 == 0 {
                pb.inc(chunk_size as u64);
            }

            (alice_sifted, bob_sifted, indices)
        })
        .collect();

    pb.finish_with_message("Basis reconciliation complete");

    let mut all_alice = Vec::new();
    let mut all_bob = Vec::new();
    let mut all_indices = Vec::new();

    for (mut alice, mut bob, mut idx) in results {
        all_alice.append(&mut alice);
        all_bob.append(&mut bob);
        all_indices.append(&mut idx);
    }

    let length = all_alice.len();

    SiftedKey {
        alice_bits: all_alice,
        bob_bits: all_bob,
        indices: all_indices,
        length,
    }
}

pub fn reconcile_bases_packed(
    alice_packed_bases: &[u64],
    alice_packed_bits: &[u64],
    bob_packed_bases: &[u64],
    bob_packed_bits: &[u64],
    lost_flags: &[bool],
    total_photons: usize,
) -> (Vec<Bit>, Vec<Bit>, Vec<usize>) {
    let chunk_size = (total_photons / rayon::current_num_threads()).max(1024 * 64);
    let num_chunks = (total_photons + chunk_size - 1) / chunk_size;

    let results: Vec<_> = (0..num_chunks)
        .into_par_iter()
        .map(|chunk_idx| {
            let start = chunk_idx * chunk_size;
            let end = (start + chunk_size).min(total_photons);

            let mut alice_sifted = Vec::new();
            let mut bob_sifted = Vec::new();
            let mut indices = Vec::new();

            for i in start..end {
                let word_idx = i / 64;
                let bit_idx = i % 64;

                if lost_flags[i] {
                    continue;
                }

                let alice_basis = (alice_packed_bases[word_idx] & (1u64 << bit_idx)) != 0;
                let bob_basis = (bob_packed_bases[word_idx] & (1u64 << bit_idx)) != 0;

                if alice_basis == bob_basis {
                    let alice_bit = (alice_packed_bits[word_idx] & (1u64 << bit_idx)) != 0;
                    let bob_bit = (bob_packed_bits[word_idx] & (1u64 << bit_idx)) != 0;
                    alice_sifted.push(Bit::from(alice_bit));
                    bob_sifted.push(Bit::from(bob_bit));
                    indices.push(i);
                }
            }

            (alice_sifted, bob_sifted, indices)
        })
        .collect();

    let mut all_alice = Vec::new();
    let mut all_bob = Vec::new();
    let mut all_indices = Vec::new();

    for (mut alice, mut bob, mut idx) in results {
        all_alice.append(&mut alice);
        all_bob.append(&mut bob);
        all_indices.append(&mut idx);
    }

    (all_alice, all_bob, all_indices)
}

pub struct PublicChannel;

impl PublicChannel {
    pub fn new() -> Self {
        PublicChannel
    }

    pub fn announce_bases(&self, bases: &[Basis]) -> Vec<Basis> {
        bases.to_vec()
    }

    pub fn compare_bases(
        &self,
        alice_bases: &[Basis],
        bob_bases: &[Basis],
    ) -> Vec<bool> {
        alice_bases.iter()
            .zip(bob_bases.iter())
            .map(|(a, b)| a == b)
            .collect()
    }
}
