use std::marker::PhantomData;

// Internal helper type
pub(crate) type InvariantLifetime<'token> = PhantomData<fn(&'token ()) -> &'token ()>;

/// Used to create a token to fill a buffer
pub struct ProducerToken<'producer> {
    _marker: InvariantLifetime<'producer>,
}

unsafe impl<'producer> Send for ProducerToken<'producer> {}
unsafe impl<'producer> Sync for ProducerToken<'producer> {}

impl<'producer> ProducerToken<'producer> {
    /// Create a new producer token to push new data on buffer
    ///
    /// # Example
    /// ```
    /// use dual_access_ringbuffer::{RingBuffer, ProducerToken};
    ///
    /// ProducerToken::new(|mut producer_token| {
    ///     let buffer = RingBuffer::<64>::new();
    ///     buffer.produce(&mut producer_token, b"hello world");
    /// });
    /// ```
    pub fn new<R, F>(fun: F) -> R
    where
        for<'new_token> F: FnOnce(ProducerToken<'new_token>) -> R,
    {
        let token = Self {
            _marker: InvariantLifetime::default(),
        };
        fun(token)
    }
}

/// Used to create a token to read and free a buffer
pub struct ConsumerToken<'consumer> {
    _marker: InvariantLifetime<'consumer>,
}

unsafe impl<'consumer> Send for ConsumerToken<'consumer> {}
unsafe impl<'consumer> Sync for ConsumerToken<'consumer> {}

impl<'consumer> ConsumerToken<'consumer> {
    /// Create a new consumer token to read from buffer
    ///
    /// # Example
    /// ```
    /// use dual_access_ringbuffer::{RingBuffer, ProducerToken, ConsumerToken};
    ///
    /// ProducerToken::new(|mut producer_token| {
    ///     ConsumerToken::new(|mut consumer_token| {
    ///         let buffer = RingBuffer::<64>::new();
    ///         buffer.produce(&mut producer_token, b"hello world");
    ///
    ///         buffer.consume(&mut consumer_token, |mut read_buffer| {
    ///             let mut read = [0u8; 32];
    ///             use std::io::Read;
    ///             let len = read_buffer.read(&mut read).unwrap();
    ///
    ///             len
    ///         })
    ///     });
    /// });
    /// ```
    pub fn new<R, F>(fun: F) -> R
    where
        for<'new_token> F: FnOnce(ConsumerToken<'new_token>) -> R,
    {
        let token = Self {
            _marker: InvariantLifetime::default(),
        };
        fun(token)
    }
}
