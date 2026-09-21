//! Gamepad state types mirroring the HIDMaestro SDK's `HMGamepadState`.

use std::collections::BTreeMap;

use bitflags::bitflags;
use serde::{Deserialize, Serialize};

bitflags! {
    /// Standard gamepad button bitmask. Profile-specific renames (Cross/A,
    /// Circle/B, etc.) are handled by HIDMaestro based on the active profile.
    /// Bit values match the SDK's `HMButton` enum. Serializes as the raw
    /// `u32` mask the bridge expects.
    #[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default)]
    pub struct Buttons: u32 {
        const NONE          = 0;
        const A             = 1 << 0;
        const B             = 1 << 1;
        const X             = 1 << 2;
        const Y             = 1 << 3;
        const LEFT_BUMPER   = 1 << 4;
        const RIGHT_BUMPER  = 1 << 5;
        const BACK          = 1 << 6;   // Select / Share / View
        const START         = 1 << 7;   // Options / Menu
        const LEFT_STICK    = 1 << 8;   // L3
        const RIGHT_STICK   = 1 << 9;   // R3
        const GUIDE         = 1 << 10;  // Xbox / PS / Home
        const TOUCHPAD      = 1 << 11;  // PS touchpad click
        const SHARE         = 1 << 12;  // Xbox Series Share
        const RIGHT_PADDLE  = 1 << 13;
        const LEFT_PADDLE   = 1 << 14;
        const MISC1         = 1 << 15;  // Switch 2 C / DualSense mic mute
        const RIGHT_PADDLE2 = 1 << 16;
        const LEFT_PADDLE2  = 1 << 17;

        // PlayStation aliases.
        const CROSS    = Self::A.bits();
        const CIRCLE   = Self::B.bits();
        const SQUARE   = Self::X.bits();
        const TRIANGLE = Self::Y.bits();
    }
}

impl Buttons {
    /// Parse a button name (case-insensitive): `"a"`, `"cross"`, `"left_bumper"`, ...
    pub fn by_name(name: &str) -> Option<Self> {
        let n = name.to_ascii_lowercase().replace(['-', ' '], "_");
        Some(match n.as_str() {
            "a" | "cross" => Self::A,
            "b" | "circle" => Self::B,
            "x" | "square" => Self::X,
            "y" | "triangle" => Self::Y,
            "left_bumper" | "lb" | "l1" => Self::LEFT_BUMPER,
            "right_bumper" | "rb" | "r1" => Self::RIGHT_BUMPER,
            "back" | "select" | "view" | "share_x360" => Self::BACK,
            "start" | "options" | "menu" => Self::START,
            "left_stick" | "l3" => Self::LEFT_STICK,
            "right_stick" | "r3" => Self::RIGHT_STICK,
            "guide" | "home" | "ps" | "xbox" => Self::GUIDE,
            "touchpad" => Self::TOUCHPAD,
            "share" => Self::SHARE,
            "right_paddle" => Self::RIGHT_PADDLE,
            "left_paddle" => Self::LEFT_PADDLE,
            "misc1" | "mic" | "c" => Self::MISC1,
            "right_paddle2" => Self::RIGHT_PADDLE2,
            "left_paddle2" => Self::LEFT_PADDLE2,
            _ => return None,
        })
    }

    /// Names of every set bit, canonical form.
    pub fn names(self) -> Vec<&'static str> {
        const ALL: [(Buttons, &str); 18] = [
            (Buttons::A, "a"),
            (Buttons::B, "b"),
            (Buttons::X, "x"),
            (Buttons::Y, "y"),
            (Buttons::LEFT_BUMPER, "left_bumper"),
            (Buttons::RIGHT_BUMPER, "right_bumper"),
            (Buttons::BACK, "back"),
            (Buttons::START, "start"),
            (Buttons::LEFT_STICK, "left_stick"),
            (Buttons::RIGHT_STICK, "right_stick"),
            (Buttons::GUIDE, "guide"),
            (Buttons::TOUCHPAD, "touchpad"),
            (Buttons::SHARE, "share"),
            (Buttons::RIGHT_PADDLE, "right_paddle"),
            (Buttons::LEFT_PADDLE, "left_paddle"),
            (Buttons::MISC1, "misc1"),
            (Buttons::RIGHT_PADDLE2, "right_paddle2"),
            (Buttons::LEFT_PADDLE2, "left_paddle2"),
        ];
        ALL.iter()
            .filter(|(b, _)| self.contains(*b))
            .map(|(_, n)| *n)
            .collect()
    }
}

impl Serialize for Buttons {
    fn serialize<S: serde::Serializer>(&self, s: S) -> std::result::Result<S::Ok, S::Error> {
        s.serialize_u32(self.bits())
    }
}

impl<'de> Deserialize<'de> for Buttons {
    fn deserialize<D: serde::Deserializer<'de>>(d: D) -> std::result::Result<Self, D::Error> {
        Ok(Self::from_bits_truncate(u32::deserialize(d)?))
    }
}

/// D-pad / hat-switch direction. Values match the SDK's `HMHat` enum.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default)]
pub enum Hat {
    #[default]
    None = 0,
    North = 1,
    NorthEast = 2,
    East = 3,
    SouthEast = 4,
    South = 5,
    SouthWest = 6,
    West = 7,
    NorthWest = 8,
}

impl Hat {
    /// Parse a hat direction name: `"up"`, `"north_east"`, `"down"`, ...
    pub fn by_name(name: &str) -> Option<Self> {
        let n = name.to_ascii_lowercase().replace(['-', ' '], "_");
        Some(match n.as_str() {
            "none" | "neutral" | "released" | "center" | "centre" => Self::None,
            "north" | "up" | "n" => Self::North,
            "north_east" | "up_right" | "ne" => Self::NorthEast,
            "east" | "right" | "e" => Self::East,
            "south_east" | "down_right" | "se" => Self::SouthEast,
            "south" | "down" | "s" => Self::South,
            "south_west" | "down_left" | "sw" => Self::SouthWest,
            "west" | "left" | "w" => Self::West,
            "north_west" | "up_left" | "nw" => Self::NorthWest,
            _ => return None,
        })
    }

    pub fn as_u8(self) -> u8 {
        self as u8
    }
}

/// Wire form matches the SDK's `HMHat` (byte), not the variant name.
impl Serialize for Hat {
    fn serialize<S: serde::Serializer>(&self, s: S) -> std::result::Result<S::Ok, S::Error> {
        s.serialize_u8(self.as_u8())
    }
}

impl<'de> Deserialize<'de> for Hat {
    fn deserialize<D: serde::Deserializer<'de>>(d: D) -> std::result::Result<Self, D::Error> {
        let v = u8::deserialize(d)?;
        Ok(match v {
            1 => Self::North,
            2 => Self::NorthEast,
            3 => Self::East,
            4 => Self::SouthEast,
            5 => Self::South,
            6 => Self::SouthWest,
            7 => Self::West,
            8 => Self::NorthWest,
            _ => Self::None,
        })
    }
}

/// HID-usage-addressable analog axis. Encodes `(UsagePage << 8) | Usage`,
/// matching the SDK's `HMAxis` enum. Values written to [`GamepadState::axes`]
/// normalize to `0.0..=1.0`; `0.5` is center on signed axes (sticks) and `0.0`
/// is released on unsigned axes (triggers, throttles, pedals).
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(transparent)]
pub struct Axis(pub u16);

impl Axis {
    pub const X: Self = Self(0x0130);
    pub const Y: Self = Self(0x0131);
    pub const Z: Self = Self(0x0132);
    pub const RX: Self = Self(0x0133);
    pub const RY: Self = Self(0x0134);
    pub const RZ: Self = Self(0x0135);
    pub const SLIDER: Self = Self(0x0136);
    pub const DIAL: Self = Self(0x0137);
    pub const WHEEL: Self = Self(0x0138);
    pub const VX: Self = Self(0x0140);
    pub const VY: Self = Self(0x0141);

    pub const RUDDER: Self = Self(0x02BA);
    pub const THROTTLE: Self = Self(0x02BB);
    pub const ACCELERATOR: Self = Self(0x02C4);
    pub const BRAKE: Self = Self(0x02C5);
    pub const CLUTCH: Self = Self(0x02C6);
    pub const STEERING: Self = Self(0x02C8);

    /// Parse an axis name: `"x"`, `"rx"`, `"left_trigger"`, `"throttle"`,
    /// or a raw usage value like `"0x0130"` / `"308"`.
    pub fn by_name(name: &str) -> Option<Self> {
        let n = name.to_ascii_lowercase().replace(['-', ' '], "_");
        if let Some(hex) = n.strip_prefix("0x") {
            return u16::from_str_radix(hex, 16).ok().map(Self);
        }
        Some(match n.as_str() {
            "x" | "left_x" | "left_stick_x" => Self::X,
            "y" | "left_y" | "left_stick_y" => Self::Y,
            "z" | "left_trigger" | "lt" => Self::Z,
            "rx" | "right_x" | "right_stick_x" => Self::RX,
            "ry" | "right_y" | "right_stick_y" => Self::RY,
            "rz" | "right_trigger" | "rt" => Self::RZ,
            "slider" => Self::SLIDER,
            "dial" => Self::DIAL,
            "wheel" => Self::WHEEL,
            "steering" => Self::STEERING,
            "rudder" => Self::RUDDER,
            "throttle" => Self::THROTTLE,
            "accelerator" | "gas" => Self::ACCELERATOR,
            "brake" => Self::BRAKE,
            "clutch" => Self::CLUTCH,
            _ => n.parse::<u16>().ok().map(Self)?,
        })
    }
}

/// The canonical 6-slot gamepad layout. When set on a [`GamepadState`], the
/// bridge resolves the real HID axes through the profile's stick/trigger
/// table (handles profiles where the left stick is `Rx`+`Ry`, etc.).
/// Every value is `0.0..=1.0`; omit a field to leave it at its default
/// (0.5 centered for sticks, 0.0 released for triggers).
#[derive(Debug, Clone, Copy, PartialEq, Default, Serialize, Deserialize)]
pub struct StandardAxes {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub left_stick_x: Option<f32>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub left_stick_y: Option<f32>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub right_stick_x: Option<f32>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub right_stick_y: Option<f32>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub left_trigger: Option<f32>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub right_trigger: Option<f32>,
}

/// Abstract gamepad state pushed to a virtual controller. The bridge /
/// HIDMaestro SDK translates this into the profile's native HID report.
#[derive(Debug, Clone, PartialEq, Default, Serialize, Deserialize)]
pub struct GamepadState {
    /// Pressed buttons bitmask.
    #[serde(default)]
    pub buttons: Buttons,

    /// Octant d-pad direction.
    #[serde(default)]
    pub hat: Hat,

    /// Continuous hat angle in degrees (0 = north, clockwise). Takes
    /// priority over `hat` when set. Use for high-resolution hats.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub hat_degrees: Option<f32>,

    /// Any descriptor-declared analog axis, keyed by HID usage. Merged on
    /// top of `standard_axes` when both are present.
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub axes: BTreeMap<Axis, f32>,

    /// Canonical 6-slot axes, resolved against the profile's layout by the
    /// bridge. Prefer this over `axes` unless you need an unusual axis.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub standard_axes: Option<StandardAxes>,

    /// Battery capacity 0..=10 (Sony firmware convention).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub battery_level: Option<u8>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub battery_charging: Option<bool>,

    /// Accelerometer in g, SDL sensor frame (+right / +up / +toward player).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub accel_g: Option<[f32; 3]>,
    /// Gyroscope in deg/s, right-hand rule around the accel axes.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub gyro_dps: Option<[f32; 3]>,
}

impl GamepadState {
    /// Neutral state: all sticks centered, triggers released, nothing held.
    pub fn neutral() -> Self {
        Self::default()
    }
}

/// Serializable profile metadata returned by `list_profiles` / `get_profile`.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Profile {
    pub id: String,
    #[serde(default)]
    pub name: String,
    #[serde(default)]
    pub vendor: String,
    #[serde(default)]
    pub vendor_id: u16,
    #[serde(default)]
    pub product_id: u16,
    #[serde(default)]
    pub display_name: String,
    #[serde(default)]
    pub connection: String,
    #[serde(default)]
    pub backend: String,
    #[serde(default)]
    pub button_count: usize,
    #[serde(default)]
    pub axis_count: usize,
    #[serde(default)]
    pub has_hat: bool,
    #[serde(default)]
    pub deployable: bool,
    #[serde(default)]
    pub requires_usbip_backend: bool,
}

/// A live controller as reported by `list_controllers`.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ControllerInfo {
    pub key: String,
    pub profile_id: String,
}

/// An output report (rumble, LEDs, force feedback) forwarded by the bridge.
/// Each packet produces a raw event; when the SDK can decode it, a second
/// event includes decoded fields and the CRC result.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct OutputEvent {
    pub controller: String,
    pub report_id: u8,
    #[serde(default)]
    pub fields: serde_json::Map<String, serde_json::Value>,
    #[serde(default)]
    pub raw: Vec<u8>,
    /// False for unverified raw events or a failed decoded CRC check.
    #[serde(default)]
    pub crc_valid: bool,
}
