use std::io::{Read, Write};

/// Wrapper around internal buffer which handles wrapping read/writes
pub(crate) struct BufferHelper<'a>(pub(crate) &'a mut [u8], pub(crate) &'a mut [u8]);

impl<'a> BufferHelper<'a> {
    /// Size of the accessable buffer
    pub(crate) fn len(&self) -> usize {
        self.0.len() + self.1.len()
    }
}

impl<'a> Read for BufferHelper<'a> {
    #[inline]
    fn read(&mut self, buf: &mut [u8]) -> std::io::Result<usize> {
        // read from first buffer
        let bytes = self.0.as_ref().read(buf)?;

        // check fast-path
        if bytes < self.0.len() {
            // nothing more to read
            Ok(bytes)
        } else {
            // read more on second buffer
            match self.1.as_ref().read(&mut buf[bytes..]) {
                Ok(additional_bytes) => Ok(additional_bytes + bytes),
                Err(err) => Err(err),
            }
        }
    }
}

impl<'a> Write for BufferHelper<'a> {
    #[inline]
    fn write(&mut self, buf: &[u8]) -> std::io::Result<usize> {
        // write to the first buffer
        let bytes = self.0.as_mut().write(buf)?;

        // return if we wrote all of the buffer
        if bytes == buf.len() {
            Ok(bytes)
        } else {
            // write to the second buffer
            match self.1.as_mut().write(&buf[bytes..]) {
                Ok(additional_bytes) => Ok(bytes + additional_bytes),
                Err(err) => Err(err),
            }
        }
    }

    #[inline]
    fn flush(&mut self) -> std::io::Result<()> {
        Ok(())
    }
}
