use std::fmt;
use rand::Rng;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Basis {
    Rectilinear,
    Diagonal,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Bit {
    Zero,
    One,
}

impl From<bool> for Bit {
    fn from(b: bool) -> Self {
        if b { Bit::One } else { Bit::Zero }
    }
}

#[derive(Debug, Clone, Copy)]
pub struct Photon {
    pub basis: Basis,
    pub bit: Bit,
    pub lost: bool,
}

impl Photon {
    pub fn new(basis: Basis, bit: Bit) -> Self {
        Photon {
            basis,
            bit,
            lost: false,
        }
    }

    pub fn measure<R: Rng>(&self, measurement_basis: Basis, rng: &mut R) -> Option<Bit> {
        if self.lost {
            return None;
        }
        if self.basis == measurement_basis {
            Some(self.bit)
        } else {
            Some(Bit::from(rng.r#gen::<bool>()))
        }
    }
}

impl fmt::Display for Basis {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Basis::Rectilinear => write!(f, "Z"),
            Basis::Diagonal => write!(f, "X"),
        }
    }
}

impl fmt::Display for Bit {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Bit::Zero => write!(f, "0"),
            Bit::One => write!(f, "1"),
        }
    }
}

#[inline]
pub fn measure_photon<R: Rng>(photon_basis: Basis, photon_bit: Bit, measurement_basis: Basis, rng: &mut R) -> Bit {
    if photon_basis == measurement_basis {
        photon_bit
    } else {
        Bit::from(rng.r#gen::<bool>())
    }
}

#[inline]
pub fn measure_photon_fast(photon_basis: Basis, photon_bit: Bit, measurement_basis: Basis, random_bit: bool) -> Bit {
    if photon_basis == measurement_basis {
        photon_bit
    } else {
        Bit::from(random_bit)
    }
}
