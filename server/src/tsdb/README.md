# Time-Series Database

This is a completely from-scratch implementation of a time-seriese database completely in rust.

Feel free to use this in your own project, but you may regret it (see [this warning](#warning))

## Some info

Serialization is done through `zerocopy`, although this may change to `serde` in the future to allow variable length data.

Querying the database can only be done by time range, as in here is a start and end time, what happened between them.
- this is perfect for storing weather data, and querying over time, however
- this makes it difficult to impossible to query based on data 

## Warning

Currently this is very unstable (both in terms of spec and solidness). if the program crashes during a read, 
the DB will most likely become corrupt. This will improve over time, and when it is better this warning will be removed.

## TODO
- (internal) : make pointer-using functions unsafe, or create some safe way of tracking them
- remove `mark_used` in `AllocReq` variants (it may never be set to false)
- use something other than a bump allocator
