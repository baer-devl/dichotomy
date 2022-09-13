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

## Why do we need a state
You probably wondering why it is necessary to hold an index and a state. 
Indeed, there are different solutions worth taking a closer look at.

### Flag tail
A rather simple solution would be to use a bit to flag the tail index to ensure to the head that it is full.
This would result in the following states:

 1. `T` == `H`: empty buffer
 2. `T` != `H`: partially full buffer
 3. `t` == `H`: full buffer

Where `t` means that the full flag is set for the tail index while still holding the correct index in the remaining bits.
Now, if head reads the tail, it knows that it is full and although the indicies of tail and head are equal it does not mean that the buffer is empty.
The problem relies on how the tail can safely proceed.
Obviously, if the head index changes, it is safe to remove the full flag and proceed writing to the buffer.
The only catch here is, that if the head reads all the buffer until it is empty, the tail still sees the same values as in `3`.
Further, now the head sees that the buffer is full and proceed to read from the buffer although it had already consumed it.

### Flag tail and head
An intuitive solution for the previous problem would be to flag also the head.
Because the head knows that it can read until the flagged tail index, it could also flag itself to indicate to the tail that the buffer is now empty.
Those are the new possible states:

 1. `T` == `H`: empty buffer
 2. `T` != `H`: partially full buffer
 3. `t` == `H`: full buffer
 4. `t` == `h`: empty buffer

However, we only shifting the problem.
Tail does now know that the buffer is empty if the head flag is set, but if tail would write to the buffer until it is full again, the flag would be set again.
If the head did not read in between those writes it sees the same state as in `4` indicating that the buffer is empty.

### Spare entry
Another common solution is to spare a single entry.
In this case, the buffer is already full one entry less of its capacity.
This would work fine but with the downside of unusable buffer.
That might be not a huge problem if it is a byte but as we are generic, it also could be an object with more memory impact.
Besides the not usable memory, there are also additional downsides like:
 - Choosing the correct buffer size by the capacity or the `real` capacity
 - What happens on a single byte buffer

## Index and state
The solution from `dichotomy` use a simple state which is stored as an additional information on top of the index.
This makes it possible to distinguish between an empty and a full buffer with the following possible states:

 1. `T` == `H`: empty buffer
 2. `T` != `H`: partially full buffer
 3. `t` == `h` && `S` != `s`: full buffer

Where `T` and `H` are the so called tags we are using to store the index and the state in a single `usize` value.
If both indicies (`t` and `h`) are the same and the states (`S` and `s`) differ, the buffer is full.
The state `S` gets incremented each time the tail writes new data onto the buffer.
As the state has the same size as the maximal buffer capacity, it will remain unique even if we write each byte individual.
Head is holding `s` which is the last known state of tail.
This value gets updated if the head needs to update its cached values.
Additionally, this also helps both, the head and the tail to keep track if the buffer has changed and an update make sense.

As previous mentioned, head and tail do have cached values to operate to the buffer.
If the cache need a refresh they will load the corresponding data and update the cache.
Both, read and write will be performed in a single instruction which does not need to bother atomic operations.

There are two downsides to this solution though.
First, we can only use half of the bytes from a `usize` for the maximal capacity of the buffer.
On an x64 architecture this would be a `u32` value.
Second, it needs to perform additional operations to store and load those two values from the tags resulting in an overhead.
However, because we can get rid of any synchronization primitives like atomic operations, it will be still faster to use.
