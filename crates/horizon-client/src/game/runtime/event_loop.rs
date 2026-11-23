//! What the game asks of the loop that runs it.
//!
//! A window's game runs inside winit's event loop, and its methods used to take winit's
//! `ActiveEventLoop` only to stop the loop or to say when to wake next. A headless client
//! (#4951) has no display, so it cannot have that loop: it runs its own, and hands the same
//! methods this trait instead. winit's loop implements it, so a window's game is unchanged.

use std::time::{Duration, Instant};
use winit::event_loop::{ActiveEventLoop, ControlFlow};

/// A successful frame only permits polling until the next idle iteration. If
/// the compositor stops delivering redraws, the old result must not keep the
/// loop spinning indefinitely. Offscreen frames are driven by the loop itself.
pub(super) fn redraw_flow(frame_drawn: &mut bool, offscreen: bool, now: Instant) -> ControlFlow {
    let drew_frame = std::mem::take(frame_drawn);
    if offscreen || drew_frame {
        ControlFlow::Poll
    } else {
        wait_until(now + Duration::from_millis(16))
    }
}

/// `ControlFlow::WaitUntil(at)`. In the browser winit keeps time on its own clock (web-time's
/// `Instant`), so the wait goes over as a duration from now.
pub(super) fn wait_until(at: Instant) -> ControlFlow {
    #[cfg(not(target_arch = "wasm32"))]
    return ControlFlow::WaitUntil(at);
    #[cfg(target_arch = "wasm32")]
    ControlFlow::wait_duration(at.saturating_duration_since(Instant::now()))
}

/// The time a `ControlFlow::WaitUntil` names, on std's clock.
#[cfg(not(target_arch = "wasm32"))]
pub(super) fn std_instant(at: Instant) -> Instant {
    at
}

/// The time a `ControlFlow::WaitUntil` names, on std's clock: the browser's winit names it on
/// web-time's.
#[cfg(target_arch = "wasm32")]
pub(super) fn std_instant(at: web_time::Instant) -> Instant {
    Instant::now() + at.saturating_duration_since(web_time::Instant::now())
}

pub(super) trait EventLoopCtl {
    /// Ends the loop after the current iteration (`ActiveEventLoop::exit`).
    fn exit(&self);
    /// When the loop wakes next (`ActiveEventLoop::set_control_flow`).
    fn set_control_flow(&self, flow: ControlFlow);
}

impl EventLoopCtl for ActiveEventLoop {
    fn exit(&self) {
        ActiveEventLoop::exit(self);
    }
    fn set_control_flow(&self, flow: ControlFlow) {
        ActiveEventLoop::set_control_flow(self, flow);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn missing_redraw_waits_and_a_new_frame_resumes_polling() {
        let now = Instant::now();
        let mut frame_drawn = true;
        assert_eq!(redraw_flow(&mut frame_drawn, false, now), ControlFlow::Poll);
        assert_eq!(
            redraw_flow(&mut frame_drawn, false, now),
            ControlFlow::WaitUntil(now + Duration::from_millis(16))
        );
        frame_drawn = true;
        assert_eq!(redraw_flow(&mut frame_drawn, false, now), ControlFlow::Poll);
    }

    #[test]
    fn offscreen_keeps_polling_without_a_presented_frame() {
        let mut frame_drawn = false;
        assert_eq!(
            redraw_flow(&mut frame_drawn, true, Instant::now()),
            ControlFlow::Poll
        );
    }
}
