//! Which device a macOS middle or side button press came from.
//!
//! macOS attaches no IOHIDEvent to `OtherMouseDown`/`OtherMouseUp` — a G305's
//! wheel and side buttons arrive without a sender, while its left clicks,
//! scrolls and motion name their receiver — so those presses cannot be
//! attributed the way every other pointer event is. They inherit the device
//! behind the pointer stream instead: whatever last clicked, scrolled or moved
//! the pointer. A trackpad stays a trackpad under that rule, so a press after
//! trackpad use still fails the remap policy closed.

use std::time::{Duration, Instant};

use crate::EventDevice;

/// How often pointer motion refreshes the attribution. Motion arrives at the
/// mouse's report rate (1 kHz on a G305); sampling keeps the sender lookup off
/// almost all of it while still following a switch from one device to another
/// within a fraction of a second.
const MOTION_SAMPLE_INTERVAL: Duration = Duration::from_millis(50);

/// The device behind the most recent attributed pointer event, owned by the
/// tap thread.
#[derive(Debug, Default)]
pub(crate) struct PointerSource {
    device: Option<EventDevice>,
    motion_sampled_at: Option<Instant>,
}

impl PointerSource {
    /// Record the device a click, scroll or sampled motion event named.
    pub(crate) fn observe(&mut self, device: &EventDevice) {
        if self.device.as_ref() != Some(device) {
            self.device = Some(device.clone());
        }
    }

    /// Whether a motion event at `now` should pay for a sender lookup. Claims
    /// the sample, so the caller must look the sender up when this is `true`.
    pub(crate) fn claim_motion_sample(&mut self, now: Instant) -> bool {
        if self
            .motion_sampled_at
            .is_some_and(|at| now.saturating_duration_since(at) < MOTION_SAMPLE_INTERVAL)
        {
            return false;
        }
        self.motion_sampled_at = Some(now);
        true
    }

    /// The device a sender-less button press is attributed to, or `None`
    /// before any pointer event has named one.
    pub(crate) fn inherited(&self) -> Option<EventDevice> {
        self.device.clone()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn device(vendor_id: u32, product_name: &str) -> EventDevice {
        EventDevice {
            vendor_id: Some(vendor_id),
            product_id: None,
            product_name: Some(product_name.to_owned()),
        }
    }

    #[test]
    fn nothing_is_inherited_before_a_pointer_event_names_a_device() {
        assert_eq!(PointerSource::default().inherited(), None);
    }

    #[test]
    fn a_press_inherits_the_last_device_that_drove_the_pointer() {
        let receiver = device(0x046d, "USB Receiver");
        let trackpad = device(0x05ac, "Apple Internal Keyboard / Trackpad");
        let mut source = PointerSource::default();

        source.observe(&receiver);
        assert_eq!(source.inherited(), Some(receiver.clone()));

        // Moving to the trackpad hands attribution over, so a press now is
        // the trackpad's and stays unremappable.
        source.observe(&trackpad);
        let inherited = source.inherited().expect("trackpad attribution");
        assert!(!crate::source_is_remappable(Some(&inherited)));

        source.observe(&receiver);
        let inherited = source.inherited().expect("receiver attribution");
        assert!(crate::source_is_remappable(Some(&inherited)));
    }

    #[test]
    fn motion_is_sampled_once_per_interval() {
        let start = Instant::now();
        let mut source = PointerSource::default();

        assert!(source.claim_motion_sample(start));
        assert!(!source.claim_motion_sample(start + Duration::from_millis(10)));
        assert!(!source.claim_motion_sample(
            start + MOTION_SAMPLE_INTERVAL.saturating_sub(Duration::from_millis(1))
        ));
        assert!(source.claim_motion_sample(start + MOTION_SAMPLE_INTERVAL));
    }
}
