// SPDX-FileCopyrightText: 2025-2026 Lasse Heitgres
// SPDX-License-Identifier: GPL-3.0-or-later
// Additional terms: see NOTICE

use std::io;
use std::pin::Pin;
use std::task::{Context, Poll, ready};

use tokio::io::{AsyncRead, AsyncWrite, ReadBuf};

// carkitd reads 1024 plaintext bytes per TLS receive. Larger records can leave
// their tail buffered indefinitely on the phone. Bound and flush each write
// before submitting the next part, while preserving the iAP2 byte stream.
const MAX_RECORD: usize = 1024;

#[derive(Debug)]
pub(super) struct CarkitStream<S> {
    inner: S,
    flush_pending: bool,
}

impl<S> CarkitStream<S> {
    pub(super) fn new(inner: S) -> Self {
        Self {
            inner,
            flush_pending: false,
        }
    }
}

impl<S: AsyncRead + Unpin> AsyncRead for CarkitStream<S> {
    fn poll_read(
        self: Pin<&mut Self>,
        cx: &mut Context<'_>,
        buf: &mut ReadBuf<'_>,
    ) -> Poll<io::Result<()>> {
        Pin::new(&mut self.get_mut().inner).poll_read(cx, buf)
    }
}

impl<S: AsyncWrite + Unpin> AsyncWrite for CarkitStream<S> {
    fn poll_write(
        self: Pin<&mut Self>,
        cx: &mut Context<'_>,
        buf: &[u8],
    ) -> Poll<io::Result<usize>> {
        let this = self.get_mut();
        if this.flush_pending {
            ready!(Pin::new(&mut this.inner).poll_flush(cx))?;
            this.flush_pending = false;
        }
        let written =
            ready!(Pin::new(&mut this.inner).poll_write(cx, &buf[..buf.len().min(MAX_RECORD)]))?;
        this.flush_pending = written != 0;
        Poll::Ready(Ok(written))
    }

    fn poll_flush(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<io::Result<()>> {
        let this = self.get_mut();
        ready!(Pin::new(&mut this.inner).poll_flush(cx))?;
        this.flush_pending = false;
        Poll::Ready(Ok(()))
    }

    fn poll_shutdown(mut self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<io::Result<()>> {
        ready!(self.as_mut().poll_flush(cx))?;
        Pin::new(&mut self.get_mut().inner).poll_shutdown(cx)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use tokio::io::AsyncWriteExt;

    #[derive(Default)]
    struct Receiver {
        bytes: Vec<u8>,
        unflushed: bool,
        flushes: usize,
    }

    impl AsyncWrite for Receiver {
        fn poll_write(
            self: Pin<&mut Self>,
            _: &mut Context<'_>,
            buf: &[u8],
        ) -> Poll<io::Result<usize>> {
            let this = self.get_mut();
            assert!(
                !this.unflushed,
                "flush the previous TLS record before adding another"
            );
            // Model carkitd's one-read-per-record behavior: excess plaintext is
            // unavailable until another receive event, stalling the handshake.
            this.bytes
                .extend_from_slice(&buf[..buf.len().min(MAX_RECORD)]);
            this.unflushed = true;
            Poll::Ready(Ok(buf.len()))
        }
        fn poll_flush(self: Pin<&mut Self>, _: &mut Context<'_>) -> Poll<io::Result<()>> {
            let this = self.get_mut();
            this.unflushed = false;
            this.flushes += 1;
            Poll::Ready(Ok(()))
        }
        fn poll_shutdown(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<io::Result<()>> {
            self.poll_flush(cx)
        }
    }

    #[tokio::test]
    async fn a_baa_certificate_larger_than_one_record_reaches_the_phone_in_full() {
        let mut stream = CarkitStream::new(Receiver::default());
        let certificate_packet: Vec<u8> = (0..1383).map(|n| n as u8).collect();
        stream.write_all(&certificate_packet).await.unwrap();
        stream.flush().await.unwrap();
        assert_eq!(stream.inner.bytes, certificate_packet);
        assert_eq!(stream.inner.flushes, 2);
    }
}
