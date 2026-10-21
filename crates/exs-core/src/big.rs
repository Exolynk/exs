//! Non-negative big integers for exact decimal conversion of binary64 values without lookup
//! tables: parsing with `Float::parse` and formatting Floats.

use core::cmp::Ordering;

/// `u32` limbs of one big integer, enough for every intermediate value of both conversions.
const LIMBS: usize = 170;

/// A non-negative big integer of little-endian `u32` limbs.
#[derive(Clone)]
pub(crate) struct Big {
    /// Limbs, least significant first.
    limbs: [u32; LIMBS],
    /// Number of limbs in use; higher limbs are zero.
    len: usize,
}

impl Default for Big {
    fn default() -> Self {
        Self {
            limbs: [0; LIMBS],
            len: 0,
        }
    }
}

impl Big {
    /// Returns the value `value`.
    pub(crate) fn from_small(value: u32) -> Self {
        let mut big = Self::default();
        big.add_small(value);
        big
    }

    /// Returns limb `index`, 0 beyond the limbs.
    fn limb(&self, index: usize) -> u32 {
        self.limbs.get(index).copied().unwrap_or(0)
    }

    /// Sets limb `index` to `value`; indices beyond the limbs are ignored.
    fn set_limb(&mut self, index: usize, value: u32) {
        if let Some(limb) = self.limbs.get_mut(index) {
            *limb = value;
        }
    }

    /// Returns the limbs in use.
    fn used(&self) -> &[u32] {
        self.limbs.get(..self.len).unwrap_or(&[])
    }

    /// Drops leading zero limbs from `len`.
    pub(crate) fn trim(&mut self) {
        while self.len > 0 && self.limb(self.len - 1) == 0 {
            self.len -= 1;
        }
    }

    /// Multiplies by `factor`.
    pub(crate) fn mul_small(&mut self, factor: u32) {
        let mut carry = 0_u64;
        for limb in self.limbs.iter_mut().take(self.len) {
            let product = u64::from(*limb) * u64::from(factor) + carry;
            *limb = product as u32;
            carry = product >> 32;
        }
        if carry != 0 && self.len < LIMBS {
            self.set_limb(self.len, carry as u32);
            self.len += 1;
        }
    }

    /// Adds `value`.
    pub(crate) fn add_small(&mut self, value: u32) {
        let mut carry = u64::from(value);
        let mut index = 0;
        while carry != 0 && index < LIMBS {
            let sum = u64::from(self.limb(index)) + carry;
            self.set_limb(index, sum as u32);
            carry = sum >> 32;
            index += 1;
        }
        self.len = self.len.max(index);
        self.trim();
    }

    /// Multiplies by `5^exponent`.
    pub(crate) fn mul_pow5(&mut self, mut exponent: u64) {
        // 5^13 is the largest power of five that fits a `u32`.
        while exponent >= 13 {
            self.mul_small(1_220_703_125);
            exponent -= 13;
        }
        for _ in 0..exponent {
            self.mul_small(5);
        }
    }

    /// Shifts left by `bits`.
    pub(crate) fn shl(&mut self, bits: u64) {
        let limbs = (bits / 32) as usize;
        let bits = (bits % 32) as u32;
        if self.len == 0 {
            return;
        }
        let len = (self.len + limbs + 1).min(LIMBS);
        for index in (0..len).rev() {
            let source = index.checked_sub(limbs);
            let high = source.map_or(0, |source| self.limb(source));
            let low = source
                .and_then(|source| source.checked_sub(1))
                .map_or(0, |source| self.limb(source));
            let value = if bits == 0 {
                high
            } else {
                (high << bits) | (low >> (32 - bits))
            };
            self.set_limb(index, value);
        }
        self.len = len;
        self.trim();
    }

    /// Shifts right by one bit.
    pub(crate) fn shr1(&mut self) {
        for index in 0..self.len {
            let value = (self.limb(index) >> 1) | (self.limb(index + 1) << 31);
            self.set_limb(index, value);
        }
        self.trim();
    }

    /// Returns the number of significant bits.
    pub(crate) fn bit_len(&self) -> u32 {
        match self.len {
            0 => 0,
            len => (len as u32) * 32 - self.limb(len - 1).leading_zeros(),
        }
    }

    /// Compares with `other`.
    pub(crate) fn compare(&self, other: &Self) -> Ordering {
        self.len
            .cmp(&other.len)
            .then_with(|| self.used().iter().rev().cmp(other.used().iter().rev()))
    }

    /// Subtracts `other`, which must not be larger.
    pub(crate) fn sub(&mut self, other: &Self) {
        let mut borrow = 0_i64;
        for index in 0..self.len {
            let right = if index < other.len {
                other.limb(index)
            } else {
                0
            };
            let difference = i64::from(self.limb(index)) - i64::from(right) - borrow;
            self.set_limb(index, difference.rem_euclid(1 << 32) as u32);
            borrow = i64::from(difference < 0);
        }
        self.trim();
    }

    /// Returns whether the value is zero.
    pub(crate) fn is_zero(&self) -> bool {
        self.len == 0
    }

    /// Returns the value `value`.
    pub(crate) fn from_u64(value: u64) -> Self {
        let mut big = Self::from_small((value >> 32) as u32);
        big.shl(32);
        big.add_small(value as u32);
        big
    }

    /// Adds `other`.
    pub(crate) fn add(&mut self, other: &Self) {
        let len = self.len.max(other.len);
        let mut carry = 0_u64;
        for index in 0..len {
            let right = if index < other.len {
                other.limb(index)
            } else {
                0
            };
            let sum = u64::from(self.limb(index)) + u64::from(right) + carry;
            self.set_limb(index, sum as u32);
            carry = sum >> 32;
        }
        self.len = len;
        if carry != 0 && self.len < LIMBS {
            self.set_limb(self.len, carry as u32);
            self.len += 1;
        }
    }

    /// Multiplies by `10^exponent`.
    pub(crate) fn mul_pow10(&mut self, exponent: u64) {
        self.mul_pow5(exponent);
        self.shl(exponent);
    }
}
