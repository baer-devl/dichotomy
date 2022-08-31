/// Wrapper around internal buffer which handles wrapping read/writes
pub(crate) struct BufferHelper<const N: usize> {
    pub buffer0: *mut u8,
    pub buffer1: *mut u8,
    len0: usize,
    len1: usize,
}

impl<const N: usize> BufferHelper<N> {
    pub(crate) fn new(buffer0: *mut u8, buffer1: *mut u8, len0: usize, len1: usize) -> Self {
        Self {
            buffer0,
            buffer1,
            len0,
            len1,
        }
    }

    /// Size of the accessable buffer
    #[inline]
    pub(crate) fn len(&self) -> usize {
        self.len0 + self.len1
    }

    #[inline]
    pub(crate) fn read(&mut self, buf: &mut [u8]) -> usize {
        // read from first buffer
        let buf_len = buf.len();
        let bytes = std::cmp::min(buf_len, self.len0);

        if bytes > 0 {
            // actual copy
            let src_ptr = self.buffer0;
            let dst_ptr = buf.as_mut_ptr();
            unsafe { dst_ptr.copy_from_nonoverlapping(src_ptr, bytes) };

            // remove data from buffer
            self.len0 -= bytes;
            self.buffer0 = unsafe { self.buffer0.add(bytes) };

            // check fast-path
            if buf_len == bytes {
                // nothing more to read
                return bytes;
            }
        }

        // read more on second buffer
        let bytes1 = std::cmp::min(buf_len - bytes, self.len1);

        if bytes1 > 0 {
            // actual copy
            let src_ptr = self.buffer1;
            let dst_ptr = unsafe { buf.as_mut_ptr().add(bytes) };
            unsafe { dst_ptr.copy_from_nonoverlapping(src_ptr, bytes1) };

            // remove data from buffer
            self.len1 -= bytes1;
            self.buffer1 = unsafe { self.buffer1.add(bytes1) };

            bytes + bytes1
        } else {
            bytes
        }
    }

    #[inline]
    pub(crate) fn write(&mut self, buf: &[u8]) -> usize {
        // write to the first buffer
        let buf_len = buf.len();
        let bytes = std::cmp::min(buf_len, self.len0);

        if bytes > 0 {
            // actual copy
            let src_ptr = buf.as_ptr();
            let dst_ptr = self.buffer0;
            unsafe { dst_ptr.copy_from_nonoverlapping(src_ptr, bytes) };

            // remove data from buffer
            self.len0 -= bytes;
            self.buffer0 = unsafe { self.buffer0.add(bytes) };

            // return if we wrote all of the buffer
            if bytes == buf_len {
                return bytes;
            }
        }

        // write to the second buffer
        let bytes1 = std::cmp::min(buf_len - bytes, self.len1);

        if bytes1 > 0 {
            // actual copy
            let src_ptr = unsafe { buf.as_ptr().add(bytes) };
            let dst_ptr = self.buffer1;
            unsafe { dst_ptr.copy_from_nonoverlapping(src_ptr, bytes1) };

            // remove data from buffer
            self.len1 -= bytes1;
            self.buffer1 = unsafe { self.buffer1.add(bytes1) };

            bytes + bytes1
        } else {
            bytes
        }
    }
}
