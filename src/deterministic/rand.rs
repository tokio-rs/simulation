use rand::{rngs};
use std::sync;

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
struct DeterministicRandom {
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
