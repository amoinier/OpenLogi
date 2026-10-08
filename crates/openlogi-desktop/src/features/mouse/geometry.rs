//! Geometry helpers for the centre mouse model.
//!
//! These functions keep Logitech asset coordinate translation and fallback
//! label layout separate from the GPUI element tree in `view`.

use openlogi_core::binding::ButtonId;
use openlogi_core::device::Capabilities;

use super::hotspots::{Hotspot, MOUSE_MODEL_SIZE, MouseControlId};
use super::leader_lines::{Label, Side};
use crate::services::assets::ResolvedAsset;

/// Approx pixel width of each hotspot hit-target. Logitech only gives us a
/// marker point per button, not a rectangle, so we size by hand.
const ASSET_HOTSPOT: f32 = 56.;

/// Height of a side-label card. The layout needs it to group related cards
/// without allowing them to overlap at the minimum model height.
pub(super) const LABEL_H: f32 = 56.;

/// Empty space between the grouped Back and Forward cards when the viewport
/// has enough room to pull them closer than the regular even spacing.
const NAVIGATION_GROUP_GAP: f32 = 16.;

/// Whether label cards occupy one or both sides of the device render.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum LabelDistribution {
    LeftOnly,
    BothSides,
}

/// Scale the device image to *fit inside* a `max_w` × `target_h` box while
/// preserving the **actual PNG's** aspect ratio. A tall device (a mouse) is
/// bound by the height; a wide one (a keyboard) is bound by the width — which
/// is what stops a wide keyboard render from overflowing the panel (#272).
///
/// The metadata's `origin` reports the silhouette bbox inside the PNG, which
/// is typically narrower than the full image (Logi pads transparent strips on
/// both sides); sizing by origin causes `ObjectFit::Contain` to letterbox
/// vertically and pulls every hotspot off the rendered button.
#[expect(
    clippy::cast_precision_loss,
    reason = "device images are < 4096 px on either axis — well within f32 mantissa"
)]
pub fn asset_dimensions_for_png(asset: &ResolvedAsset, target_h: f32, max_w: f32) -> (f32, f32) {
    if asset.png_height == 0 {
        return MOUSE_MODEL_SIZE;
    }
    let aspect = (asset.png_width as f32) / (asset.png_height as f32);
    let w = target_h * aspect;
    if w > max_w {
        (max_w, max_w / aspect)
    } else {
        (w, target_h)
    }
}

/// Whether the asset exposes any remappable button markers. Mice do (so the
/// model reserves a side gutter for their leader-line labels); keyboards and
/// other label-less devices don't, so the model can hand them the full width.
pub fn asset_has_button_labels(asset: &ResolvedAsset) -> bool {
    !asset_markers(asset).is_empty()
}

/// Convert the asset's button markers into mouse-local pixel rects over a
/// `mouse_w` × `mouse_h` render of its PNG.
pub fn asset_hotspots_for_png(asset: &ResolvedAsset, mouse_w: f32, mouse_h: f32) -> Vec<Hotspot> {
    asset_markers(asset)
        .into_iter()
        .map(|marker| {
            let (cx, cy) = (marker.x_frac * mouse_w, marker.y_frac * mouse_h);
            Hotspot {
                id: marker.id,
                x: cx - ASSET_HOTSPOT / 2.,
                y: cy - ASSET_HOTSPOT / 2.,
                w: ASSET_HOTSPOT,
                h: ASSET_HOTSPOT,
            }
        })
        .collect()
}

/// One control the asset marks, as fractions of the rendered PNG.
struct AssetMarker {
    id: MouseControlId,
    x_frac: f32,
    y_frac: f32,
}

/// Every control the asset marks: the Options+ `device_buttons_image`
/// markers when the depot has them, otherwise the legacy G HUB G-key markers
/// a G-series depot (the G305's) carries instead.
fn asset_markers(asset: &ResolvedAsset) -> Vec<AssetMarker> {
    let options_plus = options_plus_markers(asset);
    if !options_plus.is_empty() {
        return options_plus;
    }
    asset
        .metadata
        .legacy_markers(asset.png_width, asset.png_height)
        .into_iter()
        .filter_map(|marker| {
            Some(AssetMarker {
                id: map_g_key(marker.assignment.g_key()?)?,
                x_frac: marker.x_frac,
                y_frac: marker.y_frac,
            })
        })
        .collect()
}

/// Translate Logitech's percent-based markers from the metadata's "origin"
/// coord system (the silhouette bbox) into the actual rendered PNG.
///
/// Logi's markers are percentages of `origin` (the silhouette bbox).
/// Within the actual PNG, that bbox is centred with equal padding on the
/// left and right. We render at the *PNG's* full aspect (no letterboxing)
/// so, with `bbox = origin.width / png.width`, the marker translation is:
///
/// ```text
/// x_frac = (1 - bbox) / 2 + marker.x / 100 * bbox
/// y_frac = marker.y / 100     // height ratio is 1:1
/// ```
///
/// Primary left/right clicks deliberately have no entry — Logi never
/// exposes them as remappable (and Options+ doesn't either), so we don't
/// invent markers for them.
#[expect(
    clippy::cast_precision_loss,
    reason = "device images are < 4096 px on either axis — well within f32 mantissa"
)]
fn options_plus_markers(asset: &ResolvedAsset) -> Vec<AssetMarker> {
    let png_w = asset.png_width as f32;
    let origin_w = asset
        .metadata
        .origin()
        .map_or(png_w, |o| o.width as f32)
        .min(png_w);
    let bbox = if png_w > 0. { origin_w / png_w } else { 1. };
    asset
        .metadata
        .assignments()
        .filter_map(|a| {
            Some(AssetMarker {
                id: map_slot_name(&a.slot_name)?,
                x_frac: (1. - bbox) / 2. + a.marker.x / 100. * bbox,
                y_frac: a.marker.y / 100.,
            })
        })
        .collect()
}

/// Lay labels out evenly down one or both sides of the mouse. A two-sided
/// layout sends the leftmost half of the hotspots left and the rightmost half
/// right, then orders each side by hotspot height. Back and Forward stay
/// adjacent when both are on the same side because they form one navigation
/// pair, even when another marker sits between them.
#[expect(
    clippy::cast_precision_loss,
    reason = "hotspot count is bounded by ButtonId variants — well under f32 mantissa"
)]
pub fn labels_from_hotspots(
    hotspots: &[Hotspot],
    mouse_h: f32,
    distribution: LabelDistribution,
) -> Vec<Label> {
    if hotspots.is_empty() {
        return Vec::new();
    }

    let mut labels: Vec<Label> = hotspots
        .iter()
        .map(|hotspot| Label {
            id: hotspot.id,
            side: Side::Left,
            y: 0.,
        })
        .collect();
    if distribution == LabelDistribution::BothSides {
        let mut horizontal_order: Vec<usize> = (0..hotspots.len()).collect();
        horizontal_order
            .sort_by(|&a, &b| hotspots[a].center().0.total_cmp(&hotspots[b].center().0));
        for index in horizontal_order
            .into_iter()
            .skip(hotspots.len().div_ceil(2))
        {
            labels[index].side = Side::Right;
        }
    }

    for side in [Side::Left, Side::Right] {
        let mut vertical_order: Vec<usize> = labels
            .iter()
            .enumerate()
            .filter_map(|(index, label)| (label.side == side).then_some(index))
            .collect();
        vertical_order.sort_by(|&a, &b| hotspots[a].center().1.total_cmp(&hotspots[b].center().1));
        let back = vertical_order
            .iter()
            .position(|&index| labels[index].id == ButtonId::Back.into());
        let forward = vertical_order
            .iter()
            .position(|&index| labels[index].id == ButtonId::Forward.into());
        let navigation_pair = if let (Some(back), Some(forward)) = (back, forward) {
            let first = back.min(forward);
            let second = back.max(forward);
            if second > first + 1 {
                let navigation_button = vertical_order.remove(second);
                vertical_order.insert(first + 1, navigation_button);
            }
            Some((vertical_order[first], vertical_order[first + 1]))
        } else {
            None
        };
        let step = mouse_h / (vertical_order.len() as f32 + 1.);
        for (slot, index) in vertical_order.into_iter().enumerate() {
            labels[index].y = step * (slot as f32 + 1.);
        }
        if let Some((first, second)) = navigation_pair {
            let grouped_step = step.min(LABEL_H + NAVIGATION_GROUP_GAP);
            let adjustment = (step - grouped_step) / 2.;
            labels[first].y += adjustment;
            labels[second].y -= adjustment;
        }
    }

    labels
}

/// Label positions for the synthetic fallback silhouette.
pub fn default_labels(caps: Capabilities, distribution: LabelDistribution) -> Vec<Label> {
    labels_from_hotspots(
        &super::hotspots::default_hotspots(caps),
        MOUSE_MODEL_SIZE.1,
        distribution,
    )
}

/// Logitech's stable slot vocabulary → OpenLogi's visual control IDs. Intentionally
/// conservative; unknown names fall through so widening `MouseControlId` later
/// doesn't break old depots.
fn map_slot_name(name: &str) -> Option<MouseControlId> {
    match name {
        "SLOT_NAME_LEFT_BUTTON" => Some(MouseControlId::Button(ButtonId::LeftClick)),
        "SLOT_NAME_RIGHT_BUTTON" => Some(MouseControlId::Button(ButtonId::RightClick)),
        "SLOT_NAME_MIDDLE_BUTTON" => Some(MouseControlId::Button(ButtonId::MiddleClick)),
        // The main wheel's tilt. Logi names the two slots after the scroll they
        // produce in firmware; each is its own reprogrammable control
        // (`0x1b04` CIDs `0x005b` / `0x005d`), not part of the middle click.
        "SLOT_NAME_LEFT_SCROLL_BUTTON" | "SLOT_NAME_SCROLL_LEFT" => {
            Some(MouseControlId::Button(ButtonId::WheelTiltLeft))
        }
        "SLOT_NAME_RIGHT_SCROLL_BUTTON" | "SLOT_NAME_SCROLL_RIGHT" => {
            Some(MouseControlId::Button(ButtonId::WheelTiltRight))
        }
        "SLOT_NAME_BACK_BUTTON" => Some(MouseControlId::Button(ButtonId::Back)),
        "SLOT_NAME_FORWARD_BUTTON" => Some(MouseControlId::Button(ButtonId::Forward)),
        "SLOT_NAME_MODESHIFT_BUTTON" | "SLOT_NAME_DPI_BUTTON" => {
            Some(MouseControlId::Button(ButtonId::DpiToggle))
        }
        "SLOT_NAME_THUMBWHEEL" => Some(MouseControlId::ThumbwheelRotation),
        "SLOT_NAME_GESTURE_BUTTON" => Some(MouseControlId::Button(ButtonId::GestureButton)),
        // The MX Master 4 Haptic Sense Panel. Logi names the slot after its
        // Options+ default assignment (the radial Actions Ring menu), but the
        // marker is the panel itself.
        "ASSIGNMENT_NAME_SHOW_RADIAL_MENU" => Some(MouseControlId::Button(ButtonId::HapticPanel)),
        _ => None,
    }
}

/// G HUB's G-key numbering → OpenLogi's visual control IDs. A G-series mouse
/// numbers its first five keys after the HID button each sends by default
/// (the depot's `default_configurations.json` maps input N to mouse button
/// N): G1/G2 are the primary clicks, left out like everywhere else, G3 the
/// wheel, G4 the rear and G5 the front side button. G6 and up differ per
/// model (the G305's DPI button, the G903's right-hand side pair), so they
/// stay unmapped rather than guessed.
fn map_g_key(key: u8) -> Option<MouseControlId> {
    match key {
        3 => Some(MouseControlId::Button(ButtonId::MiddleClick)),
        4 => Some(MouseControlId::Button(ButtonId::Back)),
        5 => Some(MouseControlId::Button(ButtonId::Forward)),
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use std::path::PathBuf;

    use openlogi_assets::{Assignment, Direction, ImageEntry, Metadata, Origin, Point};
    use openlogi_core::device::DeviceKind;

    use super::*;
    use crate::features::mouse::hotspots::default_hotspots;

    fn mouse(thumbwheel: bool) -> Capabilities {
        Capabilities {
            thumbwheel,
            ..Capabilities::presumed_from_kind(DeviceKind::Mouse)
        }
    }

    fn assignment(slot_id: &str, slot_name: &str, x: f32, y: f32) -> Assignment {
        Assignment {
            slot_id: slot_id.to_owned(),
            slot_name: slot_name.to_owned(),
            marker: Point { x, y },
            label: Direction::default(),
        }
    }

    fn mouse_asset(image: ImageEntry, png: (u32, u32)) -> ResolvedAsset {
        ResolvedAsset {
            depot: "test".to_owned(),
            display_name: "Test".to_owned(),
            kind: Some(DeviceKind::Mouse),
            image_path: PathBuf::from("/tmp/test.png"),
            hero_image_path: None,
            glow: None,
            metadata: Metadata {
                images: vec![image],
            },
            png_width: png.0,
            png_height: png.1,
        }
    }

    /// The G305 depot: G HUB G-keys in pixels of a 1400×2514 canvas, over
    /// the black render that is one pixel narrower.
    fn g305_asset() -> ResolvedAsset {
        mouse_asset(
            ImageEntry {
                key: "device_image".to_owned(),
                origin: Origin {
                    width: 1400,
                    height: 2514,
                },
                assignments: vec![
                    assignment("g305_g1_m1", "", 320., 651.),
                    assignment("g305_g2_m1", "", 1090., 651.),
                    assignment("g305_g3_m1", "", 709., 475.),
                    assignment("g305_g4_m1", "", 4., 1386.),
                    assignment("g305_g5_m1", "", 13., 976.),
                    assignment("g305_g6_m1", "", 707., 892.),
                ],
            },
            (1399, 2514),
        )
    }

    #[test]
    fn g_hub_depots_mark_the_wheel_and_side_buttons() {
        let asset = g305_asset();
        assert!(asset_has_button_labels(&asset));
        let hotspots = asset_hotspots_for_png(&asset, 1399., 2514.);
        let ids: Vec<MouseControlId> = hotspots.iter().map(|h| h.id).collect();
        assert_eq!(
            ids,
            [ButtonId::MiddleClick, ButtonId::Back, ButtonId::Forward].map(MouseControlId::from),
            "G1/G2 are the primary clicks and G6 differs per model"
        );
        let (back_x, back_y) = hotspots[1].center();
        assert!((back_x - 4. / 1400. * 1399.).abs() < 0.01);
        assert!((back_y - 1386.).abs() < 0.01);
        // The rear side button sits below the front one.
        assert!(hotspots[1].center().1 > hotspots[2].center().1);
    }

    #[test]
    fn options_plus_markers_translate_from_the_silhouette_bbox() {
        let asset = mouse_asset(
            ImageEntry {
                key: "device_buttons_image".to_owned(),
                origin: Origin {
                    width: 800,
                    height: 1000,
                },
                assignments: vec![
                    assignment("x_c82", "SLOT_NAME_MIDDLE_BUTTON", 50., 20.),
                    assignment("x_c83", "SLOT_NAME_BACK_BUTTON", 0., 60.),
                ],
            },
            (1000, 1000),
        );
        let hotspots = asset_hotspots_for_png(&asset, 500., 500.);
        // The 800 px bbox is centred in the 1000 px render: 10% padding.
        for (hotspot, (x, y)) in hotspots.iter().zip([(250., 100.), (50., 300.)]) {
            let (cx, cy) = hotspot.center();
            assert!(
                (cx - x).abs() < 0.01 && (cy - y).abs() < 0.01,
                "{:?} centred at ({cx}, {cy}), expected ({x}, {y})",
                hotspot.id
            );
        }
    }

    #[test]
    fn default_labels_include_capability_gated_thumbwheel() {
        assert!(
            !default_labels(mouse(false), LabelDistribution::LeftOnly)
                .iter()
                .any(|label| label.id == MouseControlId::ThumbwheelRotation)
        );
        assert_eq!(
            default_labels(mouse(true), LabelDistribution::LeftOnly)
                .iter()
                .filter(|label| label.id == MouseControlId::ThumbwheelRotation)
                .count(),
            1
        );
    }

    #[test]
    fn thumbwheel_metadata_maps_to_one_rotation_control() {
        assert_eq!(
            map_slot_name("SLOT_NAME_THUMBWHEEL"),
            Some(MouseControlId::ThumbwheelRotation)
        );
    }

    #[test]
    fn dpi_slot_names_map_to_dpi_toggle_button() {
        assert_eq!(
            map_slot_name("SLOT_NAME_MODESHIFT_BUTTON"),
            Some(MouseControlId::Button(ButtonId::DpiToggle))
        );
        assert_eq!(
            map_slot_name("SLOT_NAME_DPI_BUTTON"),
            Some(MouseControlId::Button(ButtonId::DpiToggle))
        );
    }

    #[test]
    fn wheel_tilt_slot_names_map_to_their_own_controls() {
        // MX Anywhere uses the longer names; MX Ergo uses the shorter aliases.
        for name in ["SLOT_NAME_LEFT_SCROLL_BUTTON", "SLOT_NAME_SCROLL_LEFT"] {
            assert_eq!(
                map_slot_name(name),
                Some(MouseControlId::Button(ButtonId::WheelTiltLeft))
            );
        }
        for name in ["SLOT_NAME_RIGHT_SCROLL_BUTTON", "SLOT_NAME_SCROLL_RIGHT"] {
            assert_eq!(
                map_slot_name(name),
                Some(MouseControlId::Button(ButtonId::WheelTiltRight))
            );
        }
    }

    #[test]
    fn labels_track_hotspots_and_avoid_crossing() {
        let hotspots = default_hotspots(mouse(true));
        let labels =
            labels_from_hotspots(&hotspots, MOUSE_MODEL_SIZE.1, LabelDistribution::LeftOnly);
        assert_eq!(labels.len(), hotspots.len());

        let mut ys: Vec<f32> = labels.iter().map(|l| l.y).collect();
        ys.sort_by(f32::total_cmp);
        ys.dedup();
        assert_eq!(ys.len(), labels.len(), "each label gets a distinct slot");
    }

    #[test]
    fn navigation_labels_stay_together_when_haptic_marker_sits_between() {
        let hotspots = [
            Hotspot {
                id: ButtonId::Forward.into(),
                x: 0.,
                y: 100.,
                w: 10.,
                h: 10.,
            },
            Hotspot {
                id: ButtonId::HapticPanel.into(),
                x: 0.,
                y: 200.,
                w: 10.,
                h: 10.,
            },
            Hotspot {
                id: ButtonId::Back.into(),
                x: 0.,
                y: 300.,
                w: 10.,
                h: 10.,
            },
        ];

        let mut labels =
            labels_from_hotspots(&hotspots, MOUSE_MODEL_SIZE.1, LabelDistribution::LeftOnly);
        labels.sort_by(|a, b| a.y.total_cmp(&b.y));

        assert_eq!(
            labels.iter().map(|label| label.id).collect::<Vec<_>>(),
            [
                MouseControlId::Button(ButtonId::Forward),
                MouseControlId::Button(ButtonId::Back),
                MouseControlId::Button(ButtonId::HapticPanel),
            ]
        );
        let navigation_gap = labels[1].y - labels[0].y;
        let haptic_gap = labels[2].y - labels[1].y;
        assert!(navigation_gap < haptic_gap);
        assert!(navigation_gap >= LABEL_H);
    }

    #[test]
    fn a_two_sided_layout_uses_both_sides() {
        let hotspots = default_hotspots(mouse(true));
        let labels =
            labels_from_hotspots(&hotspots, MOUSE_MODEL_SIZE.1, LabelDistribution::BothSides);

        assert!(labels.iter().any(|label| label.side == Side::Left));
        assert!(labels.iter().any(|label| label.side == Side::Right));
    }
}
