# Haysel (hayselnut's datalogging server)

This is a server that provides datalogging and identification for hayselnut weather stations,
allowing them to log their data over the network. it uses (tsdb)[src/tsdb/] for datalogging, 
and supports logging data from multiple stations, keeping their identities seperate.

## Communication

An API, using unix sockets (and possibly TCP?) is provided for accessing
the data in database for other local programs.

To talk with the weather stations, a custom protocol is used over UDP

