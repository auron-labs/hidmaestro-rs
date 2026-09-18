use crate::*;

#[test]
fn button_names_round_trip() {
    for name in [
        "a",
        "b",
        "x",
        "y",
        "left_bumper",
        "right_bumper",
        "back",
        "start",
        "left_stick",
        "right_stick",
        "guide",
        "touchpad",
        "share",
        "misc1",
        "left_paddle",
        "right_paddle",
        "left_paddle2",
        "right_paddle2",
    ] {
        let b = Buttons::by_name(name).unwrap_or_else(|| panic!("{name}"));
        assert!(b.names().contains(&name) || name == "misc1", "{name}");
    }
    assert_eq!(Buttons::by_name("cross"), Some(Buttons::A));
    assert_eq!(Buttons::by_name("L1"), Some(Buttons::LEFT_BUMPER));
    assert_eq!(Buttons::by_name("bogus"), None);
}

#[test]
fn hat_names() {
    assert_eq!(Hat::by_name("up"), Some(Hat::North));
    assert_eq!(Hat::by_name("down-left"), Some(Hat::SouthWest));
    assert_eq!(Hat::by_name("neutral"), Some(Hat::None));
    assert_eq!(Hat::by_name("nope"), None);
}

#[test]
fn axis_names() {
    assert_eq!(Axis::by_name("x"), Some(Axis::X));
    assert_eq!(Axis::by_name("left_trigger"), Some(Axis::Z));
    assert_eq!(Axis::by_name("0x0133"), Some(Axis::RX));
    assert_eq!(Axis::by_name("304"), Some(Axis::X));
    assert_eq!(Axis::by_name("bogus"), None);
}

#[test]
fn state_serializes_like_bridge_expects() {
    let mut s = GamepadState::neutral();
    s.buttons = Buttons::A | Buttons::START;
    s.hat = Hat::North;
    s.axes.insert(Axis::X, 1.0);
    let v = serde_json::to_value(&s).unwrap();
    assert_eq!(v["buttons"], 0x81);
    assert_eq!(v["hat"], 1);
    assert_eq!(v["axes"]["304"], 1.0);
}

#[test]
fn hat_serializes_as_byte() {
    assert_eq!(serde_json::to_value(Hat::SouthEast).unwrap(), 4);
    assert_eq!(
        serde_json::from_value::<Hat>(serde_json::json!(7)).unwrap(),
        Hat::West
    );
    assert_eq!(
        serde_json::from_value::<Hat>(serde_json::json!(99)).unwrap(),
        Hat::None
    );
}
