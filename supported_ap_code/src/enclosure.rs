//! Outward-rounded enclosures for the prevalence search. No change to the
//! process rounding mode (searches can run concurrently).

use std::ops::{Add, Div, Mul, Neg, Sub};

#[derive(Clone, Copy, Debug)]
pub(crate) struct Enclosure {
    pub lo: f64,
    pub hi: f64,
}

// Written using bits to retain the crate's Rust 1.85 minimum version.
fn down(x: f64) -> f64 {
    if x == f64::NEG_INFINITY {
        return x;
    }
    if x == 0.0 {
        return -f64::from_bits(1);
    }
    f64::from_bits(if x > 0.0 {
        x.to_bits() - 1
    } else {
        x.to_bits() + 1
    })
}

fn up(x: f64) -> f64 {
    -down(-x)
}

impl Enclosure {
    pub const ZERO: Self = Self::point(0.0);
    pub const ONE: Self = Self::point(1.0);

    pub const fn point(x: f64) -> Self {
        Self { lo: x, hi: x }
    }

    pub fn integer(x: usize) -> Self {
        let value = x as f64;
        if x as u128 <= (1u128 << 53) {
            Self::point(value)
        } else {
            Self {
                lo: down(value),
                hi: up(value),
            }
        }
    }

    pub fn ratio(a: usize, b: usize) -> Self {
        if a == 0 {
            Self::ZERO
        } else if a == b {
            Self::ONE
        } else {
            Self::integer(a) / Self::integer(b)
        }
    }

    pub fn midpoint(self) -> f64 {
        self.lo + (self.hi - self.lo) * 0.5
    }

    pub fn hull(self, other: Self) -> Self {
        Self {
            lo: self.lo.min(other.lo),
            hi: self.hi.max(other.hi),
        }
    }

    pub fn intersect(self, other: Self) -> Self {
        let result = Self {
            lo: self.lo.max(other.lo),
            hi: self.hi.min(other.hi),
        };
        assert!(
            result.lo <= result.hi,
            "disjoint numerical enclosures: {self:?}, {other:?}"
        );
        result
    }

    pub fn nonnegative(self) -> Self {
        Self {
            lo: self.lo.max(0.0),
            hi: self.hi.max(0.0),
        }
    }
}

impl Add for Enclosure {
    type Output = Self;
    fn add(self, rhs: Self) -> Self {
        if self.lo == 0.0 && self.hi == 0.0 {
            return rhs;
        }
        if rhs.lo == 0.0 && rhs.hi == 0.0 {
            return self;
        }
        Self {
            lo: down(self.lo + rhs.lo),
            hi: up(self.hi + rhs.hi),
        }
    }
}

impl Neg for Enclosure {
    type Output = Self;
    fn neg(self) -> Self {
        Self {
            lo: -self.hi,
            hi: -self.lo,
        }
    }
}

impl Sub for Enclosure {
    type Output = Self;
    fn sub(self, rhs: Self) -> Self {
        self + -rhs
    }
}

impl Mul for Enclosure {
    type Output = Self;
    fn mul(self, rhs: Self) -> Self {
        if (self.lo == 0.0 && self.hi == 0.0) || (rhs.lo == 0.0 && rhs.hi == 0.0) {
            return Self::ZERO;
        }
        // Probabilities, denominators and weights usually have known signs.
        // Avoid four products and a reduction in these hot paths.
        if rhs.lo >= 0.0 {
            if self.lo >= 0.0 {
                return Self {
                    lo: down(self.lo * rhs.lo),
                    hi: up(self.hi * rhs.hi),
                };
            }
            if self.hi <= 0.0 {
                return Self {
                    lo: down(self.lo * rhs.hi),
                    hi: up(self.hi * rhs.lo),
                };
            }
        }
        if rhs.hi <= 0.0 {
            if self.lo >= 0.0 {
                return Self {
                    lo: down(self.hi * rhs.lo),
                    hi: up(self.lo * rhs.hi),
                };
            }
            if self.hi <= 0.0 {
                return Self {
                    lo: down(self.hi * rhs.hi),
                    hi: up(self.lo * rhs.lo),
                };
            }
        }
        let products = [
            self.lo * rhs.lo,
            self.lo * rhs.hi,
            self.hi * rhs.lo,
            self.hi * rhs.hi,
        ];
        Self {
            lo: down(products.into_iter().fold(f64::INFINITY, f64::min)),
            hi: up(products.into_iter().fold(f64::NEG_INFINITY, f64::max)),
        }
    }
}

impl Div for Enclosure {
    type Output = Self;
    fn div(self, rhs: Self) -> Self {
        assert!(
            rhs.lo > 0.0,
            "denominator must be strictly positive: {rhs:?}"
        );
        self * Self {
            lo: down(1.0 / rhs.hi),
            hi: up(1.0 / rhs.lo),
        }
    }
}
