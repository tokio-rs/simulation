use rand::{distributions::uniform::SampleUniform, rngs, Rng};

use rand_distr::{Distribution, Normal};
use std::{ops, sync};

#[derive(Debug)]
/// DeterministicRandom provides a deterministic RNG.
struct Inner {
    rng: rngs::SmallRng,
}

impl Inner {
    fn new_with_seed(seed: u64) -> Self {
        let rng = rand::SeedableRng::seed_from_u64(seed);
        Self { rng }
    }
}

#[derive(Debug)]
pub(crate) struct DeterministicRandom {
    inner: sync::Arc<sync::Mutex<Inner>>,
}

impl DeterministicRandom {
    pub(crate) fn new() -> Self {
        DeterministicRandom::new_with_seed(0)
    }
    pub(crate) fn new_with_seed(seed: u64) -> Self {
        let inner = Inner::new_with_seed(seed);
        let inner = sync::Arc::new(sync::Mutex::new(inner));
        Self { inner }
    }
    pub fn handle(&self) -> DeterministicRandomHandle {
        let inner = sync::Arc::clone(&self.inner);
        DeterministicRandomHandle { inner }
    }
}

#[derive(Debug, Clone)]
pub struct DeterministicRandomHandle {
    inner: sync::Arc<sync::Mutex<Inner>>,
}

impl DeterministicRandomHandle {
    pub fn normal_dist(&self, mean: f64, dev: f64) -> f64 {
        let normal = Normal::new(mean, dev).expect(&format!(
            "illegal normal params, mean: {}, deviation: {}",
            mean, dev
        ));
        let mut lock = self.inner.lock().unwrap();
        normal.sample(&mut lock.rng)
    }

    pub fn should_fault(&self, probability: f64) -> bool {
        let mut lock = self.inner.lock().unwrap();
        lock.rng.gen_bool(probability)
    }

    pub fn gen_range<T>(&self, range: ops::Range<T>) -> T
    where
        T: SampleUniform,
    {
        let mut lock = self.inner.lock().unwrap();
        lock.rng.gen_range(range.start, range.end)
    }
}
