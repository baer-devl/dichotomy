pub(crate) struct Slice {
    index: usize,
    length: usize,
}

impl Slice {
    pub fn split_at(self, index: usize) -> (Slice, Slice) {
        (
            Slice {
                index: self.index,
                length: index,
            },
            Slice {
                index: self.index + index,
                length: self.length - index,
            },
        )
    }
}

pub(crate) struct Collection {
    ptr: *mut u8,
    slice0: Slice,
    slice1: Slice,
}

impl Collection {
    pub fn swap(&self) {
        unimplemented!()
    }

    pub fn take(&self) -> Slice {
        unimplemented!()
    }

    pub fn put(&self) {
        unimplemented!()
    }
}

/// Wrapper around internal buffer which handles wrapping read/writes
pub(crate) struct BufferHelper<'a, const N: usize> {
    pub buffer0: &'a mut [u8],
    pub buffer1: &'a mut [u8],
    len: usize,
}

impl<'a, const N: usize> BufferHelper<'a, N> {
    pub(crate) fn new(buffer0: &'a mut [u8], buffer1: &'a mut [u8]) -> Self {
        let len = buffer0.len() + buffer1.len();
        Self {
            buffer0,
            buffer1,
            len,
        }
    }

    /// Size of the accessable buffer
    #[inline]
    pub(crate) fn len(&self) -> usize {
        self.len
    }

    #[inline]
    pub(crate) fn read(&mut self, buf: &mut [u8]) -> usize {
        // read from first buffer
        let buf_len = buf.len();
        let bytes = std::cmp::min(buf_len, self.buffer0.len());

        if bytes > 0 {
            // actual copy
            buf[..bytes].copy_from_slice(&self.buffer0[..bytes]);

            // remove data from buffer
            self.buffer0 = unsafe {
                std::slice::from_raw_parts_mut(
                    self.buffer0[bytes..].as_mut_ptr(),
                    self.buffer0.len() - bytes,
                )
            };

            // check fast-path
            if buf_len == bytes {
                // update len
                self.len -= bytes;

                // nothing more to read
                return bytes;
            }
        }

        // read more on second buffer
        let bytes1 = std::cmp::min(buf_len - bytes, self.buffer1.len());

        if bytes1 > 0 {
            // actual copy
            buf[bytes..bytes + bytes1].copy_from_slice(&self.buffer1[..bytes1]);

            // remove data from buffer
            self.buffer1 = unsafe {
                std::slice::from_raw_parts_mut(
                    self.buffer1[bytes1..].as_mut_ptr(),
                    self.buffer1.len() - bytes1,
                )
            };

            // update len
            self.len -= bytes + bytes1;

            bytes + bytes1
        } else {
            // update len
            self.len -= bytes;

            bytes
        }
    }

    #[inline]
    pub(crate) fn write(&mut self, buf: &[u8]) -> usize {
        // write to the first buffer
        let buf_len = buf.len();
        let bytes = std::cmp::min(buf_len, self.buffer0.len());

        if bytes > 0 {
            // actual copy
            self.buffer0[..bytes].copy_from_slice(&buf[..bytes]);

            // remove data from buffer
            self.buffer0 = unsafe {
                std::slice::from_raw_parts_mut(
                    self.buffer0[bytes..].as_mut_ptr(),
                    self.buffer0.len() - bytes,
                )
            };

            // return if we wrote all of the buffer
            if bytes == buf_len {
                // update len
                self.len -= bytes;

                return bytes;
            }
        }

        // write to the second buffer
        let bytes1 = std::cmp::min(buf_len - bytes, self.buffer1.len());

        if bytes1 > 0 {
            // actual copy
            self.buffer1[..bytes1].copy_from_slice(&buf[bytes..bytes + bytes1]);

            // remove data from buffer
            self.buffer1 = unsafe {
                std::slice::from_raw_parts_mut(
                    self.buffer1[bytes1..].as_mut_ptr(),
                    self.buffer1.len() - bytes1,
                )
            };

            // update len
            self.len -= bytes + bytes1;

            bytes + bytes1
        } else {
            // update len
            self.len -= bytes;

            bytes
        }
    }
}
