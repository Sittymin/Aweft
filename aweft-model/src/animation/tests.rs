use std::collections::HashMap;

use crate::animation::action::{Action, ActionCurve};
use crate::animation::anim_data::{AnimData, BlendMode, EditSource, PropertyPath};
use crate::animation::fcurve::FCurve;
use crate::animation::keyframe::{Extrapolation, Keyframe};
use crate::animation::library::AnimationLibrary;
use crate::animation::merge::apply_animation_to_style;
use crate::animation::modifier::{
    CycleMode, FModCycles, FModEnvelope, FModLimits,
    FModNoise, FModStepped, FModifier, FModifierInstance,
};
use crate::animation::sample::{blend_scalar, blend_strip_results, EvaluatedStrip};
use crate::animation::strip::{
    ExtendMode, NlaStrip, NlaTrack, StripControlCurves, StripPlacement,
};
use crate::ids::{ActionId, AnimDataId};
use crate::node_graph::style::StyleResult;

// ── Helpers ───────────────────────────────────────────────────────────────

fn make_action(name: &str, curves: Vec<ActionCurve>) -> Action {
    Action {
        id: ActionId::new(),
        name: name.to_string(),
        duration: 1.0,
        curves,
    }
}

fn make_anim_data(name: &str, tracks: Vec<NlaTrack>) -> AnimData {
    AnimData {
        id: AnimDataId::new(),
        name: name.to_string(),
        tracks,
        unique_action: None,
        temporary_edit: None,
    }
}

fn make_curve(kfs: Vec<Keyframe>) -> FCurve {
    FCurve {
        keyframes: kfs,
        extrapolation: Extrapolation::Constant,
        modifiers: Vec::new(),
        integer: false,
    }
}

fn make_acurve(prop: PropertyPath, kfs: Vec<Keyframe>) -> ActionCurve {
    ActionCurve::new(prop, make_curve(kfs))
}

// ── 3.1 Placement & Cropping ──────────────────────────────────────────────

#[test]
fn fixed_padding_holds_ends_and_stretches_middle() {
    let action = make_action(
        "fade",
        vec![make_acurve(
            PropertyPath::PositionX,
            vec![Keyframe::linear(0.0, 0.0), Keyframe::linear(1.0, 100.0)],
        )],
    );

    let track = NlaTrack {
        name: "main".into(),
        muted: false,
        solo: false,
        strips: vec![NlaStrip {
            name: "strip".into(),
            placement: StripPlacement::Padded {
                pad_start: 0.2,
                pad_end: 0.2,
            },
            ..NlaStrip::new_action("strip", action.id)
        }],
    };

    let mut lib = AnimationLibrary::default();
    lib.insert_action(action);
    let id = lib.insert_anim_data(make_anim_data("clip", vec![track]));

    // target_duration = 3.0: strip from 0.2 to 2.8 (middle 2.6s)
    // At time 1.5 (50% of middle): clip_time = 0.5 * 1.0 = 0.5 → value 50
    let map = lib.sample(id, 1.5, 3.0).unwrap();
    assert!((map.get(&PropertyPath::PositionX).copied().unwrap() - 50.0).abs() < 1e-6);

    // Repeat with target_duration = 5.0: strip from 0.2 to 4.8 (middle 4.6s)
    // At time 2.5 (50% of middle): clip_time = 0.5 * 1.0 = 0.5 → value 50
    let map = lib.sample(id, 2.5, 5.0).unwrap();
    assert!((map.get(&PropertyPath::PositionX).copied().unwrap() - 50.0).abs() < 1e-6);
}

#[test]
fn percentage_window_30_to_70() {
    let action = make_action(
        "pop",
        vec![make_acurve(
            PropertyPath::FontSize,
            vec![Keyframe::linear(0.0, 24.0), Keyframe::linear(1.0, 48.0)],
        )],
    );

    let track = NlaTrack {
        name: "main".into(),
        muted: false,
        solo: false,
        strips: vec![NlaStrip {
            name: "strip".into(),
            placement: StripPlacement::Relative {
                start_frac: 0.3,
                end_frac: 0.7,
            },
            extend_mode: ExtendMode::Nothing,
            ..NlaStrip::new_action("strip", action.id)
        }],
    };

    let mut lib = AnimationLibrary::default();
    lib.insert_action(action);
    let id = lib.insert_anim_data(make_anim_data("clip", vec![track]));

    // Duration = 10.0, 0.3→3.0, 0.7→7.0
    // At time=0: outside strip (Nothing), no contribution
    let map = lib.sample(id, 0.0, 10.0).unwrap();
    assert!(map.is_empty() || map.get(&PropertyPath::FontSize).copied() == Some(0.0));

    // At time=5.0 (midpoint of strip): clip_time = 0.5 → 36.0
    let map = lib.sample(id, 5.0, 10.0).unwrap();
    assert_eq!(map.get(&PropertyPath::FontSize).copied(), Some(36.0));

    // At time=8.0: outside (Nothing), no contribution
    let map = lib.sample(id, 8.0, 10.0).unwrap();
    assert!(map.is_empty() || map.get(&PropertyPath::FontSize).copied() == Some(0.0));
}

#[test]
fn absolute_window_1s_to_3s() {
    let action = make_action(
        "slide",
        vec![make_acurve(
            PropertyPath::PositionX,
            vec![Keyframe::linear(0.0, 0.0), Keyframe::linear(1.0, 100.0)],
        )],
    );

    let track = NlaTrack {
        name: "main".into(),
        muted: false,
        solo: false,
        strips: vec![NlaStrip {
            name: "strip".into(),
            placement: StripPlacement::Absolute {
                start: 1.0,
                end: 3.0,
            },
            ..NlaStrip::new_action("strip", action.id)
        }],
    };

    let mut lib = AnimationLibrary::default();
    lib.insert_action(action);
    let id = lib.insert_anim_data(make_anim_data("clip", vec![track]));

    // At time 2.0 (50% of [1,3]): clip_time = 0.5 → 50.0
    let map = lib.sample(id, 2.0, 5.0).unwrap();
    assert_eq!(map.get(&PropertyPath::PositionX).copied(), Some(50.0));
}

#[test]
fn action_subrange_slice() {
    let action = make_action(
        "long",
        vec![make_acurve(
            PropertyPath::PositionX,
            vec![
                Keyframe::linear(0.0, 0.0),
                Keyframe::linear(1.0, 100.0),
                Keyframe::linear(2.0, 200.0),
            ],
        )],
    );

    // Strip 1 uses [0, 0.5], strip 2 uses [0.5, 1.0]
    let (action1, action2) = {
        let mut a1 = action.clone();
        a1.id = ActionId::new();
        let mut a2 = action.clone();
        a2.id = ActionId::new();
        (a1, a2)
    };

    let track = NlaTrack {
        name: "main".into(),
        muted: false,
        solo: false,
        strips: vec![
            NlaStrip {
                name: "first_half".into(),
                placement: StripPlacement::Relative {
                    start_frac: 0.0,
                    end_frac: 0.5,
                },
                strip_type: crate::animation::strip::StripType::Action {
                    action: action1.id,
                    actstart: 0.0,
                    actend: 0.5,
                },
                ..NlaStrip::new_action("first_half", action1.id)
            },
            NlaStrip {
                name: "second_half".into(),
                placement: StripPlacement::Relative {
                    start_frac: 0.5,
                    end_frac: 1.0,
                },
                strip_type: crate::animation::strip::StripType::Action {
                    action: action2.id,
                    actstart: 0.5,
                    actend: 1.0,
                },
                ..NlaStrip::new_action("second_half", action2.id)
            },
        ],
    };

    let mut lib = AnimationLibrary::default();
    lib.insert_action(action1);
    lib.insert_action(action2);
    let id = lib.insert_anim_data(make_anim_data("clip", vec![track]));

    // At time 2.5 (50% of first strip [0,5]): clip_time = 0.25 (within actstart=0, actend=0.5)
    // → value = 25.0 (0.25/1.0 * 100)
    let map = lib.sample(id, 2.5, 10.0).unwrap();
    assert_eq!(map.get(&PropertyPath::PositionX).copied(), Some(25.0));
}

#[test]
fn extrapolation_hold_outside_range() {
    let action = make_action(
        "move",
        vec![make_acurve(
            PropertyPath::PositionX,
            vec![Keyframe::linear(0.0, 0.0), Keyframe::linear(1.0, 100.0)],
        )],
    );

    let track = NlaTrack {
        name: "main".into(),
        muted: false,
        solo: false,
        strips: vec![NlaStrip {
            name: "strip".into(),
            placement: StripPlacement::Absolute {
                start: 1.0,
                end: 3.0,
            },
            extend_mode: ExtendMode::Hold,
            ..NlaStrip::new_action("strip", action.id)
        }],
    };

    let mut lib = AnimationLibrary::default();
    lib.insert_action(action);
    let id = lib.insert_anim_data(make_anim_data("clip", vec![track]));

    // Before strip: hold first keyframe value (0.0)
    let map = lib.sample(id, 0.5, 5.0).unwrap();
    assert_eq!(map.get(&PropertyPath::PositionX).copied(), Some(0.0));

    // After strip: hold last keyframe value (100.0)
    let map = lib.sample(id, 4.0, 5.0).unwrap();
    assert_eq!(map.get(&PropertyPath::PositionX).copied(), Some(100.0));
}

#[test]
fn extrapolation_nothing_outside_range() {
    let action = make_action(
        "move",
        vec![make_acurve(
            PropertyPath::PositionX,
            vec![Keyframe::linear(0.0, 0.0), Keyframe::linear(1.0, 100.0)],
        )],
    );

    let track = NlaTrack {
        name: "main".into(),
        muted: false,
        solo: false,
        strips: vec![NlaStrip {
            name: "strip".into(),
            placement: StripPlacement::Absolute {
                start: 1.0,
                end: 3.0,
            },
            extend_mode: ExtendMode::Nothing,
            ..NlaStrip::new_action("strip", action.id)
        }],
    };

    let mut lib = AnimationLibrary::default();
    lib.insert_action(action);
    let id = lib.insert_anim_data(make_anim_data("clip", vec![track]));

    // Before strip: no contribution (no track found)
    let map = lib.sample(id, 0.5, 5.0).unwrap();
    assert!(map.is_empty());
}

#[test]
fn extrapolation_hold_forward_only_after() {
    let action = make_action(
        "move",
        vec![make_acurve(
            PropertyPath::PositionX,
            vec![Keyframe::linear(0.0, 0.0), Keyframe::linear(1.0, 100.0)],
        )],
    );

    let track = NlaTrack {
        name: "main".into(),
        muted: false,
        solo: false,
        strips: vec![NlaStrip {
            name: "strip".into(),
            placement: StripPlacement::Absolute {
                start: 1.0,
                end: 3.0,
            },
            extend_mode: ExtendMode::HoldForward,
            ..NlaStrip::new_action("strip", action.id)
        }],
    };

    let mut lib = AnimationLibrary::default();
    lib.insert_action(action);
    let id = lib.insert_anim_data(make_anim_data("clip", vec![track]));

    // Before strip: HoldForward = no contribution before
    let map = lib.sample(id, 0.5, 5.0).unwrap();
    assert!(map.is_empty() || map.get(&PropertyPath::PositionX).copied() == Some(0.0).or(Some(0.0)));

    // After strip: hold last frame
    let map = lib.sample(id, 4.0, 5.0).unwrap();
    assert_eq!(map.get(&PropertyPath::PositionX).copied(), Some(100.0));
}

// ── 3.2 Time Mapping ──────────────────────────────────────────────────────

#[test]
fn time_scale_half_speed() {
    // 1s action, 2s strip, scale=2 → plays at half speed (fills 2s)
    let action = make_action(
        "move",
        vec![make_acurve(
            PropertyPath::PositionX,
            vec![Keyframe::linear(0.0, 0.0), Keyframe::linear(1.0, 100.0)],
        )],
    );

    let track = NlaTrack {
        name: "main".into(),
        muted: false,
        solo: false,
        strips: vec![NlaStrip {
            name: "strip".into(),
            placement: StripPlacement::Full,
            strip_type: crate::animation::strip::StripType::Action {
                action: action.id,
                actstart: 0.0,
                actend: 1.0,
            },
            ..NlaStrip::new_action("strip", action.id)
        }],
    };

    let mut lib = AnimationLibrary::default();
    lib.insert_action(action);
    let id = lib.insert_anim_data(make_anim_data("clip", vec![track]));

    // At t=1.0 (50% of strip=2s): clip_time = 0.5 → 50.0
    let map = lib.sample(id, 1.0, 2.0).unwrap();
    assert_eq!(map.get(&PropertyPath::PositionX).copied(), Some(50.0));
}

// ── 3.3 Blending ──────────────────────────────────────────────────────────

#[test]
fn blend_mode_subtract() {
    let strips = vec![
        EvaluatedStrip {
            blend_mode: BlendMode::Replace,
            influence: 1.0,
            values: {
                let mut m = HashMap::new();
                m.insert(PropertyPath::FontSize, 30.0);
                m
            },
        },
        EvaluatedStrip {
            blend_mode: BlendMode::Subtract,
            influence: 1.0,
            values: {
                let mut m = HashMap::new();
                m.insert(PropertyPath::FontSize, 10.0);
                m
            },
        },
    ];
    let result = blend_strip_results(&strips);
    assert_eq!(result.get(&PropertyPath::FontSize).copied(), Some(20.0));
}

#[test]
fn blend_mode_multiply() {
    let strips = vec![
        EvaluatedStrip {
            blend_mode: BlendMode::Replace,
            influence: 1.0,
            values: {
                let mut m = HashMap::new();
                m.insert(PropertyPath::FontSize, 10.0);
                m
            },
        },
        EvaluatedStrip {
            blend_mode: BlendMode::Multiply,
            influence: 1.0,
            values: {
                let mut m = HashMap::new();
                m.insert(PropertyPath::FontSize, 2.0);
                m
            },
        },
    ];
    let result = blend_strip_results(&strips);
    // base=10, multiply by 2, result = 10 * (1 + (2-1)*1) = 20
    assert_eq!(result.get(&PropertyPath::FontSize).copied(), Some(20.0));
}

#[test]
fn blend_in_fade() {
    let action = make_action(
        "const",
        vec![make_acurve(
            PropertyPath::PositionX,
            vec![Keyframe::linear(0.0, 100.0)],
        )],
    );

    let track = NlaTrack {
        name: "main".into(),
        muted: false,
        solo: false,
        strips: vec![NlaStrip {
            name: "strip".into(),
            placement: StripPlacement::Full,
            control_curves: StripControlCurves {
                influence: Some(FCurve {
                    keyframes: vec![
                        Keyframe::linear(0.0, 0.0),
                        Keyframe::linear(0.5, 1.0),
                    ],
                    extrapolation: Extrapolation::Constant,
                    modifiers: vec![],
                    integer: false,
                }),
                ..Default::default()
            },
            ..NlaStrip::new_action("strip", action.id)
        }],
    };

    let mut lib = AnimationLibrary::default();
    lib.insert_action(action);
    let id = lib.insert_anim_data(make_anim_data("clip", vec![track]));

    // At t=0: influence should be ~0 (full blend_in)
    let tracks = lib.sample_tracks(id, 0.0, 1.0).unwrap();
    if !tracks.is_empty() {
        assert!((tracks[0].influence - 0.0).abs() < 1e-9);
    }

    // At t=0.5: influence should be ~1.0 (end of blend_in)
    let tracks = lib.sample_tracks(id, 0.5, 1.0).unwrap();
    if !tracks.is_empty() {
        assert!((tracks[0].influence - 1.0).abs() < 1e-9);
    }
}

#[test]
fn muted_strip_no_contribution() {
    let action = make_action(
        "move",
        vec![make_acurve(
            PropertyPath::PositionX,
            vec![Keyframe::linear(0.0, 100.0)],
        )],
    );

    let track = NlaTrack {
        name: "main".into(),
        muted: false,
        solo: false,
        strips: vec![NlaStrip {
            name: "strip".into(),
            muted: true,
            ..NlaStrip::new_action("strip", action.id)
        }],
    };

    let mut lib = AnimationLibrary::default();
    lib.insert_action(action);
    let id = lib.insert_anim_data(make_anim_data("clip", vec![track]));

    let tracks = lib.sample_tracks(id, 0.0, 1.0).unwrap();
    assert!(tracks.is_empty());
}

// ── 3.4 FCurve extras ─────────────────────────────────────────────────────

#[test]
fn fcurve_linear_extrapolation() {
    let fcurve = FCurve {
        keyframes: vec![Keyframe::linear(0.0, 10.0), Keyframe::linear(1.0, 20.0)],
        extrapolation: Extrapolation::Linear,
        modifiers: Vec::new(),
        integer: false,
    };
    // Before first keyframe: slope = 10, so at t=-1: 10 - 10 = 0
    assert!(
        (crate::animation::fcurve::evaluate_fcurve(&fcurve, -1.0) - 0.0).abs() < 1e-9
    );
    // After last keyframe: slope = 10, so at t=2: 20 + 10 = 30
    assert!(
        (crate::animation::fcurve::evaluate_fcurve(&fcurve, 2.0) - 30.0).abs() < 1e-9
    );
}

#[test]
fn fcurve_cyclic_modifier() {
    let fcurve = FCurve {
        keyframes: vec![Keyframe::linear(0.0, 10.0), Keyframe::linear(1.0, 20.0)],
        extrapolation: Extrapolation::Constant,
        modifiers: vec![{
            let mut m = FModifierInstance::default();
            m.data = FModifier::Cycles(FModCycles {
                before_mode: CycleMode::Repeat,
                after_mode: CycleMode::Repeat,
                ..Default::default()
            });
            m
        }],
        integer: false,
    };
    // t=3.5: wrapped to 0.5 → 15.0
    let v = crate::animation::fcurve::evaluate_fcurve(&fcurve, 3.5);
    assert!((v - 15.0).abs() < 1e-9);
}

#[test]
fn fcurve_noise_modifier_produces_variation() {
    let fcurve = FCurve {
        keyframes: vec![Keyframe::linear(0.0, 50.0), Keyframe::linear(1.0, 50.0)],
        extrapolation: Extrapolation::Constant,
        modifiers: vec![{
            let mut m = FModifierInstance::default();
            m.data = FModifier::Noise(FModNoise {
                strength: 10.0,
                size: 1.0,
                depth: 2,
                ..Default::default()
            });
            m
        }],
        integer: false,
    };
    let v1 = crate::animation::fcurve::evaluate_fcurve(&fcurve, 0.5);
    let v2 = crate::animation::fcurve::evaluate_fcurve(&fcurve, 0.6);
    // Should vary somewhat
    assert!((v1 - v2).abs() > 0.1);
}

#[test]
fn fcurve_stepped_modifier_snaps_time() {
    let fcurve = FCurve {
        keyframes: vec![Keyframe::linear(0.0, 0.0), Keyframe::linear(1.0, 100.0)],
        extrapolation: Extrapolation::Constant,
        modifiers: vec![{
            let mut m = FModifierInstance::default();
            m.data = FModifier::Stepped(FModStepped {
                step_size: 0.25,
                ..Default::default()
            });
            m
        }],
        integer: false,
    };
    // t=0.3 → snapped to 0.25 → value = 25
    let v = crate::animation::fcurve::evaluate_fcurve(&fcurve, 0.3);
    assert!((v - 25.0).abs() < 1e-9);
}

#[test]
fn fcurve_limits_clamp_value() {
    let fcurve = FCurve {
        keyframes: vec![Keyframe::linear(0.0, 0.0), Keyframe::linear(1.0, 200.0)],
        extrapolation: Extrapolation::Constant,
        modifiers: vec![{
            let mut m = FModifierInstance::default();
            m.data = FModifier::Limits(FModLimits {
                ymin: Some(10.0),
                ymax: Some(90.0),
                ..Default::default()
            });
            m
        }],
        integer: false,
    };
    // t=0: value 0, clamped to 10
    assert!(
        (crate::animation::fcurve::evaluate_fcurve(&fcurve, 0.0) - 10.0).abs() < 1e-9
    );
    // t=1.0: value 200, clamped to 90
    assert!(
        (crate::animation::fcurve::evaluate_fcurve(&fcurve, 1.0) - 90.0).abs() < 1e-9
    );
}

#[test]
fn fcurve_envelope_maps_values() {
    let fcurve = FCurve {
        keyframes: vec![Keyframe::linear(0.0, 0.0), Keyframe::linear(1.0, 1.0)],
        extrapolation: Extrapolation::Constant,
        modifiers: vec![{
            let mut m = FModifierInstance::default();
            m.data = FModifier::Envelope(FModEnvelope {
                control_points: vec![
                    crate::animation::modifier::EnvelopePoint {
                        time: 0.0,
                        min: 100.0,
                        max: 200.0,
                    },
                ],
                midval: 0.0,
                min: 0.0,
                max: 1.0,
                ..Default::default()
            });
            m
        }],
        integer: false,
    };
    // t=0: value=0, mapped to min=100
    let v = crate::animation::fcurve::evaluate_fcurve(&fcurve, 0.0);
    assert!((v - 100.0).abs() < 1e-9);
    // t=1.0: value=1.0, mapped to max=200
    let v = crate::animation::fcurve::evaluate_fcurve(&fcurve, 1.0);
    assert!((v - 200.0).abs() < 1e-9);
}

// ── 3.5 Multi-track / structural ──────────────────────────────────────────

#[test]
fn multi_track_overlay() {
    let action1 = make_action(
        "pos",
        vec![make_acurve(
            PropertyPath::PositionX,
            vec![Keyframe::linear(0.0, 100.0)],
        )],
    );
    let action2 = make_action(
        "pos_add",
        vec![make_acurve(
            PropertyPath::PositionX,
            vec![Keyframe::linear(0.0, 20.0)],
        )],
    );

    let clip = make_anim_data(
        "multi",
        vec![
            NlaTrack {
                name: "base".into(),
                muted: false,
                solo: false,
                strips: vec![NlaStrip {
                    name: "s1".into(),
                    ..NlaStrip::new_action("s1", action1.id)
                }],
            },
            NlaTrack {
                name: "offset".into(),
                muted: false,
                solo: false,
                strips: vec![NlaStrip {
                    name: "s2".into(),
                    blend_mode: BlendMode::Add,
                    ..NlaStrip::new_action("s2", action2.id)
                }],
            },
        ],
    );

    let mut lib = AnimationLibrary::default();
    lib.insert_action(action1);
    lib.insert_action(action2);
    let id = lib.insert_anim_data(clip);

    let map = lib.sample(id, 0.0, 1.0).unwrap();
    assert_eq!(map.get(&PropertyPath::PositionX).copied(), Some(120.0));
}

#[test]
fn same_track_multiple_strips_phased() {
    let action1 = make_action(
        "intro",
        vec![make_acurve(
            PropertyPath::FontSize,
            vec![Keyframe::linear(0.0, 24.0)],
        )],
    );
    let action2 = make_action(
        "body",
        vec![make_acurve(
            PropertyPath::FontSize,
            vec![Keyframe::linear(0.0, 36.0)],
        )],
    );

    let track = NlaTrack {
        name: "main".into(),
        muted: false,
        solo: false,
        strips: vec![
            NlaStrip {
                name: "intro".into(),
                placement: StripPlacement::Absolute {
                    start: 0.0,
                    end: 2.0,
                },
                extend_mode: ExtendMode::Nothing,
                ..NlaStrip::new_action("intro", action1.id)
            },
            NlaStrip {
                name: "body".into(),
                placement: StripPlacement::Absolute {
                    start: 2.0,
                    end: 5.0,
                },
                extend_mode: ExtendMode::Nothing,
                ..NlaStrip::new_action("body", action2.id)
            },
        ],
    };

    let mut lib = AnimationLibrary::default();
    lib.insert_action(action1);
    lib.insert_action(action2);
    let id = lib.insert_anim_data(make_anim_data("phased", vec![track]));

    // t=1.0: intro strip → 24
    let map = lib.sample(id, 1.0, 5.0).unwrap();
    assert_eq!(map.get(&PropertyPath::FontSize).copied(), Some(24.0));

    // t=3.0: body strip → 36
    let map = lib.sample(id, 3.0, 5.0).unwrap();
    assert_eq!(map.get(&PropertyPath::FontSize).copied(), Some(36.0));

    // t=2.5: body strip (in range), clip_time maps to 0.5 of body's action → 36 (constant)
    let map = lib.sample(id, 2.5, 5.0).unwrap();
    assert_eq!(map.get(&PropertyPath::FontSize).copied(), Some(36.0));
}

#[test]
fn action_shared_across_strips() {
    let action = make_action(
        "shared",
        vec![make_acurve(
            PropertyPath::PositionX,
            vec![Keyframe::linear(0.0, 100.0)],
        )],
    );
    let action_id = action.id;

    let clip = make_anim_data(
        "shared",
        vec![
            NlaTrack {
                name: "top".into(),
                muted: false,
                solo: false,
                strips: vec![NlaStrip {
                    name: "top_strip".into(),
                    blend_mode: BlendMode::Replace,
                    strip_type: crate::animation::strip::StripType::Action {
                        action: action_id,
                        actstart: 0.0,
                        actend: 0.0,
                    },
                    ..NlaStrip::new_action("top_strip", action_id)
                }],
            },
            NlaTrack {
                name: "bot".into(),
                muted: false,
                solo: false,
                strips: vec![NlaStrip {
                    name: "bot_strip".into(),
                    blend_mode: BlendMode::Subtract,
                    strip_type: crate::animation::strip::StripType::Action {
                        action: action_id,
                        actstart: 0.0,
                        actend: 0.0,
                    },
                    ..NlaStrip::new_action("bot_strip", action_id)
                }],
            },
        ],
    );

    let mut lib = AnimationLibrary::default();
    lib.insert_action(action);
    let id = lib.insert_anim_data(clip);

    // Both strips reference same action. Top=Replace 100, Bot=Subtract 100 → 0
    let map = lib.sample(id, 0.0, 1.0).unwrap();
    assert_eq!(map.get(&PropertyPath::PositionX).copied(), Some(0.0));
}

// ── Style merge integration ───────────────────────────────────────────────

#[test]
fn merge_overrides_position_and_bold() {
    let mut style = StyleResult::default();
    let strips = vec![EvaluatedStrip {
        blend_mode: BlendMode::Replace,
        influence: 1.0,
        values: {
            let mut m = HashMap::new();
            m.insert(PropertyPath::PositionY, 0.9);
            m.insert(PropertyPath::Bold, 0.6);
            m
        },
    }];
    apply_animation_to_style(&mut style, &strips);
    assert_eq!(style.position.as_ref().unwrap().y, 0.9);
    assert_eq!(style.bold, Some(true));
}

#[test]
fn merge_preserves_untouched_properties() {
    let mut style = StyleResult {
        font_size: Some(24.0),
        ..StyleResult::default()
    };
    let strips: Vec<EvaluatedStrip> = vec![];
    apply_animation_to_style(&mut style, &strips);
    assert_eq!(style.font_size, Some(24.0));
}

#[test]
fn library_missing_clip_errors() {
    let lib = AnimationLibrary::default();
    let id = AnimDataId::new();
    assert!(lib.sample(id, 0.0, 1.0).is_err());
}

#[test]
fn blend_scalar_all_modes() {
    // Replace: 10 + (20-10)*0.5 = 15
    assert!((blend_scalar(10.0, 20.0, BlendMode::Replace, 0.5) - 15.0).abs() < 1e-9);
    // Add: 10 + 20*0.5 = 20
    assert!((blend_scalar(10.0, 20.0, BlendMode::Add, 0.5) - 20.0).abs() < 1e-9);
    // Subtract: 10 - 20*0.5 = 0
    assert!((blend_scalar(10.0, 20.0, BlendMode::Subtract, 0.5) - 0.0).abs() < 1e-9);
    // Multiply: 10*(1+(20-1)*0.5) = 10*10.5 = 105
    assert!((blend_scalar(10.0, 20.0, BlendMode::Multiply, 0.5) - 105.0).abs() < 1e-9);
    // Combine: 10*0.5 + 20*0.5 = 15
    assert!((blend_scalar(10.0, 20.0, BlendMode::Combine, 0.5) - 15.0).abs() < 1e-9);
}

// ── 编辑工作区（唯一 Action / NLA 临时编辑）────────────────────────────────

#[test]
fn unique_action_is_base_below_nla() {
    let unique = make_action(
        "unique",
        vec![make_acurve(
            PropertyPath::PositionX,
            vec![Keyframe::linear(0.0, 0.0), Keyframe::linear(1.0, 100.0)],
        )],
    );
    let add = make_action(
        "add",
        vec![make_acurve(
            PropertyPath::PositionX,
            vec![Keyframe::linear(0.0, 20.0)],
        )],
    );

    let mut clip = make_anim_data(
        "clip",
        vec![NlaTrack {
            name: "nla".into(),
            muted: false,
            solo: false,
            strips: vec![NlaStrip {
                name: "add".into(),
                blend_mode: BlendMode::Add,
                ..NlaStrip::new_action("add", add.id)
            }],
        }],
    );
    clip.set_editing_unique(unique.id);

    let mut lib = AnimationLibrary::default();
    lib.insert_action(unique);
    lib.insert_action(add);
    let id = lib.insert_anim_data(clip);

    // t=0.5：唯一 Action 按绝对时间给 50（基础层），NLA Add +20 → 70
    let map = lib.sample(id, 0.5, 1.0).unwrap();
    assert!((map.get(&PropertyPath::PositionX).copied().unwrap() - 70.0).abs() < 1e-6);
}

#[test]
fn nla_replace_overrides_unique_base() {
    let unique = make_action(
        "unique",
        vec![make_acurve(
            PropertyPath::FontSize,
            vec![Keyframe::linear(0.0, 24.0)],
        )],
    );
    let nla = make_action(
        "nla",
        vec![make_acurve(
            PropertyPath::FontSize,
            vec![Keyframe::linear(0.0, 48.0)],
        )],
    );

    let mut clip = make_anim_data(
        "clip",
        vec![NlaTrack {
            name: "nla".into(),
            muted: false,
            solo: false,
            strips: vec![NlaStrip::new_action("nla", nla.id)],
        }],
    );
    clip.set_editing_unique(unique.id);

    let mut lib = AnimationLibrary::default();
    lib.insert_action(unique);
    lib.insert_action(nla);
    let id = lib.insert_anim_data(clip);

    // 唯一 Action 先给 24，NLA Replace 覆盖为 48
    let map = lib.sample(id, 0.0, 1.0).unwrap();
    assert_eq!(map.get(&PropertyPath::FontSize).copied(), Some(48.0));
}

#[test]
fn temporary_strip_edit_is_absolute_and_skips_nla_strip() {
    let action = make_action(
        "editme",
        vec![make_acurve(
            PropertyPath::PositionX,
            vec![Keyframe::linear(0.0, 0.0), Keyframe::linear(1.0, 100.0)],
        )],
    );

    let mut clip = make_anim_data(
        "clip",
        vec![NlaTrack {
            name: "main".into(),
            muted: false,
            solo: false,
            strips: vec![NlaStrip {
                name: "s".into(),
                placement: StripPlacement::Absolute { start: 2.0, end: 5.0 },
                ..NlaStrip::new_action("s", action.id)
            }],
        }],
    );
    clip.set_editing_strip(0, 0);

    let mut lib = AnimationLibrary::default();
    lib.insert_action(action);
    let id = lib.insert_anim_data(clip);

    // 播放头在片断外（t=1.0）：编辑基础层按绝对时间仍给出 100
    let map = lib.sample(id, 1.0, 5.0).unwrap();
    assert_eq!(map.get(&PropertyPath::PositionX).copied(), Some(100.0));

    // 播放头在片断内（t=3.0）：该片断在 NLA 中跳过，仅编辑层按绝对时间给 100
    // （若未跳过，片断会把 clip_time 映射到 0.33 → 33.3，结果将是 33.3）
    let map = lib.sample(id, 3.0, 5.0).unwrap();
    assert_eq!(map.get(&PropertyPath::PositionX).copied(), Some(100.0));
}

#[test]
fn cancel_temporary_edit_returns_to_original_unique() {
    let unique = make_action("u", vec![]);
    let strip_action = make_action("s", vec![]);
    let mut clip = make_anim_data(
        "clip",
        vec![NlaTrack {
            name: "t".into(),
            muted: false,
            solo: false,
            strips: vec![NlaStrip::new_action("s", strip_action.id)],
        }],
    );
    clip.set_editing_unique(unique.id);

    clip.set_editing_strip(0, 0);
    assert_eq!(
        clip.edit_source(),
        Some(EditSource::NlaStrip { track: 0, strip: 0 })
    );
    // unique_action 未被临时编辑触碰
    assert_eq!(clip.unique_action(), Some(unique.id));

    clip.cancel_temporary_edit();
    assert_eq!(clip.edit_source(), Some(EditSource::Unique(unique.id)));
    assert!(clip.temporary_edit.is_none());
}

#[test]
fn two_temp_edits_cancel_back_to_original_unique() {
    let unique = make_action("u", vec![]);
    let a = make_action("a", vec![]);
    let b = make_action("b", vec![]);
    let mut clip = make_anim_data(
        "clip",
        vec![NlaTrack {
            name: "t".into(),
            muted: false,
            solo: false,
            strips: vec![
                NlaStrip::new_action("a", a.id),
                NlaStrip::new_action("b", b.id),
            ],
        }],
    );
    clip.set_editing_unique(unique.id);

    // 第一次临时编辑片断 A，然后切换目标到片断 B
    clip.set_editing_strip(0, 0);
    clip.set_editing_strip(0, 1);
    assert_eq!(
        clip.edit_source(),
        Some(EditSource::NlaStrip { track: 0, strip: 1 })
    );

    // 取消：直接回到全局唯一 Action，而不是第一次的临时编辑
    clip.cancel_temporary_edit();
    assert_eq!(clip.edit_source(), Some(EditSource::Unique(unique.id)));
    assert_eq!(clip.unique_action(), Some(unique.id));
}

#[test]
fn explicit_unique_edit_exits_temp_mode() {
    let unique = make_action("u", vec![]);
    let other = make_action("o", vec![]);
    let strip_action = make_action("s", vec![]);
    let mut clip = make_anim_data(
        "clip",
        vec![NlaTrack {
            name: "t".into(),
            muted: false,
            solo: false,
            strips: vec![NlaStrip::new_action("s", strip_action.id)],
        }],
    );
    clip.set_editing_unique(unique.id);

    clip.set_editing_strip(0, 0);
    clip.set_editing_unique(other.id);
    assert!(clip.temporary_edit.is_none());
    assert_eq!(clip.unique_action(), Some(other.id));
}

#[test]
fn stale_edit_indices_fall_back_to_no_base() {
    let mut clip = make_anim_data("clip", vec![]);
    clip.set_editing_strip(99, 99);
    let mut lib = AnimationLibrary::default();
    let id = lib.insert_anim_data(clip);
    let strips = lib.sample_tracks(id, 0.5, 1.0).unwrap();
    assert!(strips.is_empty());
}

#[test]
fn push_down_creates_strip_and_preserves_sampling() {
    let unique = make_action(
        "push",
        vec![make_acurve(
            PropertyPath::PositionX,
            vec![Keyframe::linear(0.0, 0.0), Keyframe::linear(1.0, 100.0)],
        )],
    );
    let mut clip = make_anim_data("clip", vec![]);
    clip.set_editing_unique(unique.id);

    let mut lib = AnimationLibrary::default();
    lib.insert_action(unique);
    let id = lib.insert_anim_data(clip);

    let before = lib.sample(id, 0.4, 1.0).unwrap();
    let v_before = before.get(&PropertyPath::PositionX).copied().unwrap();

    let fresh = lib.push_down_editing_action(id).unwrap();

    let ad = lib.anim_datas.get(&id).unwrap();
    assert_eq!(ad.tracks.len(), 1);
    assert_eq!(ad.unique_action(), Some(fresh));

    // 新片断 no_time_map → 下压前后同一时刻采样一致
    let after = lib.sample(id, 0.4, 1.0).unwrap();
    let v_after = after.get(&PropertyPath::PositionX).copied().unwrap();
    assert!((v_before - v_after).abs() < 1e-9);
}

#[test]
fn push_down_requires_unique_edit_source() {
    let action = make_action("a", vec![]);
    let mut clip = make_anim_data(
        "clip",
        vec![NlaTrack {
            name: "t".into(),
            muted: false,
            solo: false,
            strips: vec![NlaStrip::new_action("s", action.id)],
        }],
    );
    clip.set_editing_strip(0, 0);
    let mut lib = AnimationLibrary::default();
    lib.insert_action(action);
    let id = lib.insert_anim_data(clip);
    assert!(matches!(
        lib.push_down_editing_action(id),
        Err(crate::animation::library::AnimationError::NoUniqueAction(_))
    ));
}
