# Note
The Simulation library is being refactored to integrate more directly with Tokio. Currently, Simulation is not compatible with Tokio 0.2.x. As a result, it's recommended that users wait for a future release of Simulation. The issue tracking Tokio integration progress can be found here https://github.com/tokio-rs/tokio/issues/1845.

# simulation

The goal of Simulation is to provide a set of low level components which can be
used to write applications amenable to [FoundationDB style simulation testing](https://apple.github.io/foundationdb/testing.html).

Simulation is an abstraction over [Tokio], allowing application developers to write
applications which are generic over sources of nondeterminism. Additionally, Simulation
provides deterministic analogues to time, scheduling, network and eventually disk IO.
