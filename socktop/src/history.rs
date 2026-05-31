//! Small utilities to manage bounded history buffers for charts.

use std::collections::VecDeque;

/// Push a value into a capped deque. Returns the evicted front element if any.
/// Callers maintaining a running sum can use this to update the sum without
/// re-iterating the whole deque.
pub fn push_capped<T>(dq: &mut VecDeque<T>, v: T, cap: usize) -> Option<T> {
    let evicted = if dq.len() == cap {
        dq.pop_front()
    } else {
        None
    };
    dq.push_back(v);
    evicted
}

// Keeps a history deque per core with a fixed capacity.
// Storage is u64 so sparkline rendering can hand the slice directly to
// ratatui's `Sparkline::data` (which takes `&[u64]`) without per-frame
// allocation or widening conversion.
pub struct PerCoreHistory {
    pub deques: Vec<VecDeque<u64>>,
    cap: usize,
}

impl PerCoreHistory {
    pub fn new(cap: usize) -> Self {
        Self {
            deques: Vec::new(),
            cap,
        }
    }

    // Ensure we have one deque per core; resize on CPU topology changes
    pub fn ensure_cores(&mut self, n: usize) {
        if self.deques.len() == n {
            return;
        }
        self.deques = (0..n).map(|_| VecDeque::with_capacity(self.cap)).collect();
    }

    // Push a new sample set for all cores (values 0..=100)
    pub fn push_samples(&mut self, samples: &[f32]) {
        self.ensure_cores(samples.len());
        for (i, v) in samples.iter().enumerate() {
            let val = v.clamp(0.0, 100.0).round() as u64;
            push_capped(&mut self.deques[i], val, self.cap);
        }
    }
}
