use super::*;
use pretty_assertions::assert_eq;

fn err() -> BlueyError {
    BlueyError::internal("boom")
}

/// Drive a fresh machine into `state` (audio off).
fn machine_in(state: AppState) -> AppStateMachine {
    let mut m = AppStateMachine::new("general");
    let path: &[AppEvent] = match state {
        AppState::Booting => &[],
        AppState::AuthRequired => &[AppEvent::BootCompleted {
            authenticated: false,
        }],
        AppState::Ready => &[AppEvent::BootCompleted {
            authenticated: true,
        }],
        AppState::Listening => &[
            AppEvent::BootCompleted {
                authenticated: true,
            },
            AppEvent::AudioStarted,
        ],
        AppState::Capturing => &[
            AppEvent::BootCompleted {
                authenticated: true,
            },
            AppEvent::CaptureStarted,
        ],
        AppState::Analyzing => &[
            AppEvent::BootCompleted {
                authenticated: true,
            },
            AppEvent::CaptureStarted,
            AppEvent::CaptureFinished,
        ],
        AppState::Thinking => &[
            AppEvent::BootCompleted {
                authenticated: true,
            },
            AppEvent::ThinkingStarted,
        ],
        AppState::ResponseReady => &[
            AppEvent::BootCompleted {
                authenticated: true,
            },
            AppEvent::ThinkingStarted,
            AppEvent::ResponseReady,
        ],
        AppState::Error => &[
            AppEvent::BootCompleted {
                authenticated: true,
            },
            AppEvent::Failed { error: err() },
        ],
        AppState::Paused => &[
            AppEvent::BootCompleted {
                authenticated: true,
            },
            AppEvent::Paused,
        ],
    };
    for e in path {
        m.transition(e.clone()).expect("setup transition");
    }
    assert_eq!(m.state(), state, "setup reached the requested state");
    m
}

const ALL_STATES: [AppState; 10] = [
    AppState::Booting,
    AppState::AuthRequired,
    AppState::Ready,
    AppState::Listening,
    AppState::Capturing,
    AppState::Analyzing,
    AppState::Thinking,
    AppState::ResponseReady,
    AppState::Error,
    AppState::Paused,
];

fn sample_events() -> Vec<AppEvent> {
    vec![
        AppEvent::BootCompleted {
            authenticated: true,
        },
        AppEvent::BootCompleted {
            authenticated: false,
        },
        AppEvent::Authenticated,
        AppEvent::SignedOut,
        AppEvent::AudioStarted,
        AppEvent::AudioStopped,
        AppEvent::CaptureStarted,
        AppEvent::CaptureFinished,
        AppEvent::AnalysisStarted,
        AppEvent::ThinkingStarted,
        AppEvent::ResponseReady,
        AppEvent::ResponseDismissed,
        AppEvent::Failed { error: err() },
        AppEvent::Recovered,
        AppEvent::Paused,
        AppEvent::Resumed,
        AppEvent::SessionChanged {
            session_id: Some("ses_1".into()),
        },
        AppEvent::ModeChanged {
            mode_id: "interview".into(),
        },
    ]
}

/// Expected target state for (state, event) with audio off; `None` = reject.
fn expected(from: AppState, event: &AppEvent) -> Option<AppState> {
    use AppEvent as E;
    use AppState as S;
    let non_boot = !matches!(from, S::Booting | S::AuthRequired);
    match event {
        E::SessionChanged { .. } | E::ModeChanged { .. } => Some(from),
        E::BootCompleted { authenticated } => (from == S::Booting).then_some(if *authenticated {
            S::Ready
        } else {
            S::AuthRequired
        }),
        E::Authenticated => (from == S::AuthRequired).then_some(S::Ready),
        E::SignedOut => (from != S::Booting).then_some(S::AuthRequired),
        E::AudioStarted => {
            audio_toggle_allowed(from).then_some(if from == S::Ready { S::Listening } else { from })
        }
        E::AudioStopped => {
            audio_toggle_allowed(from).then_some(if from == S::Listening { S::Ready } else { from })
        }
        E::CaptureStarted => (from.is_idle() || from == S::ResponseReady).then_some(S::Capturing),
        E::CaptureFinished | E::AnalysisStarted => (from == S::Capturing).then_some(S::Analyzing),
        E::ThinkingStarted => (from.is_idle() || from == S::Analyzing || from == S::ResponseReady)
            .then_some(S::Thinking),
        E::ResponseReady => (from == S::Thinking).then_some(S::ResponseReady),
        // Audio is off in the matrix machines, so idle == Ready.
        E::ResponseDismissed => matches!(
            from,
            S::Capturing | S::Analyzing | S::Thinking | S::ResponseReady
        )
        .then_some(S::Ready),
        E::Failed { .. } => non_boot.then_some(S::Error),
        E::Recovered => (from == S::Error).then_some(S::Ready),
        E::Paused => (non_boot && from != S::Paused).then_some(S::Paused),
        E::Resumed => (from == S::Paused).then_some(S::Ready),
    }
}

#[test]
fn full_transition_matrix() {
    for state in ALL_STATES {
        for event in sample_events() {
            let mut m = machine_in(state);
            let before = m.status().clone();
            let want = expected(state, &event);
            let got = m.transition(event.clone());
            match want {
                Some(target) => {
                    let status = got.unwrap_or_else(|e| {
                        panic!("{state:?} + {} should be accepted: {e}", event_name(&event))
                    });
                    assert_eq!(status.state, target, "{state:?} + {}", event_name(&event));
                }
                None => {
                    let e = got.expect_err("transition should be rejected");
                    assert_eq!(e.from, state);
                    assert_eq!(e.event, event_name(&event));
                    assert_eq!(m.status(), &before, "rejected event must not mutate status");
                }
            }
        }
    }
}

#[test]
fn audio_region_controls_idle_state() {
    // Cycle ends in Listening when audio is on.
    let mut m = machine_in(AppState::Listening);
    m.transition(AppEvent::CaptureStarted).expect("capture");
    m.transition(AppEvent::CaptureFinished).expect("analyze");
    m.transition(AppEvent::ThinkingStarted).expect("think");
    m.transition(AppEvent::ResponseReady).expect("ready");
    let s = m.transition(AppEvent::ResponseDismissed).expect("dismiss");
    assert_eq!(s.state, AppState::Listening);
    assert!(s.audio_active);

    // Stopping audio mid-pipeline changes where the cycle lands.
    let mut m = machine_in(AppState::Listening);
    m.transition(AppEvent::CaptureStarted).expect("capture");
    m.transition(AppEvent::AudioStopped)
        .expect("stop audio while capturing");
    assert_eq!(m.state(), AppState::Capturing);
    m.transition(AppEvent::CaptureFinished).expect("analyze");
    m.transition(AppEvent::ThinkingStarted).expect("think");
    m.transition(AppEvent::ResponseReady).expect("ready");
    let s = m.transition(AppEvent::ResponseDismissed).expect("dismiss");
    assert_eq!(s.state, AppState::Ready);
    assert!(!s.audio_active);
}

#[test]
fn failure_remembers_and_recovers_to_idle() {
    let mut m = machine_in(AppState::Listening);
    m.transition(AppEvent::ThinkingStarted).expect("think");
    let s = m
        .transition(AppEvent::Failed { error: err() })
        .expect("fail");
    assert_eq!(s.state, AppState::Error);
    assert_eq!(s.resume_state, Some(AppState::Listening));
    assert!(s.error.is_some());
    let s = m.transition(AppEvent::Recovered).expect("recover");
    assert_eq!(s.state, AppState::Listening);
    assert_eq!(s.resume_state, None);
    assert!(s.error.is_none());
}

#[test]
fn pause_keeps_audio_flag_and_resumes_to_listening() {
    let mut m = machine_in(AppState::Listening);
    let s = m.transition(AppEvent::Paused).expect("pause");
    assert_eq!(s.state, AppState::Paused);
    assert!(s.audio_active, "pausing must keep audio_active as-is");
    assert_eq!(s.resume_state, Some(AppState::Listening));
    let s = m.transition(AppEvent::Resumed).expect("resume");
    assert_eq!(s.state, AppState::Listening);
    assert_eq!(s.resume_state, None);
}

#[test]
fn signed_out_clears_session_and_audio() {
    let mut m = machine_in(AppState::Listening);
    m.transition(AppEvent::SessionChanged {
        session_id: Some("ses_9".into()),
    })
    .expect("session");
    let s = m.transition(AppEvent::SignedOut).expect("sign out");
    assert_eq!(s.state, AppState::AuthRequired);
    assert_eq!(s.session_id, None);
    assert!(!s.audio_active);
}

#[test]
fn session_and_mode_changes_do_not_change_state() {
    let mut m = machine_in(AppState::Thinking);
    let s = m
        .transition(AppEvent::SessionChanged {
            session_id: Some("ses_2".into()),
        })
        .expect("session change");
    assert_eq!(s.state, AppState::Thinking);
    assert_eq!(s.session_id.as_deref(), Some("ses_2"));
    let s = m
        .transition(AppEvent::ModeChanged {
            mode_id: "sales".into(),
        })
        .expect("mode");
    assert_eq!(s.state, AppState::Thinking);
    assert_eq!(s.mode_id, "sales");
}

#[test]
fn transition_error_converts_to_bluey_error() {
    let mut m = AppStateMachine::new("general");
    let e = m
        .transition(AppEvent::CaptureStarted)
        .expect_err("must reject");
    let be: BlueyError = e.into();
    assert_eq!(be.kind, BlueyErrorKind::Internal);
    assert_eq!(be.code, "state.invalid_transition");
    assert!(be.message.contains("capture_started"));
    assert!(be.message.contains("booting"));
    let details = be.details.expect("details");
    assert_eq!(details["from"], "booting");
    assert_eq!(details["event"], "capture_started");
}

/// Property-style: after any accepted transition the status is consistent.
#[test]
fn accepted_transitions_keep_status_consistent() {
    fn assert_consistent(s: &AppStatus) {
        if s.state == AppState::Listening {
            assert!(s.audio_active, "Listening ⇒ audio_active");
        }
        assert_eq!(
            s.error.is_some(),
            s.state == AppState::Error,
            "error is set exactly in the Error state"
        );
        assert_eq!(
            s.resume_state.is_some(),
            matches!(s.state, AppState::Error | AppState::Paused),
            "resume_state is set exactly in Error/Paused"
        );
        if let Some(r) = s.resume_state {
            assert!(
                matches!(r, AppState::Ready | AppState::Listening),
                "resume is idle"
            );
        }
        assert!(!s.updated_at.is_empty());
    }

    let events = sample_events();
    // Deterministic pseudo-random walk from every state.
    let mut seed: u64 = 0x5DEECE66D;
    for start in ALL_STATES {
        let mut m = machine_in(start);
        for _ in 0..300 {
            seed = seed
                .wrapping_mul(6364136223846793005)
                .wrapping_add(1442695040888963407);
            let event = events[(seed >> 33) as usize % events.len()].clone();
            if let Ok(status) = m.transition(event) {
                assert_consistent(&status);
            }
        }
    }
}
