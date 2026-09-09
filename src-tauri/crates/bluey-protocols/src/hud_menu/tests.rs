use super::*;
use serde_json::{json, Value};

fn fixture() -> HudMenuRequest {
    serde_json::from_str(include_str!(concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/../../../tests/fixtures/native-hud-menu.json"
    )))
    .unwrap()
}

#[test]
fn ts_payload_round_trips_exactly_with_nullable_checks_and_icons() {
    let request = fixture();
    request.validate("main").unwrap();
    let encoded = serde_json::to_value(&request).unwrap();
    let expected: Value = serde_json::from_str(include_str!(concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/../../../tests/fixtures/native-hud-menu.json"
    )))
    .unwrap();
    assert_eq!(encoded, expected);
    assert_eq!(
        serde_json::to_value(request.selected_id(Some(1))).unwrap(),
        json!("hud-1")
    );
    assert_eq!(
        serde_json::to_value(request.selected_id(None)).unwrap(),
        Value::Null
    );
}

#[test]
fn only_main_can_open_a_hud_menu() {
    for label in ["settings", "onboarding", "", "MAIN", "other"] {
        assert!(fixture().validate(label).is_err());
    }
}

#[test]
fn rejects_empty_and_oversized_menus_before_native_creation() {
    let mut request = fixture();
    request.items.clear();
    assert!(request.validate("main").is_err());
    request.items = (0..MAX_ITEMS)
        .map(|i| HudMenuItem::Separator {
            id: format!("hud-{i}"),
        })
        .collect();
    request.validate("main").unwrap();
    request
        .items
        .push(HudMenuItem::Separator { id: "extra".into() });
    assert!(request.validate("main").is_err());
}

#[test]
fn rejects_duplicate_unbounded_and_nonopaque_ids() {
    let mut request = fixture();
    for id in [
        "".into(),
        "../path".into(),
        "https://example.com".into(),
        "💙".into(),
        "a".repeat(MAX_ID_BYTES + 1),
        "hud-1".into(),
    ] {
        request.items[0] = HudMenuItem::Separator { id };
        assert!(request.validate("main").is_err());
    }
    request.items[0] = HudMenuItem::Separator {
        id: "A_9-".repeat(MAX_ID_BYTES / 4),
    };
    request.validate("main").unwrap();
}

#[test]
fn label_limits_count_unicode_not_utf8_bytes_and_reject_controls() {
    let mut request = fixture();
    for label in [
        "".into(),
        " \t ".into(),
        "line\nline".into(),
        "null\0byte".into(),
        "line\u{2028}line".into(),
        "x".repeat(MAX_LABEL_CHARS + 1),
    ] {
        request.items[0] = HudMenuItem::Label {
            id: "hud-0".into(),
            label,
        };
        assert!(request.validate("main").is_err());
    }
    request.items[0] = HudMenuItem::Label {
        id: "hud-0".into(),
        label: "𠮷".repeat(MAX_LABEL_CHARS),
    };
    request.validate("main").unwrap();
}

#[test]
fn rejects_nonfinite_negative_and_excessive_client_coordinates() {
    for value in [
        f64::NAN,
        f64::INFINITY,
        f64::NEG_INFINITY,
        -1.0,
        MAX_CLIENT_POSITION + 0.1,
    ] {
        for x_axis in [true, false] {
            let mut request = fixture();
            if x_axis {
                request.position.x = value;
            } else {
                request.position.y = value;
            }
            assert!(request.validate("main").is_err());
        }
    }
    for value in [0.0, MAX_CLIENT_POSITION] {
        let mut request = fixture();
        request.position = HudMenuPosition { x: value, y: value };
        request.validate("main").unwrap();
    }
}

#[test]
fn anchors_are_bounded_by_the_actual_content_view() {
    let request = fixture();
    request.validate_view_size(560.0, 108.0).unwrap();
    request.validate_view_size(470.5, 88.25).unwrap();
    for (width, height) in [
        (470.0, 108.0),
        (560.0, 88.0),
        (0.0, 108.0),
        (560.0, -1.0),
        (f64::NAN, 108.0),
        (560.0, f64::INFINITY),
    ] {
        assert!(request.validate_view_size(width, height).is_err());
    }
}

#[test]
fn only_enabled_items_can_resolve_a_selection() {
    let request = fixture();
    assert_eq!(request.selected_id(Some(1)), Some("hud-1".into()));
    for index in [None, Some(0), Some(2), Some(3), Some(4), Some(usize::MAX)] {
        assert_eq!(request.selected_id(index), None);
    }
}

#[test]
fn rejects_actions_urls_shell_unknown_fields_and_icon_names_on_the_wire() {
    let base = serde_json::to_value(fixture()).unwrap();
    for key in ["action", "url", "shell", "window", "callback", "submenu"] {
        let mut wire = base.clone();
        wire["items"][1][key] = json!("untrusted");
        assert!(serde_json::from_value::<HudMenuRequest>(wire).is_err());
    }
    let mut wire = base.clone();
    wire["position"]["screen"] = json!(true);
    assert!(serde_json::from_value::<HudMenuRequest>(wire).is_err());
    let mut wire = base.clone();
    wire["windowLabel"] = json!("settings");
    assert!(serde_json::from_value::<HudMenuRequest>(wire).is_err());
    let mut wire = base;
    wire["items"][1]["icon"] = json!("arbitrary-file-or-symbol");
    assert!(serde_json::from_value::<HudMenuRequest>(wire).is_err());
}

#[test]
fn popup_lease_suppresses_reentrancy_and_releases_after_success_or_cancel() {
    let gate = PopupGate::new();
    for _ in 0..1000 {
        let permit = gate.try_acquire().unwrap();
        assert!(gate.try_acquire().is_none());
        drop(permit);
    }
    assert!(gate.try_acquire().is_some());
}

#[test]
fn scheduling_or_partial_creation_failure_releases_the_lease() {
    let gate = PopupGate::new();
    let permit = gate.try_acquire().unwrap();
    let unqueued_task = move || drop(permit);
    assert!(gate.try_acquire().is_none());
    drop(unqueued_task); // failed main-thread dispatch drops the closure + captured permit
    assert!(gate.try_acquire().is_some());
    let result: Result<(), ()> = {
        let _permit = gate.try_acquire().unwrap();
        Err(()) // view/creation failure, before tracking
    };
    assert!(result.is_err());
    assert!(gate.try_acquire().is_some());
}

#[test]
fn dropped_receiver_does_not_unlock_a_tracking_task() {
    let gate = PopupGate::new();
    std::thread::scope(|scope| {
        let permit = gate.try_acquire().unwrap();
        let (complete, closed) = std::sync::mpsc::channel::<()>();
        let (send, receive) = std::sync::mpsc::channel::<()>();
        let native = scope.spawn(move || {
            closed.recv().unwrap(); // mock synchronous tracking boundary
            drop(permit); // native resources would already have dropped here
            let _ = send.send(());
        });
        drop(receive); // caller/navigation disappears; native task still owns the lease
        assert!(gate.try_acquire().is_none());
        complete.send(()).unwrap();
        native.join().unwrap();
    });
    assert!(gate.try_acquire().is_some());
}

#[test]
fn unwind_releases_a_lease_without_poisoning_future_popups() {
    let gate = PopupGate::new();
    let result = std::panic::catch_unwind(|| {
        let _permit = gate.try_acquire().unwrap();
        panic!("simulated creation unwind");
    });
    assert!(result.is_err());
    assert!(gate.try_acquire().is_some());
}
