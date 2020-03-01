//! State for a simulation run.
mod ident;
mod machine;
mod simulation;
mod task;
use ident::{LogicalMachineId, LogicalTaskId};
use machine::LogicalMachine;
pub use simulation::{Simulation, SimulationHandle};
use task::LogicalTaskHandle;

#[cfg(test)]
mod tests {
    use crate::state::{Simulation, SimulationHandle};
    use crate::ExecutorHandle;
    use std::time::Duration;

    // Ensure that simulated time passes at a faster rate than real time.
    #[test]
    fn simulate_time() {
        Simulation::new(0)
            .machine("localhost", |_| async {
                let start = std::time::Instant::now();
                let delay_duration = Duration::from_secs(30);
                let simulated_now = tokio::time::Instant::now();
                tokio::time::delay_for(delay_duration).await;
                assert!(std::time::Instant::now() - start < delay_duration);
                let expected = simulated_now.checked_add(delay_duration).unwrap();
                assert_eq!(expected, tokio::time::Instant::now());
            })
            .run();
    }

    // Test that spawning a child task will inherit the parent
    // tasks context for spawn and spawn_blocking.
    #[test]
    fn task_spawning_context_inheritance() {
        Simulation::new(0)
            .machine("client1", |_| async {
                assert_eq!(
                    String::from("client1"),
                    SimulationHandle::current().hostname(),
                    "expected hostname to match for root level task"
                );
            })
            .machine("client2", |_| async {
                let child_hostname = SimulationHandle::current()
                    .spawn(async { SimulationHandle::current().hostname() })
                    .await
                    .unwrap();
                assert_eq!(
                    String::from("client2"),
                    child_hostname,
                    "expected child task to inherit parent hostname"
                );
            })
            .machine("client3", |_| async {
                let child_hostname = SimulationHandle::current()
                    .spawn_blocking(|| SimulationHandle::current().hostname())
                    .await
                    .unwrap();
                assert_eq!(
                    String::from("client3"),
                    child_hostname,
                    "expected child task to inherit parent hostname"
                );
            })
            .run();
    }
}
