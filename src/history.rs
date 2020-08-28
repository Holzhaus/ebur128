// Copyright (c) 2011 Jan Kokemüller
// Copyright (c) 2020 Sebastian Dröge <sebastian@centricular.com>
//
// Permission is hereby granted, free of charge, to any person obtaining a copy
// of this software and associated documentation files (the "Software"), to deal
// in the Software without restriction, including without limitation the rights
// to use, copy, modify, merge, publish, distribute, sublicense, and/or sell
// copies of the Software, and to permit persons to whom the Software is
// furnished to do so, subject to the following conditions:
//
// The above copyright notice and this permission notice shall be included in
// all copies or substantial portions of the Software.
//
// THE SOFTWARE IS PROVIDED "AS IS", WITHOUT WARRANTY OF ANY KIND, EXPRESS OR
// IMPLIED, INCLUDING BUT NOT LIMITED TO THE WARRANTIES OF MERCHANTABILITY,
// FITNESS FOR A PARTICULAR PURPOSE AND NONINFRINGEMENT. IN NO EVENT SHALL THE
// AUTHORS OR COPYRIGHT HOLDERS BE LIABLE FOR ANY CLAIM, DAMAGES OR OTHER
// LIABILITY, WHETHER IN AN ACTION OF CONTRACT, TORT OR OTHERWISE, ARISING FROM,
// OUT OF OR IN CONNECTION WITH THE SOFTWARE OR THE USE OR OTHER DEALINGS IN
// THE SOFTWARE.

use crate::energy_to_loudness;

use std::collections::VecDeque;
use std::fmt;

// Not using lazy_static or similar here as that slows down every access considerably.
// TODO: Make this const once f64::powf is a const function
static mut HISTOGRAM_ENERGIES: [f64; 1000] = [0.0; 1000];
static mut HISTOGRAM_ENERGY_BOUNDARIES: [f64; 1001] = [0.0; 1001];

fn init_histogram() {
    use std::sync::Once;
    static ONCE: Once = Once::new();

    // Safety: This is called once on the first History::new() call and
    // afterwards the two arrays are only ever used immutably.
    ONCE.call_once(|| unsafe {
        for (i, o) in HISTOGRAM_ENERGIES.iter_mut().enumerate() {
            *o = f64::powf(10.0, (i as f64 / 10.0 - 69.95 + 0.691) / 10.0);
        }

        for (i, o) in HISTOGRAM_ENERGY_BOUNDARIES.iter_mut().enumerate() {
            *o = f64::powf(10.0, (i as f64 / 10.0 - 70.0 + 0.691) / 10.0);
        }
    });
}

fn histogram_energy_boundaries() -> &'static [f64; 1001] {
    // Safety: See init_histogram().
    unsafe { &HISTOGRAM_ENERGY_BOUNDARIES }
}

fn histogram_energies() -> &'static [f64; 1000] {
    // Safety: See init_histogram().
    unsafe { &HISTOGRAM_ENERGIES }
}

fn find_histogram_index(energy: f64) -> usize {
    let mut min = 0;
    let mut max = 1000;

    // Binary search
    loop {
        let mid = (min + max) / 2;
        if energy >= histogram_energy_boundaries()[mid] {
            min = mid;
        } else {
            max = mid;
        }

        if max - min == 1 {
            break;
        }
    }

    min
}

pub struct Histogram(Box<[u64; 1000]>);

impl Histogram {
    fn new() -> Self {
        Histogram(Box::new([0; 1000]))
    }

    fn add(&mut self, energy: f64) {
        let idx = find_histogram_index(energy);
        self.0[idx] += 1;
    }

    fn calc_relative_threshold(&self) -> (u64, f64) {
        let mut above_thresh_counter = 0;
        let mut relative_threshold = 0.0;

        for (count, energy) in self.0.iter().zip(histogram_energies().iter()) {
            relative_threshold += *count as f64 * *energy;
            above_thresh_counter += *count;
        }

        (above_thresh_counter, relative_threshold)
    }

    fn loudness_range(h: &[u64; 1000]) -> f64 {
        let mut size = 0;
        let mut power = 0.0;

        for (count, energy) in h.iter().zip(histogram_energies().iter()) {
            size += *count;
            power += *count as f64 * *energy;
        }

        if size == 0 {
            return 0.0;
        }

        power /= size as f64;
        let minus_twenty_decibels = f64::powf(10.0, -20.0 / 10.0);
        let integrated = minus_twenty_decibels * power;

        let index = if integrated < histogram_energy_boundaries()[0] {
            0
        } else {
            let index = find_histogram_index(integrated);
            if integrated > histogram_energies()[index] {
                index + 1
            } else {
                index
            }
        };
        let size = h[index..].iter().sum::<u64>();
        if size == 0 {
            return 0.0;
        }

        let percentile_low = ((size - 1) as f64 * 0.1 + 0.5) as u64;
        let percentile_high = ((size - 1) as f64 * 0.95 + 0.5) as u64;

        // TODO: Use an iterator here, maybe something around Iterator::scan()
        let mut j = index;
        let mut size = 0;
        while size <= percentile_low {
            size += h[j];
            j += 1;
        }
        let l_en = histogram_energies()[j - 1];

        while size <= percentile_high {
            size += h[j];
            j += 1;
        }
        let h_en = histogram_energies()[j - 1];

        energy_to_loudness(h_en) - energy_to_loudness(l_en)
    }
}

// TODO: Would ideally use a linked-list based queue of fixed-size queues
// to not require a huge contiguous allocation
pub struct Queue {
    queue: VecDeque<f64>,
    max: usize,
}

impl Queue {
    fn new(max: usize) -> Self {
        Queue {
            queue: VecDeque::with_capacity(std::cmp::min(max, 5000)),
            max,
        }
    }

    fn add(&mut self, energy: f64) {
        // Remove last element to keep the size
        if self.max == self.queue.len() {
            self.queue.pop_front();
        }
        self.queue.push_back(energy);
    }

    fn set_max_size(&mut self, max: usize) {
        if self.queue.len() < max {
            // FIXME: Use shrink() once stabilized
            self.queue.resize(max, 0.0);
            self.queue.shrink_to_fit();
        }
        self.max = max;
    }

    fn calc_relative_threshold(&self) -> (u64, f64) {
        (self.queue.len() as u64, self.queue.iter().sum::<f64>())
    }

    fn loudness_range(q: &[f64]) -> f64 {
        if q.is_empty() {
            return 0.0;
        }

        let power = q.iter().sum::<f64>() / q.len() as f64;
        let minus_twenty_decibels = f64::powf(10.0, -20.0 / 10.0);
        let integrated = minus_twenty_decibels * power;

        // TODO: Use iterators here or otherwise get rid of bounds checks
        let mut relgated = 0;
        let mut relgated_size = q.len();
        while relgated_size > 0 && q[relgated] < integrated {
            relgated += 1;
            relgated_size -= 1;
        }

        if relgated_size > 0 {
            let h_en = q[relgated + ((relgated_size - 1) as f64 * 0.95 + 0.5) as usize];
            let l_en = q[relgated + ((relgated_size - 1) as f64 * 0.1 + 0.5) as usize];

            energy_to_loudness(h_en) - energy_to_loudness(l_en)
        } else {
            0.0
        }
    }
}

pub enum History {
    Queue(Queue),
    Histogram(Histogram),
}

impl fmt::Debug for History {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            History::Histogram(..) => f.debug_struct("History::Histogram").finish(),
            History::Queue(..) => f.debug_struct("History::Queue").finish(),
        }
    }
}

impl History {
    pub fn new(use_histogram: bool, max: usize) -> Self {
        init_histogram();

        if use_histogram {
            History::Histogram(Histogram::new())
        } else {
            History::Queue(Queue::new(max))
        }
    }

    pub fn add(&mut self, energy: f64) {
        if energy < histogram_energy_boundaries()[0] {
            return;
        }

        match self {
            History::Histogram(ref mut h) => h.add(energy),
            History::Queue(ref mut q) => q.add(energy),
        }
    }

    pub fn set_max_size(&mut self, max: usize) {
        match self {
            History::Histogram(_) => (),
            History::Queue(ref mut q) => q.set_max_size(max),
        }
    }

    fn calc_relative_threshold(&self) -> (u64, f64) {
        match self {
            History::Histogram(ref h) => h.calc_relative_threshold(),
            History::Queue(ref q) => q.calc_relative_threshold(),
        }
    }

    pub fn gated_loudness(&self) -> f64 {
        Self::gated_loudness_multiple(&[self])
    }

    pub fn gated_loudness_multiple(s: &[&Self]) -> f64 {
        let (above_thresh_counter, relative_threshold) = s.iter().fold((0, 0.0), |mut acc, h| {
            let (above_thresh_counter, relative_threshold) = h.calc_relative_threshold();
            acc.0 += above_thresh_counter;
            acc.1 += relative_threshold;

            acc
        });

        if above_thresh_counter == 0 {
            return -f64::INFINITY;
        }

        let relative_gate = -10.0;
        let relative_gate_factor = f64::powf(10.0, relative_gate / 10.0);
        let relative_threshold =
            (relative_threshold / above_thresh_counter as f64) * relative_gate_factor;

        let mut above_thresh_counter = 0;
        let mut gated_loudness = 0.0;

        let start_index = if relative_threshold < histogram_energy_boundaries()[0] {
            0
        } else {
            let start_index = find_histogram_index(relative_threshold);
            if relative_threshold > histogram_energies()[start_index] {
                start_index + 1
            } else {
                start_index
            }
        };

        for h in s {
            match h {
                History::Histogram(ref h) => {
                    for (count, energy) in h.0[start_index..]
                        .iter()
                        .zip(histogram_energies()[start_index..].iter())
                    {
                        gated_loudness += *count as f64 * *energy;
                        above_thresh_counter += *count;
                    }
                }
                History::Queue(ref q) => {
                    for v in q.queue.iter() {
                        if *v >= relative_threshold {
                            above_thresh_counter += 1;
                            gated_loudness += *v;
                        }
                    }
                }
            }
        }

        if above_thresh_counter == 0 {
            return -f64::INFINITY;
        }

        energy_to_loudness(gated_loudness / above_thresh_counter as f64)
    }

    pub fn relative_threshold(&self) -> f64 {
        let (above_thresh_counter, relative_threshold) = self.calc_relative_threshold();

        if above_thresh_counter == 0 {
            return -70.0;
        }

        let relative_gate = -10.0;
        let relative_gate_factor = f64::powf(10.0, relative_gate / 10.0);
        let relative_threshold =
            (relative_threshold / above_thresh_counter as f64) * relative_gate_factor;

        energy_to_loudness(relative_threshold)
    }

    pub fn loudness_range(&self) -> f64 {
        // This can only fail if multiple histories are passed
        // and have a mix of histograms and queues
        Self::loudness_range_multiple(&[self]).unwrap()
    }

    pub fn loudness_range_multiple(s: &[&Self]) -> Result<f64, ()> {
        if s.is_empty() {
            return Ok(0.0);
        }

        match s[0] {
            History::Histogram(ref h) => {
                let mut combined;

                let combined = if s.len() == 1 {
                    &*h.0
                } else {
                    combined = [0; 1000];

                    for h in s {
                        match h {
                            History::Histogram(ref h) => {
                                for (i, o) in h.0.iter().zip(combined.iter_mut()) {
                                    *o += *i;
                                }
                            }
                            _ => return Err(()),
                        }
                    }

                    &combined
                };

                Ok(Histogram::loudness_range(combined))
            }
            History::Queue(_) => {
                let mut len = 0;
                for h in s {
                    match h {
                        History::Queue(ref q) => {
                            len += q.queue.len();
                        }
                        _ => return Err(()),
                    }
                }

                let mut combined = Vec::with_capacity(len);
                for h in s {
                    match h {
                        History::Queue(ref q) => {
                            let (v1, v2) = q.queue.as_slices();
                            combined.extend_from_slice(v1);
                            combined.extend_from_slice(v2);
                        }
                        _ => return Err(()),
                    }
                }

                combined.sort_unstable_by(|a, b| a.partial_cmp(b).unwrap());

                Ok(Queue::loudness_range(&*combined))
            }
        }
    }
}

#[cfg(feature = "c-tests")]
use std::os::raw::c_void;

#[cfg(feature = "c-tests")]
extern "C" {
    pub fn history_init_c();

    pub fn history_create_c(use_histogram: i32, max: usize) -> *mut c_void;

    pub fn history_add_c(history: *mut c_void, energy: f64);

    pub fn history_set_max_size_c(history: *mut c_void, max: usize);

    pub fn history_gated_loudness_c(history: *const c_void) -> f64;

    pub fn history_relative_threshold_c(history: *const c_void) -> f64;

    pub fn history_loudness_range_c(history: *const c_void) -> f64;

    pub fn history_destroy_c(history: *mut c_void);
}

#[cfg(feature = "c-tests")]
#[cfg(test)]
mod tests {
    use super::*;
    use float_eq::float_eq;
    use quickcheck_macros::quickcheck;
    use std::num::NonZeroU16;

    #[derive(Clone, Copy, Debug)]
    struct Energy(f64);

    impl quickcheck::Arbitrary for Energy {
        fn arbitrary<G: quickcheck::Gen>(g: &mut G) -> Self {
            use rand::Rng;

            Energy(g.gen_range(-5.0, 1200.0))
        }
    }

    fn init() {
        use std::sync::Once;

        static START: Once = Once::new();

        START.call_once(|| unsafe { history_init_c() });
    }

    #[quickcheck]
    fn compare_c_impl_gated_loudness(
        energy: Vec<Energy>,
        use_histogram: bool,
        max: NonZeroU16,
    ) -> Result<(), String> {
        init();

        let mut hist = History::new(use_histogram, max.get() as usize);
        for e in &energy {
            hist.add(e.0);
        }

        let val = hist.gated_loudness();

        let val_c = unsafe {
            let hist_c = history_create_c(if use_histogram { 1 } else { 0 }, max.get() as usize);
            for e in &energy {
                history_add_c(hist_c, e.0);
            }

            let val = history_gated_loudness_c(hist_c);
            history_destroy_c(hist_c);
            val
        };

        if !float_eq!(val, val_c, ulps <= 2) {
            Err(format!("{} != {}", val, val_c))
        } else {
            Ok(())
        }
    }

    #[quickcheck]
    fn compare_c_impl_relative_threshold(
        energy: Vec<Energy>,
        use_histogram: bool,
        max: NonZeroU16,
    ) -> Result<(), String> {
        init();

        let mut hist = History::new(use_histogram, max.get() as usize);
        for e in &energy {
            hist.add(e.0);
        }

        let val = hist.relative_threshold();

        let val_c = unsafe {
            let hist_c = history_create_c(if use_histogram { 1 } else { 0 }, max.get() as usize);
            for e in &energy {
                history_add_c(hist_c, e.0);
            }

            let val = history_relative_threshold_c(hist_c);
            history_destroy_c(hist_c);
            val
        };

        if !float_eq!(val, val_c, ulps <= 2) {
            Err(format!("{} != {}", val, val_c))
        } else {
            Ok(())
        }
    }

    #[quickcheck]
    fn compare_c_impl_loudness_range(
        energy: Vec<Energy>,
        use_histogram: bool,
        max: NonZeroU16,
    ) -> Result<(), String> {
        init();

        let mut hist = History::new(use_histogram, max.get() as usize);
        for e in &energy {
            hist.add(e.0);
        }

        let val = hist.loudness_range();

        let val_c = unsafe {
            let hist_c = history_create_c(if use_histogram { 1 } else { 0 }, max.get() as usize);
            for e in &energy {
                history_add_c(hist_c, e.0);
            }

            let val = history_loudness_range_c(hist_c);
            history_destroy_c(hist_c);
            val
        };

        if !float_eq!(val, val_c, ulps <= 2) {
            Err(format!("{} != {}", val, val_c))
        } else {
            Ok(())
        }
    }
}
