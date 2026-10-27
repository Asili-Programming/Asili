//! Fixed-width integers (`Biti8`…`uBiti64`): the value, held exactly in an `i128`, and the
//! width it lives in. Arithmetic wraps at the width (two's complement), as the spec says.

use asili_parser::{fixed_int_name, fixed_int_range};

/// The width of a fixed-width integer: `bits` wide, two's complement when `signed`.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct IntWidth {
    pub bits: u8,
    pub signed: bool,
}

impl IntWidth {
    /// The type name, `Biti32` or `uBiti8`.
    pub fn name(self) -> String {
        fixed_int_name(self.bits, self.signed)
    }

    /// The smallest and largest value of this width.
    pub fn range(self) -> (i128, i128) {
        fixed_int_range(self.bits, self.signed)
    }

    /// Whether `v` is in this width's range.
    pub fn fits(self, v: i128) -> bool {
        let (lo, hi) = self.range();
        lo <= v && v <= hi
    }

    /// `v` reduced to this width, wrapping (two's complement): `127 + 1` in `Biti8` is `-128`.
    pub fn wrap(self, v: i128) -> i128 {
        let modulus = 1i128 << self.bits;
        let r = v.rem_euclid(modulus);
        if self.signed && r >= modulus / 2 {
            r - modulus
        } else {
            r
        }
    }
}
