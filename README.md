# Dichotomy
A lock-free generic ring-buffer without atomic operations for handling access for producer and consumer.

## Concept
A ring-buffer consists of a head and a tail which are used to figure out where the producer can write to and a consumer can read from.
There are three states a buffer can have:
 1. Empty: head and tail do have the same value, eg: (0,0)
 2. Partially Full: head and tail do have different values, eg: (10,0)
 3. Full: head and tail do have the same value, eg: (10,10), (0,0)

As you can see, the buffer can be either empty or full and without additional information it is not possible to distinguish.
However, to store additional information we would need to synchronize the read/write to it.
The solution to this problem is to use half of head and tail to store those additional information.
Because we are using an `usize`, the architecture on which the code runs will determine the max size of the buffer. 
This restriction cannot be overcome as we need the processor to be able to handle read and writes on a single instruction. 
Thus, `usize` will do the trick as the processor need to be able to handle pointer in a single instruction.
