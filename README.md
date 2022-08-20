# Dual Access Ringbuffer
Generic sized ringbuffer which can be written to via `WriteAccess` the same time as it gets consumed.
This helps to have a producer and a consumer accessing the same buffer at the same time.
Both accesses ensure that they do not overwrite data of the other half.

