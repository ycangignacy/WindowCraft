// Made by Nordan
//! Frame pacing against the display refresh.
//!
//! The software renderer presents through GDI, which never waits for the
//! display. Left alone, an animation renders as fast as the CPU allows, so
//! frames land at uneven points between refreshes and motion judders while
//! one core runs flat out. Animations here instead wake once per refresh, a
//! moment after the vertical blank that DWM reports, which gives every frame
//! the same time step.

use iced::futures::channel::mpsc;
use iced::{Subscription, window};
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::{Duration, Instant};
use windows::Win32::Foundation::HWND;
use windows::Win32::Graphics::Dwm::{DWM_TIMING_INFO, DwmGetCompositionTimingInfo};
use windows::Win32::System::Performance::{QueryPerformanceCounter, QueryPerformanceFrequency};

/// Used when DWM gives no timing, for example in a session without composition.
const FALLBACK_PERIOD: Duration = Duration::from_micros(16_667);
/// Wake this long after the blank, so timer jitter never lands a wake on the
/// wrong side of it.
const PHASE: Duration = Duration::from_micros(1_000);

/// Set when the renderer itself waits for the vertical blank, as wgpu does
/// with its default present mode. Pacing on top of that only adds a second
/// clock that drifts against the first, and the frames come out uneven.
static PRESENTER_WAITS: AtomicBool = AtomicBool::new(false);

pub fn set_presenter_waits(waits: bool) {
    PRESENTER_WAITS.store(waits, Ordering::Relaxed);
}

/// The redraw an animation asks for after drawing the frame at `now`.
pub fn next_redraw(now: Instant) -> window::RedrawRequest {
    if PRESENTER_WAITS.load(Ordering::Relaxed) {
        window::RedrawRequest::NextFrame
    } else {
        window::RedrawRequest::At(next_frame(now))
    }
}

/// The frame clock for animations driven by app messages.
pub fn animation_frames() -> Subscription<Instant> {
    if PRESENTER_WAITS.load(Ordering::Relaxed) { window::frames() } else { frames() }
}

/// Last vertical blank and refresh period as DWM reports them.
fn timing() -> Option<(Instant, Duration)> {
    let mut info = DWM_TIMING_INFO { cbSize: size_of::<DWM_TIMING_INFO>() as u32, ..Default::default() };
    let (mut counter, mut frequency) = (0_i64, 0_i64);
    // SAFETY: `info` has cbSize set, the null window asks for desktop timing
    // (required since Windows 8.1), and both counters are valid i64s.
    unsafe {
        DwmGetCompositionTimingInfo(HWND::default(), &raw mut info).ok()?;
        QueryPerformanceCounter(&raw mut counter).ok()?;
        QueryPerformanceFrequency(&raw mut frequency).ok()?;
    }
    let now = Instant::now();
    if frequency <= 0 {
        return None;
    }
    let ticks = |value: f64| Duration::from_secs_f64(value.abs() / frequency as f64);
    let period = ticks(info.qpcRefreshPeriod as f64);
    // Anything outside 20 to 500 Hz is a bogus reading.
    if !(Duration::from_millis(2)..=Duration::from_millis(50)).contains(&period) {
        return None;
    }
    let age = counter as f64 - info.qpcVBlank as f64;
    let vblank = if age >= 0.0 { now.checked_sub(ticks(age))? } else { now + ticks(age) };
    Some((vblank, period))
}

/// The first wake time after `after` that sits just past a vertical blank.
pub fn next_frame(after: Instant) -> Instant {
    let Some((vblank, period)) = timing() else {
        return after + FALLBACK_PERIOD;
    };
    next_after(vblank + PHASE, period, after)
}

fn next_after(anchor: Instant, period: Duration, after: Instant) -> Instant {
    if anchor > after {
        return anchor;
    }
    let behind = after.duration_since(anchor).as_nanos();
    let steps = behind / period.as_nanos() + 1;
    anchor + period * u32::try_from(steps).unwrap_or(u32::MAX)
}

/// Emits one instant per display refresh for as long as it is subscribed.
/// The thread behind it ends with the subscription. A frame the app has not
/// picked up yet is dropped, so a slow frame never queues a backlog.
pub fn frames() -> Subscription<Instant> {
    Subscription::run(clock)
}

fn clock() -> mpsc::Receiver<Instant> {
    let (mut sender, receiver) = mpsc::channel(0);
    let spawned = std::thread::Builder::new().name("frame-clock".to_owned()).spawn(move || {
        loop {
            let wake = next_frame(Instant::now());
            std::thread::sleep(wake.saturating_duration_since(Instant::now()));
            if let Err(error) = sender.try_send(Instant::now())
                && error.is_disconnected()
            {
                break;
            }
        }
    });
    drop(spawned);
    receiver
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_next_frame_is_on_the_refresh_grid_and_strictly_later() {
        let anchor = Instant::now();
        let period = Duration::from_millis(10);
        assert_eq!(next_after(anchor, period, anchor), anchor + period);
        assert_eq!(next_after(anchor, period, anchor + Duration::from_millis(25)), anchor + Duration::from_millis(30));
        let later = anchor + Duration::from_millis(5);
        assert_eq!(next_after(later, period, anchor), later);
    }

    #[test]
    fn next_frame_is_within_one_refresh() {
        let now = Instant::now();
        let wake = next_frame(now);
        assert!(wake > now);
        assert!(wake - now <= Duration::from_millis(51), "{:?}", wake - now);
    }
}
