//! Skins and animations, read from a rigged bar built here.

use super::*;
use crate::math::combine;
use crate::{Interpolation, Keys, MAX_JOINTS, Pose, Transform};

const U8: u32 = 5121;
const I16: u32 = 5122;

/// A matrix that only moves.
fn moved(by: [f32; 3]) -> [f32; 16] {
    let mut matrix = [0.0; 16];
    for diagonal in [0, 5, 10, 15] {
        matrix[diagonal] = 1.0;
    }
    matrix[12..15].copy_from_slice(&by);
    matrix
}

fn bytes_of<const N: usize>(items: &[[u8; N]]) -> Vec<u8> {
    items.iter().flatten().copied().collect()
}

/// The data of a square bar with a bone at each end. Accessors: 0 positions,
/// 1 triangles, 2 normals, 3 bone numbers, 4 bone weights, 5 bind poses,
/// 6 key times, 7 rotations, 8 translations.
fn bar() -> Builder {
    let mut file = Builder::default();
    file.positions(&QUAD);
    file.indices(&[0, 1, 2, 0, 2, 3]);
    file.vec3(&[[0.0, 0.0, 1.0]; 4]);
    file.list(
        &bytes_of(&[[0, 1, 0, 0], [0, 1, 0, 0], [1, 0, 0, 0], [1, 0, 0, 0]]),
        4,
        U8,
        "VEC4",
        "",
    );
    file.floats(
        "VEC4",
        &[
            [2.0, 2.0, 0.0, 0.0],
            [0.25, 0.75, 0.0, 0.0],
            [1.0, 0.0, 0.0, 0.0],
            [0.0, 0.0, 0.0, 1.0],
        ],
        "",
    );
    file.floats(
        "MAT4",
        &[moved([0.0, -1.0, 0.0]), moved([0.0, -2.0, 0.0])],
        "",
    );
    file.floats("SCALAR", &[[0.0], [0.5], [1.0]], r#","min":[0],"max":[1]"#);
    file.floats(
        "VEC4",
        &[
            [0.0, 0.0, 0.0, 1.0],
            [0.0, 0.0, 0.6, 0.8],
            [0.0, 0.0, 2.0, 0.0],
        ],
        "",
    );
    file.vec3(&[[0.0, 0.0, 0.0], [0.0, 0.5, 0.0], [0.0, 0.0, 0.0]]);
    file
}

const BAR_MESH: &str = r#""meshes":[{"name":"Bar","primitives":[{"attributes":{"POSITION":0,"NORMAL":2,"JOINTS_0":3,"WEIGHTS_0":4},"indices":1}]}]"#;
const BAR_NODES: &str = r#""nodes":[
    {"name":"Bar","mesh":0,"skin":0,"translation":[5,0,0]},
    {"name":"Armature","translation":[0,1,0],"children":[2]},
    {"name":"Root","children":[3]},
    {"name":"Tip","translation":[0,1,0],"scale":[1,2,1]}],
    "scenes":[{"nodes":[0,1]}]"#;
const BAR_SKIN: &str =
    r#""skins":[{"name":"Rig","joints":[2,3],"inverseBindMatrices":5,"skeleton":2}]"#;
const BAR_CLIP: &str = r#""animations":[{"name":"Bend",
    "samplers":[{"input":6,"output":7},{"input":6,"output":8,"interpolation":"STEP"}],
    "channels":[{"sampler":0,"target":{"node":3,"path":"rotation"}},
                {"sampler":1,"target":{"node":2,"path":"translation"}}]}]"#;

fn bar_json() -> String {
    [BAR_MESH, BAR_NODES, BAR_SKIN, BAR_CLIP].join(",")
}

/// Reads the bar after `change` has altered its data and its description.
fn changed(change: impl FnOnce(&mut Builder, &mut String)) -> Result<Scene, AssetError> {
    let mut file = bar();
    let mut json = bar_json();
    change(&mut file, &mut json);
    read(&file.glb(&json))
}

/// The message of the `Damaged` error that `change` must cause.
fn damage(change: impl FnOnce(&mut Builder, &mut String)) -> String {
    let error = changed(change).unwrap_err();
    assert!(matches!(error, AssetError::Damaged { .. }), "{error}");
    error.to_string()
}

fn swap(json: &mut String, from: &str, to: &str) {
    assert!(json.contains(from), "{from} is not in {json}");
    *json = json.replace(from, to);
}

fn close(a: &Transform, b: &Transform) -> bool {
    a.iter()
        .flatten()
        .zip(b.iter().flatten())
        .all(|(a, b)| (a - b).abs() < 1e-6)
}

#[test]
fn a_rigged_bar_reads_its_skeleton_and_its_clip() {
    let scene = changed(|_, _| {}).unwrap();
    assert_eq!(scene.warnings.len(), 2, "{:?}", scene.warnings);
    assert!(
        scene.warnings.iter().all(|w| w.contains("rest pose")),
        "{:?}",
        scene.warnings
    );

    // The tree, parents first.
    let names: Vec<_> = scene.nodes.iter().map(|n| n.name.as_deref()).collect();
    assert_eq!(
        names,
        [Some("Bar"), Some("Armature"), Some("Root"), Some("Tip")]
    );
    let parents: Vec<_> = scene.nodes.iter().map(|n| n.parent).collect();
    assert_eq!(parents, [None, None, Some(1), Some(2)]);
    assert_eq!(scene.nodes[1].rest.translation, [0.0, 1.0, 0.0]);
    assert_eq!(scene.nodes[2].rest, Pose::default());
    assert_eq!(scene.nodes[3].rest.scale, [1.0, 2.0, 1.0]);
    assert_eq!(
        (scene.nodes[0].instance, scene.nodes[0].skin),
        (Some(0), Some(0))
    );
    assert_eq!((scene.nodes[3].instance, scene.nodes[3].skin), (None, None));
    // Rest poses multiplied down the tree are where things stand.
    assert!(close(
        &scene.nodes[0].rest.transform(),
        &scene.instances[0].transform
    ));
    let tip = [1, 2, 3].iter().fold(crate::math::IDENTITY, |place, &n| {
        combine(&place, &scene.nodes[n].rest.transform())
    });
    assert_eq!(tip[3], [0.0, 2.0, 0.0, 1.0]);
    assert_eq!(tip[1], [0.0, 2.0, 0.0, 0.0]);

    assert_eq!(scene.skins.len(), 1);
    let skin = &scene.skins[0];
    assert_eq!(skin.name.as_deref(), Some("Rig"));
    let joints: Vec<_> = skin.joints.iter().map(|j| j.node).collect();
    assert_eq!(joints, [2, 3]);
    assert_eq!(skin.joints[0].inverse_bind[3], [0.0, -1.0, 0.0, 1.0]);
    assert_eq!(skin.joints[1].inverse_bind[3], [0.0, -2.0, 0.0, 1.0]);

    // Weights add up to 1 with the strongest first; a bone that does not
    // pull is bone 0 with no weight.
    let part = &scene.meshes[0].parts[0];
    assert_eq!(
        part.joints.as_deref().unwrap(),
        [[0, 1, 0, 0], [1, 0, 0, 0], [1, 0, 0, 0], [0, 0, 0, 0]]
    );
    assert_eq!(
        part.weights.as_deref().unwrap(),
        [
            [0.5, 0.5, 0.0, 0.0],
            [0.75, 0.25, 0.0, 0.0],
            [1.0, 0.0, 0.0, 0.0],
            [1.0, 0.0, 0.0, 0.0]
        ]
    );

    assert_eq!(scene.animations.len(), 1);
    let clip = &scene.animations[0];
    assert_eq!(clip.name.as_deref(), Some("Bend"));
    assert_eq!(clip.duration, 1.0);
    assert_eq!(clip.tracks.len(), 2);
    let turn = &clip.tracks[0];
    assert_eq!((turn.node, turn.interpolation), (3, Interpolation::Linear));
    assert_eq!(turn.times, [0.0, 0.5, 1.0]);
    // The last rotation was twice as long as a rotation should be.
    assert_eq!(
        turn.keys,
        Keys::Rotation(vec![
            [0.0, 0.0, 0.0, 1.0],
            [0.0, 0.0, 0.6, 0.8],
            [0.0, 0.0, 1.0, 0.0]
        ])
    );
    let slide = &clip.tracks[1];
    assert_eq!((slide.node, slide.interpolation), (2, Interpolation::Step));
    assert_eq!(
        slide.keys,
        Keys::Translation(vec![[0.0, 0.0, 0.0], [0.0, 0.5, 0.0], [0.0, 0.0, 0.0]])
    );

    // A server reading shapes only gets the same skeleton and clip.
    let shapes = Scene::from_bytes(
        "level.glb",
        &bar().glb(&bar_json()),
        None,
        LoadOptions::shapes_only(),
    )
    .unwrap();
    assert_eq!(
        (&shapes.nodes, &shapes.skins, &shapes.animations),
        (&scene.nodes, &scene.skins, &scene.animations)
    );
}

#[test]
fn a_pose_turns_into_a_matrix() {
    // A quarter turn about +Z takes +X to +Y; then stretched and moved.
    let half = 0.5f32.sqrt();
    let pose = Pose {
        translation: [1.0, 2.0, 3.0],
        rotation: [0.0, 0.0, half, half],
        scale: [2.0, 1.0, 1.0],
    };
    let expected = [
        [0.0, 2.0, 0.0, 0.0],
        [-1.0, 0.0, 0.0, 0.0],
        [0.0, 0.0, 1.0, 0.0],
        [1.0, 2.0, 3.0, 1.0],
    ];
    assert!(
        close(&pose.transform(), &expected),
        "{:?}",
        pose.transform()
    );
    assert_eq!(Pose::default().transform(), crate::math::IDENTITY);
}

#[test]
fn points_pulled_by_many_bones_keep_the_four_strongest() {
    let scene = changed(|file, json| {
        // A second set of four, in whole numbers out of 255 and 65535.
        file.list(&bytes_of(&[[1, 1, 1, 1]; 4]), 4, U8, "VEC4", "");
        let weights: Vec<u8> = [[0u16, 0, 0, 0], [65535, 0, 0, 0], [0; 4], [0; 4]]
            .iter()
            .flatten()
            .flat_map(|w| w.to_le_bytes())
            .collect();
        file.list(&weights, 4, U16, "VEC4", r#","normalized":true"#);
        // The first set pulls point 0 four ways, and point 3 not at all.
        file.accessors[4] = file.accessors[4].replace('}', r#","normalized":true}"#);
        let first = bytes_of(&[[51, 102, 51, 51], [0, 255, 0, 0], [255, 0, 0, 0], [0; 4]]);
        let start = 4 * 12 + 12 + 4 * 12 + 16;
        file.bin[start..start + 16].copy_from_slice(&first);
        file.accessors[4] = file.accessors[4].replace("5126", "5121");
        swap(
            json,
            r#""WEIGHTS_0":4"#,
            r#""WEIGHTS_0":4,"JOINTS_1":9,"WEIGHTS_1":10"#,
        );
    })
    .unwrap();
    let part = &scene.meshes[0].parts[0];
    let (joints, weights) = (
        part.joints.as_deref().unwrap(),
        part.weights.as_deref().unwrap(),
    );
    // Point 0: 0.2, 0.4, 0.2, 0.2 of bones 0, 1, 0, 0.
    assert_eq!(joints[0], [1, 0, 0, 0]);
    assert!((weights[0][0] - 0.4).abs() < 1e-6 && (weights[0][3] - 0.2).abs() < 1e-6);
    // Point 1: bone 1 fully from each set; both are kept and share it.
    assert_eq!(
        (joints[1], weights[1]),
        ([1, 1, 0, 0], [0.5, 0.5, 0.0, 0.0])
    );
    // Point 3 has no weights at all: it follows the first bone, and says so.
    assert_eq!((joints[3], weights[3]), ([0; 4], [1.0, 0.0, 0.0, 0.0]));
    for weights in weights {
        assert!((weights.iter().sum::<f32>() - 1.0).abs() < 1e-6);
    }
    assert!(
        scene
            .warnings
            .iter()
            .any(|w| w.contains("1 points that no bone moves")),
        "{:?}",
        scene.warnings
    );
    assert!(!scene.warnings.iter().any(|w| w.contains("more than four")));

    // Five bones on one point: the weakest goes, and the author is told.
    let scene = changed(|file, json| {
        file.list(&bytes_of(&[[1, 1, 1, 1]; 4]), 4, U8, "VEC4", "");
        file.floats("VEC4", &[[4.0, 0.0, 0.0, 0.0]; 4], "");
        let start = 4 * 12 + 12 + 4 * 12 + 16;
        let first: Vec<u8> = [1.0f32, 3.0, 2.0, 2.0]
            .iter()
            .flat_map(|w| w.to_le_bytes())
            .collect();
        file.bin[start..start + 16].copy_from_slice(&first);
        swap(
            json,
            r#""WEIGHTS_0":4"#,
            r#""WEIGHTS_0":4,"JOINTS_1":9,"WEIGHTS_1":10"#,
        );
    })
    .unwrap();
    let part = &scene.meshes[0].parts[0];
    assert_eq!(part.joints.as_deref().unwrap()[0], [1, 1, 0, 0]);
    assert_eq!(
        part.weights.as_deref().unwrap()[0],
        [4.0 / 11.0, 3.0 / 11.0, 2.0 / 11.0, 2.0 / 11.0]
    );
    assert!(
        scene.warnings.iter().any(|w| w.contains("Limit Total")),
        "{:?}",
        scene.warnings
    );
}

#[test]
fn bone_weights_follow_points_that_are_split() {
    // Without normals every corner gets its own point, and its own weights.
    let scene = changed(|_, json| swap(json, r#""NORMAL":2,"#, "")).unwrap();
    let part = &scene.meshes[0].parts[0];
    assert_eq!(part.positions.len(), 6);
    assert_eq!(
        part.joints.as_deref().unwrap(),
        [
            [0, 1, 0, 0],
            [1, 0, 0, 0],
            [1, 0, 0, 0],
            [0, 1, 0, 0],
            [1, 0, 0, 0],
            [0, 0, 0, 0]
        ]
    );
    assert_eq!(part.weights.as_deref().unwrap()[3], [0.5, 0.5, 0.0, 0.0]);
}

#[test]
fn smooth_curves_and_packed_rotations_are_read() {
    let scene = changed(|file, json| {
        // Two keys of slope, value, slope; rotations as whole numbers out of 32767.
        file.floats("SCALAR", &[[0.25], [2.0]], "");
        let values: [[i16; 4]; 6] = [
            [0; 4],
            [0, 0, 0, 16000],
            [100, 0, 0, 0],
            [-100, 0, 0, 0],
            [32767, 0, 0, 0],
            [0; 4],
        ];
        let bytes: Vec<u8> = values
            .iter()
            .flatten()
            .flat_map(|v| v.to_le_bytes())
            .collect();
        file.list(&bytes, 6, I16, "VEC4", r#","normalized":true"#);
        swap(
            json,
            r#"{"input":6,"output":7}"#,
            r#"{"input":9,"output":10,"interpolation":"CUBICSPLINE"}"#,
        );
    })
    .unwrap();
    let clip = &scene.animations[0];
    assert_eq!(clip.duration, 2.0);
    let turn = &clip.tracks[0];
    assert_eq!(turn.interpolation, Interpolation::CubicSpline);
    assert_eq!(turn.times, [0.25, 2.0]);
    let Keys::Rotation(keys) = &turn.keys else {
        panic!("{:?}", turn.keys);
    };
    assert_eq!(keys.len(), 6);
    // Values are made unit length; slopes are left as they are.
    assert_eq!(keys[1], [0.0, 0.0, 0.0, 1.0]);
    assert_eq!(keys[4], [1.0, 0.0, 0.0, 0.0]);
    assert_eq!(keys[0], [0.0; 4]);
    assert!((keys[2][0] - 100.0 / 32767.0).abs() < 1e-7);
    assert!((keys[3][0] + 100.0 / 32767.0).abs() < 1e-7);
}

#[test]
fn tracks_that_cannot_be_used_are_left_out() {
    let scene = changed(|file, json| {
        file.floats("SCALAR", &[[0.0], [1.0], [0.5]], "");
        // Shape keys; two tracks for the tip's rotation; and an object that
        // is in the file but not in the scene.
        swap(
            json,
            BAR_CLIP,
            r#""animations":[{"name":"Bend",
            "samplers":[{"input":6,"output":7},{"input":6,"output":8},{"input":6,"output":9}],
            "channels":[{"sampler":2,"target":{"node":0,"path":"weights"}},
                {"sampler":0,"target":{"node":3,"path":"rotation"}},
                {"sampler":0,"target":{"node":3,"path":"rotation"}},
                {"sampler":1,"target":{"node":4,"path":"translation"}},
                {"sampler":1,"target":{"node":2,"path":"translation"}}]}]"#,
        );
        swap(
            json,
            r#""scale":[1,2,1]}]"#,
            r#""scale":[1,2,1]},{"name":"Spare"}]"#,
        );
    })
    .unwrap();
    let tracks = &scene.animations[0].tracks;
    let moved: Vec<_> = tracks.iter().map(|t| t.node).collect();
    assert_eq!(moved, [3, 2]);
    assert!(matches!(tracks[0].keys, Keys::Rotation(_)));
    assert!(
        scene.warnings.iter().any(|w| w.contains("shape keys")),
        "{:?}",
        scene.warnings
    );
}

#[test]
fn a_mesh_with_no_weights_on_a_skeleton_says_so() {
    let scene = changed(|_, json| swap(json, r#","JOINTS_0":3,"WEIGHTS_0":4"#, "")).unwrap();
    assert_eq!(scene.nodes[0].skin, None);
    assert!(scene.skins.is_empty());
    assert!(
        scene
            .warnings
            .iter()
            .any(|w| w.contains("\"Bar\"") && w.contains("automatic weights")),
        "{:?}",
        scene.warnings
    );

    // A skin nothing uses is not read, and an animation alone keeps the tree.
    let scene = changed(|_, json| swap(json, r#""mesh":0,"skin":0,"#, r#""mesh":0,"#)).unwrap();
    assert!(scene.skins.is_empty());
    assert_eq!(scene.nodes.len(), 4);
    assert_eq!(scene.animations[0].tracks.len(), 2);
    // The weights are still there for whoever places the mesh on a skeleton.
    assert!(scene.meshes[0].parts[0].joints.is_some());
}

#[test]
fn two_objects_share_one_skeleton() {
    let scene = changed(|_, json| {
        swap(
            json,
            r#""scale":[1,2,1]}]"#,
            r#""scale":[1,2,1]},{"name":"Copy","mesh":0,"skin":0}]"#,
        );
        swap(
            json,
            r#""scenes":[{"nodes":[0,1]}]"#,
            r#""scenes":[{"nodes":[0,1,4]}]"#,
        );
    })
    .unwrap();
    assert_eq!(scene.skins.len(), 1);
    // Top-level objects come first, so the copy is the third node.
    assert_eq!(scene.nodes[2].name.as_deref(), Some("Copy"));
    assert_eq!(
        (scene.nodes[2].instance, scene.nodes[2].skin),
        (Some(1), Some(0))
    );
    let joints: Vec<_> = scene.skins[0].joints.iter().map(|j| j.node).collect();
    assert_eq!(joints, [3, 4]);
}

#[test]
fn damaged_skins_are_refused_without_crashing() {
    let whole = bar().glb(&bar_json());
    for length in 0..whole.len() {
        assert!(read(&whole[..length]).is_err(), "cut to {length} bytes");
    }

    // A point moved by a bone the skeleton does not have.
    let message = damage(|file, _| file.bin[4 * 12 + 12 + 4 * 12] = 2);
    assert!(
        message.contains("\"Bar\"")
            && message.contains("bone number 3")
            && message.contains("\"Rig\""),
        "{message}"
    );
    // A bone that is not in the scene.
    let message = damage(|_, json| {
        swap(
            json,
            r#""scale":[1,2,1]}]"#,
            r#""scale":[1,2,1]},{"name":"Lost"}]"#,
        );
        swap(json, r#""joints":[2,3]"#, r#""joints":[2,4]"#);
    });
    assert!(message.contains("\"Lost\""), "{message}");
    // Bind poses: too few, the wrong kind of data, not numbers.
    damage(|file, _| file.accessors[5] = file.accessors[5].replace(r#""count":2"#, r#""count":1"#));
    damage(|file, _| {
        file.accessors[5] = file.accessors[5]
            .replace(r#""count":2"#, r#""count":8"#)
            .replace("MAT4", "VEC4");
    });
    damage(|file, _| {
        let start = file.bin.len() - 3 * 4 - 3 * 16 - 3 * 12 - 16;
        file.bin[start..start + 4].copy_from_slice(&f32::NAN.to_le_bytes());
    });
    // Numbers without weights, and data of the wrong kind.
    damage(|_, json| swap(json, r#","WEIGHTS_0":4"#, ""));
    damage(|_, json| {
        swap(
            json,
            r#""JOINTS_0":3,"WEIGHTS_0":4"#,
            r#""JOINTS_0":4,"WEIGHTS_0":4"#,
        )
    });
    damage(|_, json| {
        swap(
            json,
            r#""JOINTS_0":3,"WEIGHTS_0":4"#,
            r#""JOINTS_0":3,"WEIGHTS_0":2"#,
        )
    });
    // Weights below zero or not numbers.
    for bad in [-0.5f32, f32::NAN, f32::INFINITY] {
        let message = damage(|file, _| {
            let start = 4 * 12 + 12 + 4 * 12 + 16;
            file.bin[start..start + 4].copy_from_slice(&bad.to_le_bytes());
        });
        assert!(message.contains("bone weight"), "{message}");
    }

    // Too many bones says how to export fewer.
    let error = changed(|file, json| {
        let count = MAX_JOINTS + 1;
        file.floats("MAT4", &vec![moved([0.0; 3]); count], "");
        let joints = vec!["2"; count].join(",");
        swap(
            json,
            r#""joints":[2,3],"inverseBindMatrices":5"#,
            &format!(r#""joints":[{joints}],"inverseBindMatrices":9"#),
        );
    })
    .unwrap_err();
    assert!(
        matches!(error, AssetError::TooManyJoints { joints: 257, .. }),
        "{error}"
    );
    let message = error.to_string();
    assert!(
        message.contains("\"Rig\"")
            && message.contains("256")
            && message.contains("Deformation Bones Only"),
        "{message}"
    );
}

#[test]
fn damaged_animations_are_refused_without_crashing() {
    let times = 4 * 12 + 12 + 4 * 12 + 16 + 4 * 16 + 2 * 64;
    let set_time = |file: &mut Builder, key: usize, time: f32| {
        file.bin[times + key * 4..times + key * 4 + 4].copy_from_slice(&time.to_le_bytes());
    };
    // Times that go backwards, repeat, start before zero or are not numbers.
    for (key, time) in [
        (1, 2.0),
        (1, 1.0),
        (0, -1.0),
        (2, f32::NAN),
        (2, f32::INFINITY),
    ] {
        let message = damage(|file, _| set_time(file, key, time));
        assert!(
            message.contains("\"Bend\"") && message.contains("key times"),
            "{message}"
        );
    }
    // Times or values stored as the wrong kind of data.
    damage(|_, json| {
        swap(
            json,
            r#"{"input":6,"output":7}"#,
            r#"{"input":8,"output":7}"#,
        )
    });
    damage(|_, json| {
        swap(
            json,
            r#"{"input":6,"output":7}"#,
            r#"{"input":1,"output":7}"#,
        )
    });
    damage(|_, json| {
        swap(
            json,
            r#"{"input":6,"output":7}"#,
            r#"{"input":6,"output":8}"#,
        )
    });
    damage(|_, json| {
        swap(
            json,
            r#"{"input":6,"output":8,"#,
            r#"{"input":6,"output":7,"#,
        )
    });
    damage(|_, json| {
        swap(
            json,
            r#"{"input":6,"output":8,"#,
            r#"{"input":6,"output":1,"#,
        )
    });
    // A different number of values than times.
    let message = damage(|file, _| {
        file.accessors[7] = file.accessors[7].replace(r#""count":3"#, r#""count":2"#);
    });
    assert!(message.contains("3 key times but 2 values"), "{message}");
    // A smooth curve needs three values for each time, and two times.
    damage(|_, json| {
        swap(
            json,
            r#""interpolation":"STEP""#,
            r#""interpolation":"CUBICSPLINE""#,
        )
    });
    damage(|file, json| {
        file.accessors[6] = file.accessors[6].replace(r#""count":3"#, r#""count":1"#);
        swap(
            json,
            r#"{"input":6,"output":7}"#,
            r#"{"input":6,"output":7,"interpolation":"CUBICSPLINE"}"#,
        );
    });
    // A rotation of all zeros, and a place that is not a number.
    let rotations = times + 3 * 4;
    let message = damage(|file, _| file.bin[rotations + 32..rotations + 48].fill(0));
    assert!(message.contains("rotation"), "{message}");
    damage(|file, _| {
        file.bin[rotations + 48..rotations + 52].copy_from_slice(&f32::NAN.to_le_bytes());
    });
}

#[test]
fn a_small_file_cannot_ask_for_endless_animation() {
    // Many tracks share one long list of keys: a megabyte or two on disk
    // that would unpack to far more than the limit.
    const KEYS: usize = 100_000;
    const NODES: usize = 140;
    let mut file = Builder::default();
    let times: Vec<[f32; 1]> = (0..KEYS).map(|key| [key as f32]).collect();
    file.floats("SCALAR", &times, "");
    file.floats("VEC4", &vec![[0.0, 0.0, 0.0, 1.0]; KEYS], "");
    let nodes = vec!["{}"; NODES].join(",");
    let all: Vec<String> = (0..NODES).map(|n| n.to_string()).collect();
    let channels: Vec<String> = (0..NODES)
        .map(|n| format!(r#"{{"sampler":0,"target":{{"node":{n},"path":"rotation"}}}}"#))
        .collect();
    let json = format!(
        r#""nodes":[{nodes}],"scenes":[{{"nodes":[{}]}}],
        "animations":[{{"samplers":[{{"input":0,"output":1}}],"channels":[{}]}}]"#,
        all.join(","),
        channels.join(",")
    );
    let error = read(&file.glb(&json)).unwrap_err();
    assert!(
        matches!(error, AssetError::TooMuchAnimation { .. }),
        "{error}"
    );
    let message = error.to_string();
    assert!(
        message.contains("level.glb") && message.contains("256 MiB"),
        "{message}"
    );
}
