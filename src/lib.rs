//!  Implementation of the [EBU R128 loudness standard](https://tech.ebu.ch/docs/r/r128.pdf).
//!
//!  The European Broadcasting Union Loudness Recommendation (EBU R128) informs broadcasters how
//!  they can analyze and normalize audio so that each piece of audio sounds roughly the same
//!  volume to the human ear.
//!
//!  This crate provides an API which analyzes audio and outputs perceived loudness. The results
//!  can then be used to normalize volume during playback.
//!
//!  Features:
//!   * Implements M, S and I modes
//!   * Implements loudness range measurement (EBU - TECH 3342)
//!   * True peak scanning
//!   * Supports all samplerates by recalculation of the filter coefficients

#[allow(unused, non_camel_case_types, non_upper_case_globals)]
mod ffi;

mod ebur128;
pub use self::ebur128::*;

#[cfg(feature = "internal-tests")]
pub mod interp;
#[cfg(not(feature = "internal-tests"))]
pub(crate) mod interp;

#[cfg(feature = "internal-tests")]
pub mod true_peak;
#[cfg(not(feature = "internal-tests"))]
pub(crate) mod true_peak;

#[cfg(feature = "internal-tests")]
pub mod history;
#[cfg(not(feature = "internal-tests"))]
pub(crate) mod history;

#[cfg(test)]
mod tests {
    #[derive(Clone, Debug)]
    pub struct Signal<T: FromF32> {
        pub data: Vec<T>,
        pub channels: u32,
        pub rate: u32,
    }

    pub trait FromF32: Copy + Clone + std::fmt::Debug + Send + Sync + 'static {
        fn from_f32(val: f32) -> Self;
    }

    impl FromF32 for i16 {
        fn from_f32(val: f32) -> Self {
            (val * (std::i16::MAX - 1) as f32) as i16
        }
    }

    impl FromF32 for i32 {
        fn from_f32(val: f32) -> Self {
            (val * (std::i32::MAX - 1) as f32) as i32
        }
    }

    impl FromF32 for f32 {
        fn from_f32(val: f32) -> Self {
            val
        }
    }

    impl FromF32 for f64 {
        fn from_f32(val: f32) -> Self {
            val as f64
        }
    }

    impl<T: FromF32> quickcheck::Arbitrary for Signal<T> {
        fn arbitrary<G: quickcheck::Gen>(g: &mut G) -> Self {
            use rand::Rng;

            let channels = g.gen_range(1, 16);
            let rate = g.gen_range(16_000, 224_000);
            let num_frames = (rate as f64 * g.gen_range(0.0, 5.0)) as usize;

            let max = g.gen_range(0.0, 1.0);
            let freqs = [
                g.gen_range(20.0, 16_000.0),
                g.gen_range(20.0, 16_000.0),
                g.gen_range(20.0, 16_000.0),
                g.gen_range(20.0, 16_000.0),
            ];
            let volumes = [
                g.gen_range(0.0, 1.0),
                g.gen_range(0.0, 1.0),
                g.gen_range(0.0, 1.0),
                g.gen_range(0.0, 1.0),
            ];
            let volume_scale = 1.0 / volumes.iter().sum::<f32>();
            let mut accumulators = [0.0; 4];
            let steps = [
                2.0 * std::f32::consts::PI * freqs[0] / rate as f32,
                2.0 * std::f32::consts::PI * freqs[1] / rate as f32,
                2.0 * std::f32::consts::PI * freqs[2] / rate as f32,
                2.0 * std::f32::consts::PI * freqs[3] / rate as f32,
            ];

            let mut data = vec![T::from_f32(0.0); num_frames * channels as usize];
            for frame in data.chunks_exact_mut(channels as usize) {
                let val = max
                    * (f32::sin(accumulators[0]) * volumes[0]
                        + f32::sin(accumulators[1]) * volumes[1]
                        + f32::sin(accumulators[2]) * volumes[2]
                        + f32::sin(accumulators[3]) * volumes[3])
                    / volume_scale;

                for sample in frame.iter_mut() {
                    *sample = T::from_f32(val);
                }

                for (acc, step) in accumulators.iter_mut().zip(steps.iter()) {
                    *acc += step;
                }
            }

            Signal {
                data,
                channels,
                rate,
            }
        }
    }
}
