# Time-Series Database

## TODO
- (internal) : make pointer-using functions unsafe, or create some safe way of tracking them
- TimeSegments storing data in-order is causing lots of pain...
- remove start_time and end_time, just use the first and last elements of the list (and if it has a one before it)
