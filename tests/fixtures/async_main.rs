#![forbid(unsafe_code)]

use core::future::Future;
use core::pin::{Pin, pin};
use core::task::{Context, Poll, Waker};

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

async fn measure() -> u8 {
    let samples = [2_u8, 4, 6];
    Pause(false).await;
    samples[2]
}

fn main() {
    let mut cx = Context::from_waker(Waker::noop());
    let mut task = pin!(measure());
    assert!(task.as_mut().poll(&mut cx).is_pending());
    assert!(task.as_mut().poll(&mut cx) == Poll::Ready(6));
}
