//! Monotonic Chromium compositor transactions independent of authored time.
//!
//! Runtime effects may seek backward or repeat a frame. Chromium compositor
//! transactions may not, so this module gives each capture an ordered positive
//! tick while retaining the authored frame identity used for reconciliation.

use chromiumoxide::cdp::browser_protocol::headless_experimental::{
    BeginFrameParams, ScreenshotParams, ScreenshotParamsFormat,
};
use onmark_core::protocol::{WireFrame, WireFrameRate};

const SURFACE_INITIALIZATION_TIME_MILLIS: f64 = 1.0;
const COMPOSITOR_BASE_TIME_MILLIS: f64 = 1_000.0;
// One transaction may use two positive substeps. A one-millisecond stride
// keeps every substep representable and strictly before the next placement.
const COMPOSITOR_TRANSACTION_STEP_MILLIS: f64 = 1.0;
const MAX_COMPOSITOR_OFFSET_MILLIS: f64 = 0.001;

#[derive(Debug)]
pub(super) struct CompositorClock {
    initialized: bool,
    next_capture_time_millis: f64,
    active: Option<CompositorTransaction>,
}

impl CompositorClock {
    pub(super) const fn new() -> Self {
        Self {
            initialized: false,
            next_capture_time_millis: COMPOSITOR_BASE_TIME_MILLIS,
            active: None,
        }
    }

    pub(super) fn initialize(&mut self, frame_rate: WireFrameRate) -> BeginFrameParams {
        let frame_time_ticks = if self.initialized {
            self.next_capture_time_millis - COMPOSITOR_TRANSACTION_STEP_MILLIS / 2.0
        } else {
            SURFACE_INITIALIZATION_TIME_MILLIS
        };
        self.initialized = true;
        surface_initialization_parameters(frame_rate, frame_time_ticks)
    }

    pub(super) fn begin(
        &mut self,
        authored_frame: WireFrame,
        frame_rate: WireFrameRate,
    ) -> CompositorTransaction {
        let transaction = CompositorTransaction {
            authored_frame,
            capture_time_millis: self.next_capture_time_millis,
            interval_millis: frame_interval_millis(frame_rate),
        };
        self.next_capture_time_millis += COMPOSITOR_TRANSACTION_STEP_MILLIS;
        self.active = Some(transaction);
        transaction
    }

    pub(super) fn active_for(&self, authored_frame: WireFrame) -> Option<CompositorTransaction> {
        self.active
            .filter(|transaction| transaction.authored_frame == authored_frame)
    }
}

/// The ordered surface-commit, capture, retry, and reconciliation ticks for one frame.
#[derive(Clone, Copy, Debug)]
pub(super) struct CompositorTransaction {
    authored_frame: WireFrame,
    capture_time_millis: f64,
    interval_millis: f64,
}

impl CompositorTransaction {
    pub(super) fn surface_commit_parameters(self) -> BeginFrameParams {
        visual_frame_parameters(
            self.capture_time_millis - self.offset_millis(),
            self.interval_millis,
        )
    }

    pub(super) fn capture_parameters(self) -> BeginFrameParams {
        captured_frame_parameters(self.capture_time_millis, self.interval_millis)
    }

    pub(super) fn retry_parameters(self) -> BeginFrameParams {
        captured_frame_parameters(
            self.capture_time_millis + self.offset_millis(),
            self.interval_millis,
        )
    }

    pub(super) fn reconciliation_parameters(self) -> BeginFrameParams {
        captured_frame_parameters(
            self.capture_time_millis + self.offset_millis() * 2.0,
            self.interval_millis,
        )
    }

    fn offset_millis(self) -> f64 {
        (self.interval_millis / 4.0).min(MAX_COMPOSITOR_OFFSET_MILLIS)
    }
}

fn captured_frame_parameters(frame_time_ticks: f64, interval: f64) -> BeginFrameParams {
    let screenshot = ScreenshotParams::builder()
        .format(ScreenshotParamsFormat::Png)
        .optimize_for_speed(true)
        .build();

    BeginFrameParams::builder()
        .frame_time_ticks(frame_time_ticks)
        .interval(interval)
        .screenshot(screenshot)
        .build()
}

fn visual_frame_parameters(frame_time_ticks: f64, interval: f64) -> BeginFrameParams {
    BeginFrameParams::builder()
        .frame_time_ticks(frame_time_ticks)
        .interval(interval)
        .no_display_updates(false)
        .build()
}

fn surface_initialization_parameters(
    frame_rate: WireFrameRate,
    frame_time_ticks: f64,
) -> BeginFrameParams {
    visual_frame_parameters(frame_time_ticks, frame_interval_millis(frame_rate))
}

fn frame_interval_millis(frame_rate: WireFrameRate) -> f64 {
    f64::from(frame_rate.denominator()) * 1_000.0 / f64::from(frame_rate.numerator())
}

#[cfg(test)]
mod tests {
    use onmark_core::model::FrameRate;
    use onmark_core::protocol::{WireFrame, WireFrameRate};

    use super::{COMPOSITOR_TRANSACTION_STEP_MILLIS, CompositorClock};

    #[test]
    fn transactions_follow_capture_order_not_authored_frames() {
        let rate = frame_rate(30, 1);
        let mut clock = CompositorClock::new();
        let captures = [17, 3, 29, 17].map(|index| {
            let frame = WireFrame::new(index).expect("the fixture frame is browser-safe");
            compositor_time(&clock.begin(frame, rate).capture_parameters())
        });

        assert!(captures.windows(2).all(|pair| {
            (pair[1] - pair[0] - COMPOSITOR_TRANSACTION_STEP_MILLIS).abs() < f64::EPSILON
        }));
    }

    #[test]
    fn active_transaction_remembers_its_authored_frame() {
        let frame = WireFrame::new(17).expect("the fixture frame is browser-safe");
        let other = WireFrame::new(3).expect("the fixture frame is browser-safe");
        let mut clock = CompositorClock::new();
        clock.begin(frame, frame_rate(30, 1));

        assert_eq!(
            clock
                .active_for(frame)
                .map(|transaction| transaction.authored_frame),
            Some(frame),
        );
        assert!(clock.active_for(other).is_none());
    }

    #[test]
    fn surface_initialization_is_visual_and_precedes_the_capture_baseline() {
        let parameters = CompositorClock::new().initialize(frame_rate(30, 1));

        assert_eq!(parameters.frame_time_ticks, Some(1.0));
        assert_eq!(parameters.no_display_updates, Some(false));
        assert_eq!(parameters.interval, Some(1_000.0 / 30.0));
        assert_eq!(parameters.screenshot, None);
    }

    #[test]
    fn reused_surface_initialization_never_rewinds_the_clock() {
        let frame = WireFrame::new(15).expect("the fixture frame is browser-safe");
        let rate = frame_rate(30, 1);
        let mut clock = CompositorClock::new();
        let _ = clock.initialize(rate);
        let previous = compositor_time(&clock.begin(frame, rate).reconciliation_parameters());
        let initialization = compositor_time(&clock.initialize(rate));

        assert!(
            initialization > previous,
            "a reused browser target must not return to its first initialization tick",
        );
    }

    #[test]
    fn staged_surface_commit_precedes_its_capture_transaction() {
        let frame = WireFrame::new(15).expect("the fixture frame is browser-safe");
        let transaction = CompositorClock::new().begin(frame, frame_rate(30, 1));
        let commit = transaction.surface_commit_parameters();
        let capture = transaction.capture_parameters();

        assert_eq!(commit.frame_time_ticks, Some(999.999));
        assert_eq!(commit.no_display_updates, Some(false));
        assert_eq!(commit.screenshot, None);
        assert_eq!(capture.frame_time_ticks, Some(1_000.0));
        let screenshot = capture
            .screenshot
            .expect("every exact compositor frame must carry its capture");
        assert_eq!(screenshot.format, Some(super::ScreenshotParamsFormat::Png));
        assert_eq!(screenshot.optimize_for_speed, Some(true));
    }

    #[test]
    fn capture_followups_advance_by_distinct_bounded_offsets() {
        let frame = WireFrame::new(15).expect("the fixture frame is browser-safe");
        let transaction = CompositorClock::new().begin(frame, frame_rate(30, 1));
        let initial = transaction.capture_parameters();
        let retry = transaction.retry_parameters();
        let reconciliation = transaction.reconciliation_parameters();

        assert_eq!(initial.frame_time_ticks, Some(1_000.0));
        assert_eq!(retry.frame_time_ticks, Some(1_000.001));
        assert_eq!(reconciliation.frame_time_ticks, Some(1_000.002));
        assert_eq!(retry.interval, initial.interval);
        assert_eq!(retry.screenshot, initial.screenshot);
        assert_eq!(reconciliation.interval, initial.interval);
        assert_eq!(reconciliation.screenshot, initial.screenshot);
    }

    #[test]
    fn substeps_remain_between_adjacent_transactions() {
        let previous = WireFrame::new(17).expect("the fixture frame is browser-safe");
        let frame = WireFrame::new(3).expect("the fixture frame is browser-safe");
        let next = WireFrame::new(29).expect("the fixture frame is browser-safe");
        let rates = [frame_rate(u32::MAX, 1), frame_rate(1, u32::MAX)];

        for rate in rates {
            let mut clock = CompositorClock::new();
            let previous = clock.begin(previous, rate);
            let current = clock.begin(frame, rate);
            let next = clock.begin(next, rate);
            let previous_reconciliation = compositor_time(&previous.reconciliation_parameters());
            let commit = compositor_time(&current.surface_commit_parameters());
            let capture = compositor_time(&current.capture_parameters());
            let retry = compositor_time(&current.retry_parameters());
            let reconciliation = compositor_time(&current.reconciliation_parameters());
            let next_commit = compositor_time(&next.surface_commit_parameters());

            assert!(previous_reconciliation < commit);
            assert!(commit < capture);
            assert!(capture < retry);
            assert!(retry < reconciliation);
            assert!(reconciliation < next_commit);
        }
    }

    fn frame_rate(numerator: u32, denominator: u32) -> WireFrameRate {
        WireFrameRate::from(
            FrameRate::new(numerator, denominator)
                .expect("the fixture rate is canonical and nonzero"),
        )
    }

    fn compositor_time(parameters: &super::BeginFrameParams) -> f64 {
        parameters
            .frame_time_ticks
            .expect("frame parameters carry compositor time")
    }
}
