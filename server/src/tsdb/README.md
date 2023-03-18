# Time-Series Database

## TODO
- (internal) : make pointer-using functions unsafe, or create some safe way of tracking them
- remove `mark_used` in `AllocReq` variants (it may never be set to false)
- use something other than a bump allocator
