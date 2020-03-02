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
    use futures::SinkExt;
    use std::time::Duration;
    use tokio::stream::StreamExt;
    use tokio_util::codec::{Framed, LinesCodec};

    // Ensure that simulated time passes at a faster rate than real time.
    #[test]
    fn simulate_time() {
        Simulation::new(0)
            .machine("localhost", async {
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
            .machine("client1", async {
                assert_eq!(
                    String::from("client1"),
                    SimulationHandle::current().hostname(),
                    "expected hostname to match for root level task"
                );
            })
            .machine("client2", async {
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
            .machine("client3", async {
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

    /// Test that client connections can be made and disconnects are witnessed.
    #[test]
    fn test_client_server_connect() {
        let mut simulation = Simulation::new(0);
        simulation
            .machine("server", async {
                let handle = crate::state::SimulationHandle::current();
                let mut listener = handle.bind(9092);
                while let Ok((socket, addr)) = listener.accept().await {
                    let mut transport = Framed::new(socket, LinesCodec::new());
                    match transport.next().await {
                        Some(Ok(msg)) => {
                            transport.send(format!("Hello, {}", msg)).await.unwrap();
                        }
                        Some(Err(e)) => panic!(e),
                        None => panic!("server listener closed"),
                    };
                }
            })
            .machine("client", async {
                let handle = crate::state::SimulationHandle::current();
                // Induce reordering so the server starts before this task.
                tokio::time::delay_for(Duration::from_secs(5)).await;
                let stream = handle
                    .connect(String::from("server"), 9092)
                    .await
                    .expect("failed to connect");
                let mut transport = Framed::new(stream, LinesCodec::new());
                transport.send(String::from("Simulation")).await.unwrap();
                let response = transport.next().await.unwrap().unwrap();
                assert_eq!("Hello, Simulation", response);
            })
            .run();
    }
}
