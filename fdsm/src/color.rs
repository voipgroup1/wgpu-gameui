//! Handling edge colors.
//!
//! As part of the MSDF generation process, each edge of a shape is
//! assigned a color.
//!
//! See Section 3.3 of (Chlumský, 2015) for more information.

use std::{
    fmt::Debug,
    ops::{BitAnd, BitAndAssign, BitOr, BitOrAssign, BitXor, BitXorAssign, Not},
};

/// The number of channels used.
pub const NUM_CHANNELS: usize = 3;

/// The color of an edge.
///
/// Each of the three channels can be on or off.
///
/// See Section 3.3 of (Chlumský, 2015) for more information.
#[derive(PartialEq, Eq, Copy, Clone)]
#[repr(transparent)]
pub struct Color(u8);

impl Color {
    /// Creates a new color from the underlying bits.
    ///
    /// Numbering the bits such that 0 is the least significant bit:
    ///
    /// * Bit 0 corresponds to the red channel.
    /// * Bit 1 corresponds to the green channel.
    /// * Bit 2 corresponds to the blue channel.
    ///
    /// Bits 3 and above are truncated in the resulting color.
    #[inline]
    pub fn new(value: u8) -> Self {
        Self(value & ((1 << NUM_CHANNELS) - 1))
    }

    /// Returns the underlying bits of this color.
    #[inline]
    pub fn value(&self) -> u8 {
        self.0
    }

    /// Returns true if the red channel is on for this color.
    #[inline]
    pub fn has_red(&self) -> bool {
        (self.0 & 1) != 0
    }

    /// Returns true if the green channel is on for this color.
    #[inline]
    pub fn has_green(&self) -> bool {
        (self.0 & 2) != 0
    }

    /// Returns true if the blue channel is on for this color.
    #[inline]
    pub fn has_blue(&self) -> bool {
        (self.0 & 4) != 0
    }

    /// Returns true if this color has at least two channels set.
    #[inline]
    pub fn is_bright(&self) -> bool {
        (self.0 & (self.0 - 1)) != 0
    }

    /// A helepr function for choosing the next color when performing edge coloring.
    ///
    /// See [`crate::shape::ColoredContour::edge_coloring_simple`] for more details.
    // See https://github.com/Chlumsky/msdfgen/blob/master/core/edge-coloring.cpp#L28
    pub fn switch(self, seed: u64, banned: Color) -> (Color, u64) {
        let combined = self & banned;
        if matches!(combined, Self::RED | Self::GREEN | Self::BLUE) {
            (!combined, seed)
        } else if matches!(self, Self::BLACK | Self::WHITE) {
            (
                [Self::CYAN, Self::MAGENTA, Self::YELLOW][(seed % 3) as usize],
                seed / 3,
            )
        } else {
            let shifted = self.0 << (1 + (seed & 1));
            (Self::new(shifted | (shifted >> 3)), seed >> 1)
        }
    }

    pub const BLACK: Color = Color(0);
    pub const WHITE: Color = Color(7);
    pub const YELLOW: Color = Color(3);
    pub const CYAN: Color = Color(6);
    pub const MAGENTA: Color = Color(5);
    pub const RED: Color = Color(1);
    pub const GREEN: Color = Color(2);
    pub const BLUE: Color = Color(4);
}

impl BitAnd for Color {
    type Output = Self;

    #[inline]
    fn bitand(self, rhs: Self) -> Self::Output {
        Self(self.0 & rhs.0)
    }
}

impl BitAndAssign for Color {
    #[inline]
    fn bitand_assign(&mut self, rhs: Self) {
        self.0 &= rhs.0
    }
}

impl BitOr for Color {
    type Output = Self;

    #[inline]
    fn bitor(self, rhs: Self) -> Self::Output {
        Self(self.0 | rhs.0)
    }
}

impl BitOrAssign for Color {
    #[inline]
    fn bitor_assign(&mut self, rhs: Self) {
        self.0 |= rhs.0
    }
}

impl BitXor for Color {
    type Output = Self;

    #[inline]
    fn bitxor(self, rhs: Self) -> Self::Output {
        Self(self.0 ^ rhs.0)
    }
}

impl BitXorAssign for Color {
    #[inline]
    fn bitxor_assign(&mut self, rhs: Self) {
        self.0 ^= rhs.0;
    }
}

impl Not for Color {
    type Output = Self;

    #[inline]
    fn not(self) -> Self::Output {
        Self(self.0 ^ 7)
    }
}

impl Debug for Color {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(
            f,
            "({}, {}, {})",
            self.0 & 1,
            (self.0 >> 1) & 1,
            (self.0 >> 2) & 1
        )
    }
}
