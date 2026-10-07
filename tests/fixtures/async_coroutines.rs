#![no_std]
#![forbid(unsafe_code)]

use core::cell::Cell;
use core::future::{Future, pending};
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

async fn immediate(value: u8) -> u8 {
    assert!(value < 6);
    value
}

pub fn guarded_immediate(value: u8, cx: &mut Context<'_>) {
    if value < 6 {
        let mut task = pin!(immediate(value));
        assert!(task.as_mut().poll(cx) == Poll::Ready(value));
    }
}

pub fn bad_immediate(value: u8, cx: &mut Context<'_>) {
    let mut task = pin!(immediate(value));
    let _ = task.as_mut().poll(cx);
}

async fn after_pause(value: u8) -> u8 {
    Pause(false).await;
    assert!(value < 6);
    value
}

pub fn guarded_resume(value: u8, cx: &mut Context<'_>) {
    if value < 6 {
        let mut task = pin!(after_pause(value));
        assert!(task.as_mut().poll(cx).is_pending());
        assert!(task.as_mut().poll(cx) == Poll::Ready(value));
    }
}

pub fn only_the_first_poll_is_checked(value: u8, cx: &mut Context<'_>) {
    let mut task = pin!(after_pause(value));
    assert!(task.as_mut().poll(cx).is_pending());
}

pub fn a_later_panic_is_reachable(value: u8, cx: &mut Context<'_>) {
    let mut task = pin!(after_pause(value));
    let _ = task.as_mut().poll(cx);
    let _ = task.as_mut().poll(cx);
}

pub fn resuming_after_completion_panics(cx: &mut Context<'_>) {
    let mut task = pin!(immediate(0));
    let _ = task.as_mut().poll(cx);
    let _ = task.as_mut().poll(cx);
}

async fn repeated_pauses(value: u8) -> u8 {
    let saved = value;
    Pause(false).await;
    assert!(saved < 6);
    Pause(false).await;
    assert!(saved == value);
    saved
}

pub fn shared_slots(value: u8, cx: &mut Context<'_>) {
    if value < 6 {
        let mut task = pin!(repeated_pauses(value));
        assert!(task.as_mut().poll(cx).is_pending());
        assert!(task.as_mut().poll(cx).is_pending());
        assert!(task.as_mut().poll(cx) == Poll::Ready(value));
    }
}

async fn nested(value: u8) -> u8 {
    after_pause(value).await;
    after_pause(value).await
}

pub fn nested_futures(value: u8, cx: &mut Context<'_>) {
    if value < 6 {
        let mut task = pin!(nested(value));
        assert!(task.as_mut().poll(cx).is_pending());
        assert!(task.as_mut().poll(cx).is_pending());
        assert!(task.as_mut().poll(cx) == Poll::Ready(value));
    }
}

async fn write_after_pause(target: &mut u8) {
    Pause(false).await;
    *target = 4;
}

pub fn captures_preserve_mutable_storage(target: &mut u8, cx: &mut Context<'_>) {
    {
        let mut task = pin!(write_after_pause(target));
        assert!(task.as_mut().poll(cx).is_pending());
        assert!(task.as_mut().poll(cx).is_ready());
    }
    assert!(*target == 4);
}

pub fn a_wrong_post_resume_write(target: &mut u8, cx: &mut Context<'_>) {
    {
        let mut task = pin!(write_after_pause(target));
        let _ = task.as_mut().poll(cx);
        let _ = task.as_mut().poll(cx);
    }
    assert!(*target == 3);
}

pub fn a_real_context_constructor() {
    let mut cx = Context::from_waker(Waker::noop());
    guarded_resume(5, &mut cx);
}

pub fn unbounded_polling(cx: &mut Context<'_>) {
    let mut task = pin!(pending::<()>());
    while task.as_mut().poll(cx).is_pending() {}
}

struct Wakes;
impl Future for Wakes {
    type Output = ();
    fn poll(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<()> {
        cx.waker().wake_by_ref();
        Poll::Pending
    }
}

pub fn context_observation_is_unknown(cx: &mut Context<'_>) {
    let mut task = pin!(Wakes);
    let _ = task.as_mut().poll(cx);
}

struct Counted<'a>(&'a Cell<u8>);
impl Drop for Counted<'_> {
    fn drop(&mut self) {
        self.0.set(self.0.get() + 1);
    }
}

async fn keep_guard(guard: Counted<'_>) {
    Pause(false).await;
    drop(guard);
}

pub fn cancellation_preserves_drop_effects() {
    let hits = Cell::new(0);
    {
        let mut task = pin!(keep_guard(Counted(&hits)));
        assert!(
            task.as_mut()
                .poll(&mut Context::from_waker(Waker::noop()))
                .is_pending()
        );
    }
    assert!(hits.get() == 1);
}

struct Explosive;
impl Drop for Explosive {
    fn drop(&mut self) {
        panic!("cancellation must check captured destructors");
    }
}

async fn owned_capture(value: Explosive) {
    Pause(false).await;
    drop(value);
}

pub fn cancellation_checks_the_destructor(cx: &mut Context<'_>) {
    let mut task = pin!(owned_capture(Explosive));
    let _ = task.as_mut().poll(cx);
}

async fn indirect(read: fn() -> u8) -> u8 {
    Pause(false).await;
    read()
}

pub fn an_indirect_awaited_call_is_unknown(read: fn() -> u8, cx: &mut Context<'_>) {
    let mut task = pin!(indirect(read));
    let _ = task.as_mut().poll(cx);
    let _ = task.as_mut().poll(cx);
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn polls_and_saved_values_match_native_execution() {
        let mut cx = Context::from_waker(Waker::noop());
        for value in 0..=u8::MAX {
            guarded_immediate(value, &mut cx);
            guarded_resume(value, &mut cx);
            only_the_first_poll_is_checked(value, &mut cx);
            shared_slots(value, &mut cx);
            nested_futures(value, &mut cx);
        }
        let mut target = 0;
        captures_preserve_mutable_storage(&mut target, &mut cx);
        a_real_context_constructor();
        cancellation_preserves_drop_effects();
    }

    #[test]
    #[should_panic]
    fn a_resumed_panic_replays() {
        let mut cx = Context::from_waker(Waker::noop());
        a_later_panic_is_reachable(6, &mut cx);
    }

    #[test]
    #[should_panic]
    fn cancellation_replays_the_captured_destructor() {
        let mut cx = Context::from_waker(Waker::noop());
        cancellation_checks_the_destructor(&mut cx);
    }

    #[test]
    #[should_panic]
    fn polling_a_completed_future_replays_the_state_check() {
        let mut cx = Context::from_waker(Waker::noop());
        resuming_after_completion_panics(&mut cx);
    }
}
