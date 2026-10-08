#![no_std]
#![forbid(unsafe_code)]

use core::future::Future;
use core::pin::Pin;
use core::task::{Context, Poll};

struct Pause(bool);

impl Future for Pause {
    type Output = ();

    fn poll(mut self: Pin<&mut Self>, _: &mut Context<'_>) -> Poll<()> {
        if self.0 {
            Poll::Ready(())
        } else {
            self.0 = true;
            Poll::Pending
        }
    }
}

pub async fn guarded_samples(index: u8) -> u8 {
    if index < 4 {
        let samples = [3, 5, 7, 9];
        Pause(false).await;
        samples[index as usize]
    } else {
        0
    }
}

pub async fn bounded_pauses() {
    for _ in 0..3 {
        Pause(false).await;
    }
}

pub async fn bad_after_pause() {
    Pause(false).await;
    panic!("failure in deferred body");
}

pub async fn endless_pending() {
    core::future::pending::<()>().await;
}

pub async fn unsupported_callback(read: fn() -> u8) -> u8 {
    Pause(false).await;
    read()
}

pub fn initialization() {
    panic!("failure in initialization");
}

#[cfg(test)]
mod tests {
    use super::*;
    use core::task::Waker;

    fn run(future: impl Future<Output = u8>) -> u8 {
        let mut future = core::pin::pin!(future);
        let mut context = Context::from_waker(Waker::noop());
        loop {
            if let Poll::Ready(value) = future.as_mut().poll(&mut context) {
                return value;
            }
        }
    }

    #[test]
    fn samples_remain_in_bounds_after_resumption() {
        for index in 0..=u8::MAX {
            let value = run(guarded_samples(index));
            assert_eq!(
                value,
                if index < 4 {
                    [3, 5, 7, 9][index as usize]
                } else {
                    0
                }
            );
        }
    }
}
