//! Simulation is a wrapper around Tokio which supports building
//! applications amenable to FoundationDB style simulation testing.
#![allow(unused_variables, dead_code)]

mod api;
mod executor;
pub use api::ExecutorHandle;
mod state;
pub(crate) mod tcp;
mod util;

/*
pub struct Simulation {
    seed: u64,
    next_task_id: u64,
    taskid_to_hostname: HashMap<LogicalTaskId, String>,
    machines: HashMap<LogicalTaskId, Pin<Box<dyn Future<Output = ()> + Send>>>,
}

impl Simulation {
    pub fn new(seed: u64) -> Self {
        Simulation {
            seed,
            next_task_id: 0,
            taskid_to_hostname: HashMap::new(),
            machines: HashMap::new(),
        }
    }

    fn new_task_identifier(&mut self) -> LogicalTaskId {
        let next = self.next_task_id;
        self.next_task_id += 1;
        LogicalTaskId::new(next)
    }

    pub fn add_machine<F, S>(&mut self, hostname: S, f: F) -> &mut Self
    where
        F: Future<Output = ()> + Send + 'static,
        S: Into<String> + Clone,
    {
        let hostname = hostname.into();
        let boxed = Box::pin(f);
        let task_identifier = self.new_task_identifier();
        let wrapped = task::context::Wrapped::new(task_identifier, boxed);
        self.machines.insert(task_identifier, Box::pin(wrapped));
        self.taskid_to_hostname
            .insert(task_identifier, hostname.clone());
        self
    }

    fn create_runtime(&self) -> tokio::runtime::Runtime {
        tokio::runtime::Builder::new()
            .basic_scheduler()
            .enable_time()
            .freeze_time()
            .build()
            .expect("failed to construct runtime")
    }

    pub fn run(self) {
        let mut rt = self.create_runtime();
        let mut spawned = vec![];
        for (ident, fut) in self.machines.into_iter() {
            let fut = task::context::Wrapped::new(ident, fut);
            let handle = rt.spawn(fut);
            spawned.push(handle);
        }
        rt.block_on(async {
            for handle in spawned {
                handle.await.unwrap();
            }
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    async fn server(port: u16) {
        tokio::time::delay_for(std::time::Duration::from_secs(100)).await;
        let now = tokio::time::Instant::now();
        println!(
            "{:?} - time: {:?}",
            crate::task::TaskIdentifier::current(),
            now
        );
    }

    async fn client<S: Into<String>>(hostport: S) {
        let now = tokio::time::Instant::now();
        println!(
            "{:?} - time: {:?}",
            crate::task::TaskIdentifier::current(),
            now
        );
    }

    #[test]
    fn ideal() {
        let mut sim = Simulation::new(64);
        sim.add_machine("server", server(9092))
            .add_machine("client1", client("server:9092"))
            .add_machine("client2", client("server:9092"))
            .add_machine("client3", client("server:9092"));
        sim.run();
    }
}
*/
