use std::io::{Read, Write};

/// Wrapper around internal buffer which handles wrapping read/writes
pub(crate) struct BufferHelper(pub(crate) *mut [u8], pub(crate) *mut [u8]);

impl<'a> BufferHelper {
    /// Size of the accessable buffer
    pub(crate) fn len(&self) -> usize {
        unsafe { &*self.0 }.len() + unsafe { &*self.1 }.len()
    }
}

impl<'a> Read for BufferHelper {
    #[inline]
    fn read(&mut self, buf: &mut [u8]) -> std::io::Result<usize> {
        // read from first buffer
        let bytes = unsafe { &*self.0 }.read(buf)?;

        // remove data from buffer
        unsafe { self.0 = &mut (&mut *self.0)[bytes..] as *mut [u8] };

        // check fast-path
        if bytes < unsafe { &*self.0 }.len() {
            // nothing more to read
            Ok(bytes)
        } else {
            // read more on second buffer
            match unsafe { &*self.1 }.read(&mut buf[bytes..]) {
                Ok(additional_bytes) => {
                    // remove data from buffer
                    unsafe { self.1 = &mut (&mut *self.1)[additional_bytes..] as *mut [u8] };

                    Ok(additional_bytes + bytes)
                }
                Err(err) => Err(err),
            }
        }
    }
}

impl<'a> Write for BufferHelper {
    #[inline]
    fn write(&mut self, buf: &[u8]) -> std::io::Result<usize> {
        // write to the first buffer
        let bytes = unsafe { &mut *self.0 }.write(buf)?;

        // remove data from buffer
        unsafe { self.0 = &mut (&mut *self.0)[bytes..] as *mut [u8] };

        // return if we wrote all of the buffer
        if bytes == buf.len() {
            Ok(bytes)
        } else {
            // write to the second buffer
            match unsafe { &mut *self.1 }.write(&buf[bytes..]) {
                Ok(additional_bytes) => {
                    // remove data from buffer
                    unsafe { self.1 = &mut (&mut *self.1)[additional_bytes..] as *mut [u8] };

                    Ok(bytes + additional_bytes)
                }

                Err(err) => Err(err),
            }
        }
    }

    #[inline]
    fn flush(&mut self) -> std::io::Result<()> {
        Ok(())
    }
}
