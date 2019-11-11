# Interesting bits from FoundationDB

Collection of notes gleaned from FoundationDB.

### Organization

Applications written using the FoundationDB simulator have heirarchy.

DataCenter -> Machine -> Process -> Interface

Each of these can be killed, which effectively halts processing. Processes are sometimes
not killed because it would immediately render the simulation useless. For that, a specific
`canKillProcesses` is provided to check.

### Fault Injection and Workloads

FoundationDB seperates out fault injection from workloads, but the lines are a bit blurry. Generally
fault injection seems to refer to probabilistically injecting faults at IO callsites. Workloads are more
general, and can range from killing random nodes to generating transactions.