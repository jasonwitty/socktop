//! Small utilities to manage bounded history buffers for charts.

use std::collections::VecDeque;

pub fn push_capped<T>(dq: &mut VecDeque<T>, v: T, cap: usize) {
    if dq.len() == cap {
        dq.pop_front();
    }
    dq.push_back(v);
}

// Keeps a history deque per core with a fixed capacity
pub struct PerCoreHistory {
    pub deques: Vec<VecDeque<u16>>,
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
            let val = v.clamp(0.0, 100.0).round() as u16;
            push_capped(&mut self.deques[i], val, self.cap);
        }
    }
}
