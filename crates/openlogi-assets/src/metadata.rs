//! Parses the per-depot hotspot metadata shipped by the Logi Options+
//! installer (and re-hosted by assets.openlogi.org) — `core_metadata.json`
//! on newer depots, `metadata.json` on older ones. The caller picks the
//! filename and hands the path to [`Metadata::load_from`].
//!
//! The two generations *mostly* share a schema, but older `metadata.json`
//! files (e.g. the G513 keyboard depot) identify assignments by `slotId`
//! only — there is no `slotName` — so every observed-optional field must
//! stay soft: one missing field would otherwise fail the whole file and
//! drop the `origin` dimensions the renderer needs.
//!
//! Only the fields OpenLogi actually consumes are deserialized — every
//! other field is silently ignored. The schema below is observed-from-the-
//! wild, not derived from any Logitech specification.
//!
//! ```json
//! {
//!   "images": [
//!     {
//!       "key": "device_image",
//!       "origin": { "width": 687, "height": 1024 }
//!     },
//!     {
//!       "key": "device_buttons_image",
//!       "origin": { "width": 687, "height": 1024 },
//!       "assignments": [
//!         { "slotId": "...", "slotName": "SLOT_NAME_MIDDLE_BUTTON",
//!           "marker": { "x": 73, "y": 18 },
//!           "label":  { "x": 1,  "y": 0  } }
//!       ]
//!     }
//!   ]
//! }
//! ```
//!
//! `marker.{x,y}` is a percentage 0..100 of the device image's origin
//! dimensions. `label.{x,y}` is a direction code (-1 = left, 0 = centre,
//! +1 = right; same for y) hinting where the annotation card should sit
//! relative to the marker.
//!
//! The legacy G HUB generation (G-series mice and keyboards) differs in three
//! ways: its assignments hang off the `device_image` entry itself, each
//! `slotId` names a G-key (`g305_g4_m1` is G4 in mode 1), and every marker is
//! in absolute pixels of `origin`. [`Metadata::legacy_markers`] is the one
//! reader of that generation.

use std::path::Path;

use serde::Deserialize;

use crate::error::AssetError;
use crate::http;

#[derive(Debug, Clone, Default, PartialEq, Deserialize)]
pub struct Metadata {
    pub images: Vec<ImageEntry>,
}

#[derive(Debug, Clone, PartialEq, Deserialize)]
pub struct ImageEntry {
    pub key: String,
    pub origin: Origin,
    #[serde(default)]
    pub assignments: Vec<Assignment>,
}

#[derive(Debug, Deserialize, Clone, Copy, PartialEq, Eq)]
pub struct Origin {
    pub width: u32,
    pub height: u32,
}

#[derive(Debug, Clone, PartialEq, Deserialize)]
pub struct Assignment {
    /// Logi's per-depot slot identifier, e.g. `mx-keys-mini-2b369_c266`. On
    /// keyboard depots the `_c<decimal>` suffix is the key's HID++ `0x1b04`
    /// control ID — see [`Assignment::control_id`]. Empty on depots that
    /// predate the field.
    #[serde(rename = "slotId", default)]
    pub slot_id: String,
    /// Empty on older keyboard depots whose assignments carry only `slotId`;
    /// `map_slot_name`-style consumers treat unknown names as "no hotspot".
    #[serde(rename = "slotName", default)]
    pub slot_name: String,
    /// Camera depots ship marker-less settings-slot assignments (under the
    /// `device_camera_image` entry, which no hotspot consumer reads); a
    /// missing marker defaults to the origin rather than failing the file.
    #[serde(default)]
    pub marker: Point,
    #[serde(default)]
    pub label: Direction,
}

impl Assignment {
    /// The HID++ `0x1b04` control ID Logi's slot identifier encodes: the
    /// decimal after the trailing `_c`, as in `ergo-k860-6b359_c111` for the
    /// Lock key (`0x006f`). `None` for settings slots
    /// (`…_touchpad_settings`) and any depot that names slots differently.
    #[must_use]
    pub fn control_id(&self) -> Option<u16> {
        let (_, cid) = self.slot_id.rsplit_once("_c")?;
        cid.parse().ok()
    }

    /// The G-key a legacy G HUB slot identifier names: the decimal between the
    /// trailing `_g` and the `_m<mode>` suffix, as in `g305_g4_m1` for G4.
    /// `None` for Options+ slots and for G HUB slots that are not G-keys
    /// (`g604_scroll1_m1`).
    #[must_use]
    pub fn g_key(&self) -> Option<u8> {
        let (head, mode) = self.slot_id.rsplit_once("_m")?;
        mode.parse::<u8>().ok()?;
        let (_, key) = head.rsplit_once("_g")?;
        key.parse().ok()
    }
}

/// A legacy pixel marker placed on the cached render, as fractions (0..=1)
/// of its width and height.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct LegacyMarker<'a> {
    pub assignment: &'a Assignment,
    pub x_frac: f32,
    pub y_frac: f32,
}

/// How far, per axis, a render may differ from the canvas legacy markers were
/// authored on and still take them. The G305 depot's black render is one
/// pixel narrower than its `origin` (1399 vs 1400) and its colour variants
/// under 1% larger; a marker set authored for a different render (the G513's
/// `metadata.json`, drawn on the G512 banner) is off by more than 20%.
const LEGACY_RENDER_TOLERANCE: f64 = 0.01;

#[derive(Debug, Deserialize, Clone, Copy, Default, PartialEq)]
pub struct Point {
    pub x: f32,
    pub y: f32,
}

#[derive(Debug, Deserialize, Clone, Copy, Default, PartialEq, Eq)]
pub struct Direction {
    pub x: i32,
    pub y: i32,
}

impl Metadata {
    /// Load and parse a metadata JSON file from disk.
    pub fn load_from(path: &Path) -> Result<Self, AssetError> {
        http::load_json(path)
    }

    /// Image dimensions (use the `device_image` entry — both entries
    /// always share the same origin in practice).
    #[must_use]
    pub fn origin(&self) -> Option<Origin> {
        self.images.first().map(|i| i.origin)
    }

    /// Raw assignment iterator over the `device_buttons_image` entry.
    /// Slot-name → application-button mapping is intentionally left to
    /// the consumer (the GUI owns the ButtonId enum).
    pub fn assignments(&self) -> impl Iterator<Item = &Assignment> + '_ {
        self.images
            .iter()
            .find(|i| i.key == "device_buttons_image")
            .into_iter()
            .flat_map(|img| img.assignments.iter())
    }

    /// The legacy G HUB markers placed on a `png_width` × `png_height`
    /// render, or nothing when the depot has none or authored them against a
    /// different render — a marker set drawn for another canvas would misplace
    /// every callout.
    ///
    /// Percent-schema depots never exceed 100 on either axis, so a marker that
    /// does not is dropped: mixed files don't exist in the wild, but a percent
    /// marker slipping through would land off by an order of magnitude.
    #[must_use]
    pub fn legacy_markers(&self, png_width: u32, png_height: u32) -> Vec<LegacyMarker<'_>> {
        let Some(img) = self
            .images
            .iter()
            .find(|img| img.key == "device_image" && !img.assignments.is_empty())
        else {
            return Vec::new();
        };
        let fits = |rendered: u32, authored: u32| {
            authored > 0
                && (f64::from(rendered) / f64::from(authored) - 1.0).abs()
                    <= LEGACY_RENDER_TOLERANCE
        };
        if !fits(png_width, img.origin.width) || !fits(png_height, img.origin.height) {
            return Vec::new();
        }
        #[expect(
            clippy::cast_precision_loss,
            reason = "depot image dimensions are a few thousand pixels at most"
        )]
        let (w, h) = (img.origin.width as f32, img.origin.height as f32);
        img.assignments
            .iter()
            .filter(|asg| asg.marker.x > 100. || asg.marker.y > 100.)
            .map(|assignment| LegacyMarker {
                assignment,
                x_frac: (assignment.marker.x / w).clamp(0.0, 1.0),
                y_frac: (assignment.marker.y / h).clamp(0.0, 1.0),
            })
            .collect()
    }
}

#[cfg(test)]
mod tests {
    use super::Metadata;

    /// Older keyboard depots (G513) identify assignments by `slotId` only —
    /// no `slotName` — and add fields like `assignmentOffset`. Parsing must
    /// not fail wholesale: the renderer still needs `origin`, and unknown
    /// slot names already degrade to "no hotspot" in the consumer.
    #[test]
    fn slot_ids_decode_their_control_id() {
        let json = r#"{
          "images": [
            {
              "key": "device_keys_image",
              "origin": { "width": 1872, "height": 728 },
              "assignments": [
                { "slotId": "mx-keys-mini-2b369_c266", "slotName": "SLOT_NAME_SCREEN_CAPTURE",
                  "marker": { "x": 53.5, "y": 13.8 } },
                { "slotId": "k400-4404d_touchpad_settings", "slotName": "SLOT_NAME_ZOOM_GESTURE",
                  "marker": { "x": 79, "y": 48 } },
                { "slotId": "g513_g1_m1", "marker": { "x": 370, "y": 300 } }
              ]
            }
          ]
        }"#;
        let meta: Metadata = serde_json::from_str(json).expect("parses");
        let cids: Vec<Option<u16>> = meta.images[0]
            .assignments
            .iter()
            .map(super::Assignment::control_id)
            .collect();
        assert_eq!(cids, vec![Some(0x010a), None, None]);
    }

    #[test]
    fn old_slot_id_only_metadata_parses() {
        let json = r#"{
          "images": [
            {
              "key": "device_image",
              "origin": { "width": 3598, "height": 1315 },
              "assignmentOffset": { "x": 800, "y": 0 },
              "assignments": [
                { "slotId": "g513_g1_m1",
                  "marker": { "x": 370, "y": 300 },
                  "label":  { "x": -1200, "y": 300 } }
              ]
            }
          ]
        }"#;
        let meta: Metadata = serde_json::from_str(json).expect("old schema must parse");
        let origin = meta.origin().expect("origin survives");
        assert_eq!((origin.width, origin.height), (3598, 1315));
        assert_eq!(meta.images[0].assignments[0].slot_name, "");
    }

    /// Camera depots (StreamCam) list settings-slot assignments with no
    /// `marker` under their `device_camera_image` entry. Parsing must not
    /// fail wholesale, and `assignments()` must not surface them (it reads
    /// only the `device_buttons_image` entry).
    #[test]
    fn camera_metadata_without_markers_parses() {
        let json = r#"{
          "images": [
            { "key": "device_image", "origin": { "width": 1280, "height": 800 } },
            {
              "key": "device_camera_image",
              "origin": { "width": 396, "height": 396 },
              "assignments": [
                { "slotId": "streamcam-0893_webcam_camera_settings",
                  "slotName": "SLOT_NAME_WEBCAM_CAMERA_SETTINGS",
                  "disableAssignmentClick": true }
              ]
            }
          ]
        }"#;
        let meta: Metadata = serde_json::from_str(json).expect("camera schema must parse");
        let origin = meta.origin().expect("origin survives");
        assert_eq!((origin.width, origin.height), (1280, 800));
        assert_eq!(meta.assignments().count(), 0);
    }

    /// The G305 depot's `metadata.json`, trimmed to three of its six slots:
    /// the wheel (G3) and the two side buttons (G4 rear, G5 front).
    const G305: &str = r#"{
      "images": [
        {
          "key": "device_image",
          "origin": { "width": 1400, "height": 2514 },
          "assignments": [
            { "slotId": "g305_g3_m1", "marker": { "x": 709, "y": 475 },
              "label": { "x": 709, "y": -200 } },
            { "slotId": "g305_g4_m1", "marker": { "x": 4, "y": 1386 },
              "label": { "x": -1400, "y": 1386 } },
            { "slotId": "g305_g5_m1", "marker": { "x": 13, "y": 976 },
              "label": { "x": -1400, "y": 976 } }
          ]
        }
      ]
    }"#;

    #[test]
    fn g_hub_slot_ids_decode_their_g_key() {
        let key = |slot_id: &str| {
            super::Assignment {
                slot_id: slot_id.to_owned(),
                slot_name: String::new(),
                marker: super::Point::default(),
                label: super::Direction::default(),
            }
            .g_key()
        };
        assert_eq!(key("g305_g4_m1"), Some(4));
        assert_eq!(key("g502wireless_g11_m1"), Some(11));
        assert_eq!(key("prowirelessmouse_g5_m2"), Some(5));
        // G HUB slots that are not G-keys, and every Options+ slot.
        assert_eq!(key("g604_scroll1_m1"), None);
        assert_eq!(key("mx-keys-mini-2b369_c266"), None);
        assert_eq!(key("m720-triathlon-6b015_mouse_settings"), None);
        assert_eq!(key(""), None);
    }

    /// The black G305 render is 1399 px wide against a 1400 px origin: a
    /// rounding difference, not a different canvas.
    #[test]
    fn legacy_markers_survive_a_rounding_difference_in_the_render() {
        let meta: Metadata = serde_json::from_str(G305).expect("parses");
        let markers = meta.legacy_markers(1399, 2514);
        let g_keys: Vec<Option<u8>> = markers.iter().map(|m| m.assignment.g_key()).collect();
        assert_eq!(g_keys, vec![Some(3), Some(4), Some(5)]);
        let rear = markers[1];
        assert!((rear.x_frac - 4. / 1400.).abs() < 1e-6);
        assert!((rear.y_frac - 1386. / 2514.).abs() < 1e-6);
    }

    #[test]
    fn legacy_markers_for_a_different_render_are_rejected() {
        let meta: Metadata = serde_json::from_str(G305).expect("parses");
        // Within 1% on one axis is not enough when the other is off.
        assert!(meta.legacy_markers(1400, 2800).is_empty());
        assert!(meta.legacy_markers(1687, 2514).is_empty());
        assert!(meta.legacy_markers(0, 0).is_empty());
    }

    /// A percent marker in a pixel file would land near the canvas corner.
    #[test]
    fn legacy_markers_drop_percent_scale_points() {
        let json = r#"{
          "images": [
            {
              "key": "device_image",
              "origin": { "width": 1400, "height": 2514 },
              "assignments": [
                { "slotId": "g305_g3_m1", "marker": { "x": 709, "y": 475 } },
                { "slotId": "g305_g4_m1", "marker": { "x": 50, "y": 40 } }
              ]
            }
          ]
        }"#;
        let meta: Metadata = serde_json::from_str(json).expect("parses");
        let markers = meta.legacy_markers(1400, 2514);
        assert_eq!(markers.len(), 1);
        assert_eq!(markers[0].assignment.g_key(), Some(3));
    }

    /// Options+ depots keep their assignments on `device_buttons_image`;
    /// their `device_image` entry carries none.
    #[test]
    fn options_plus_metadata_has_no_legacy_markers() {
        let json = r#"{
          "images": [
            { "key": "device_image", "origin": { "width": 687, "height": 1024 } },
            {
              "key": "device_buttons_image",
              "origin": { "width": 687, "height": 1024 },
              "assignments": [
                { "slotId": "x_c82", "slotName": "SLOT_NAME_MIDDLE_BUTTON",
                  "marker": { "x": 73, "y": 18 } }
              ]
            }
          ]
        }"#;
        let meta: Metadata = serde_json::from_str(json).expect("parses");
        assert!(meta.legacy_markers(687, 1024).is_empty());
        assert_eq!(meta.assignments().count(), 1);
    }
}
