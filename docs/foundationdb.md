# Interesting bits from FoundationDB

Collection of notes gleaned from FoundationDB.

### Organization

Applications written using the FoundationDB simulator have heirarchy.

DataCenter -> Machine -> Process -> Interface

Each of these can be killed, which effectively halts processing. Processes are sometimes
not killed because it would immediately render the simulation useless. For that, a specific
`canKillProcesses` is provided to check.

### Actors

FoundationDB seems to manage all state using actors, which is particularly interesting since these
actors are also deterministic.