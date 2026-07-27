#![doc = include_str!("../README.md")]

extern crate nalgebra as na;

pub mod bezier;
pub mod color;
pub mod distance;
pub mod generate;
pub mod render;
pub mod shape;
pub mod transform;
#[cfg(feature = "visualize")]
pub mod visualize;

pub mod correct_error;
mod interpolate;
