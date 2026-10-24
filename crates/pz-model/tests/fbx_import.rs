//! Synthetic FBX files built in the test (one node tree written both as
//! binary 7.4 and ASCII 7.4) run through the FBX mesh path.

use pz_model::import::{self, Format, Steps};
use pz_model::loader::{load_mesh, MeshParams};

#[derive(Clone)]
enum P {
    I(i32),
    L(i64),
    D(f64),
    S(String),
    Ad(Vec<f64>),
    Ai(Vec<i32>),
}

struct N {
    name: &'static str,
    props: Vec<P>,
    children: Vec<N>,
}

fn n(name: &'static str, props: Vec<P>, children: Vec<N>) -> N {
    N {
        name,
        props,
        children,
    }
}

fn s(v: &str) -> P {
    P::S(v.to_string())
}

/// `P: "name", "type", "", "", values...`
fn prop(name: &str, ty: &str, vals: Vec<P>) -> N {
    let mut p = vec![s(name), s(ty), s(""), s("")];
    p.extend(vals);
    n("P", p, vec![])
}

fn tree() -> Vec<N> {
    let model_id = 1000i64;
    let geom_id = 2000i64;
    let mat_id = 3000i64;
    vec![
        n(
            "FBXHeaderExtension",
            vec![],
            vec![n("FBXVersion", vec![P::I(7400)], vec![])],
        ),
        n(
            "GlobalSettings",
            vec![],
            vec![n(
                "Properties70",
                vec![],
                vec![prop("UnitScaleFactor", "double", vec![P::D(1.0)])],
            )],
        ),
        n(
            "Objects",
            vec![],
            vec![
                n(
                    "Geometry",
                    vec![P::L(geom_id), s("Quad\u{0}\u{1}Geometry"), s("Mesh")],
                    vec![
                        n(
                            "Vertices",
                            vec![P::Ad(vec![0., 0., 0., 1., 0., 0., 1., 1., 0., 0., 1., 0.])],
                            vec![],
                        ),
                        n("PolygonVertexIndex", vec![P::Ai(vec![0, 1, 2, -4])], vec![]),
                        n(
                            "LayerElementNormal",
                            vec![P::I(0)],
                            vec![
                                n("MappingInformationType", vec![s("ByPolygonVertex")], vec![]),
                                n("ReferenceInformationType", vec![s("Direct")], vec![]),
                                n("Normals", vec![P::Ad([0., 0., 1.].repeat(4))], vec![]),
                            ],
                        ),
                        n(
                            "LayerElementUV",
                            vec![P::I(0)],
                            vec![
                                n("Name", vec![s("UVMap")], vec![]),
                                n("MappingInformationType", vec![s("ByPolygonVertex")], vec![]),
                                n("ReferenceInformationType", vec![s("IndexToDirect")], vec![]),
                                n(
                                    "UV",
                                    vec![P::Ad(vec![0., 0., 1., 0., 1., 1., 0., 1.])],
                                    vec![],
                                ),
                                n("UVIndex", vec![P::Ai(vec![0, 1, 2, 3])], vec![]),
                            ],
                        ),
                        n(
                            "LayerElementMaterial",
                            vec![P::I(0)],
                            vec![
                                n("MappingInformationType", vec![s("AllSame")], vec![]),
                                n("ReferenceInformationType", vec![s("IndexToDirect")], vec![]),
                                n("Materials", vec![P::Ai(vec![0])], vec![]),
                            ],
                        ),
                        n(
                            "Layer",
                            vec![P::I(0)],
                            vec![
                                n(
                                    "LayerElement",
                                    vec![],
                                    vec![
                                        n("Type", vec![s("LayerElementNormal")], vec![]),
                                        n("TypedIndex", vec![P::I(0)], vec![]),
                                    ],
                                ),
                                n(
                                    "LayerElement",
                                    vec![],
                                    vec![
                                        n("Type", vec![s("LayerElementUV")], vec![]),
                                        n("TypedIndex", vec![P::I(0)], vec![]),
                                    ],
                                ),
                                n(
                                    "LayerElement",
                                    vec![],
                                    vec![
                                        n("Type", vec![s("LayerElementMaterial")], vec![]),
                                        n("TypedIndex", vec![P::I(0)], vec![]),
                                    ],
                                ),
                            ],
                        ),
                    ],
                ),
                n(
                    "Model",
                    vec![P::L(model_id), s("Quad\u{0}\u{1}Model"), s("Mesh")],
                    vec![n(
                        "Properties70",
                        vec![],
                        vec![
                            prop(
                                "PreRotation",
                                "Vector3D",
                                vec![P::D(-90.0), P::D(0.0), P::D(0.0)],
                            ),
                            prop(
                                "Lcl Translation",
                                "Lcl Translation",
                                vec![P::D(0.0), P::D(2.0), P::D(0.0)],
                            ),
                            prop(
                                "Lcl Scaling",
                                "Lcl Scaling",
                                vec![P::D(0.5), P::D(0.5), P::D(0.5)],
                            ),
                        ],
                    )],
                ),
                n(
                    "Material",
                    vec![P::L(mat_id), s("Skin\u{0}\u{1}Material"), s("")],
                    vec![n("ShadingModel", vec![s("lambert")], vec![])],
                ),
            ],
        ),
        n(
            "Connections",
            vec![],
            vec![
                n("C", vec![s("OO"), P::L(model_id), P::L(0)], vec![]),
                n("C", vec![s("OO"), P::L(geom_id), P::L(model_id)], vec![]),
                n("C", vec![s("OO"), P::L(mat_id), P::L(model_id)], vec![]),
            ],
        ),
    ]
}

fn write_binary(nodes: &[N]) -> Vec<u8> {
    fn node(out: &mut Vec<u8>, nd: &N) {
        let start = out.len();
        out.extend_from_slice(&[0u8; 12]);
        out.push(nd.name.len() as u8);
        out.extend_from_slice(nd.name.as_bytes());
        let pstart = out.len();
        for p in &nd.props {
            match p {
                P::I(v) => {
                    out.push(b'I');
                    out.extend_from_slice(&v.to_le_bytes());
                }
                P::L(v) => {
                    out.push(b'L');
                    out.extend_from_slice(&v.to_le_bytes());
                }
                P::D(v) => {
                    out.push(b'D');
                    out.extend_from_slice(&v.to_le_bytes());
                }
                P::S(v) => {
                    out.push(b'S');
                    out.extend_from_slice(&(v.len() as u32).to_le_bytes());
                    out.extend_from_slice(v.as_bytes());
                }
                P::Ad(v) => {
                    out.push(b'd');
                    out.extend_from_slice(&(v.len() as u32).to_le_bytes());
                    out.extend_from_slice(&0u32.to_le_bytes());
                    out.extend_from_slice(&((v.len() * 8) as u32).to_le_bytes());
                    for x in v {
                        out.extend_from_slice(&x.to_le_bytes());
                    }
                }
                P::Ai(v) => {
                    out.push(b'i');
                    out.extend_from_slice(&(v.len() as u32).to_le_bytes());
                    out.extend_from_slice(&0u32.to_le_bytes());
                    out.extend_from_slice(&((v.len() * 4) as u32).to_le_bytes());
                    for x in v {
                        out.extend_from_slice(&x.to_le_bytes());
                    }
                }
            }
        }
        let plen = (out.len() - pstart) as u32;
        if !nd.children.is_empty() {
            for c in &nd.children {
                node(out, c);
            }
            out.extend_from_slice(&[0u8; 13]);
        }
        let end = out.len() as u32;
        out[start..start + 4].copy_from_slice(&end.to_le_bytes());
        out[start + 4..start + 8].copy_from_slice(&(nd.props.len() as u32).to_le_bytes());
        out[start + 8..start + 12].copy_from_slice(&plen.to_le_bytes());
    }
    let mut out = b"Kaydara FBX Binary  \x00\x1a\x00".to_vec();
    out.extend_from_slice(&7400u32.to_le_bytes());
    for nd in nodes {
        node(&mut out, nd);
    }
    out.extend_from_slice(&[0u8; 13]);
    out
}

fn write_ascii(nodes: &[N]) -> String {
    fn val(p: &P) -> String {
        match p {
            P::I(v) => v.to_string(),
            P::L(v) => v.to_string(),
            P::D(v) => format!("{v:?}"),
            P::S(v) => {
                // binary "Name\0\1Class" becomes "Class::Name" in ASCII
                let v = match v.split_once("\u{0}\u{1}") {
                    Some((a, b)) => format!("{b}::{a}"),
                    None => v.clone(),
                };
                format!("\"{v}\"")
            }
            _ => unreachable!(),
        }
    }
    fn node(out: &mut String, nd: &N, depth: usize) {
        let ind = "\t".repeat(depth);
        out.push_str(&format!("{ind}{}: ", nd.name));
        let arr = nd.props.iter().find_map(|p| match p {
            P::Ad(v) => Some(v.iter().map(|x| format!("{x:?}")).collect::<Vec<_>>()),
            P::Ai(v) => Some(v.iter().map(|x| x.to_string()).collect::<Vec<_>>()),
            _ => None,
        });
        if let Some(a) = arr {
            out.push_str(&format!(
                "*{} {{\n{ind}\ta: {}\n{ind}}}\n",
                a.len(),
                a.join(",")
            ));
            return;
        }
        out.push_str(&nd.props.iter().map(val).collect::<Vec<_>>().join(", "));
        if nd.children.is_empty() {
            out.push('\n');
        } else {
            out.push_str(" {\n");
            for c in &nd.children {
                node(out, c, depth + 1);
            }
            out.push_str(&format!("{ind}}}\n"));
        }
    }
    let mut out = String::from("; FBX 7.4.0 project file\n");
    for nd in nodes {
        node(&mut out, nd, 0);
    }
    out
}

fn check(data: &[u8]) {
    let scene = import::import(data, Format::Fbx, Steps::mesh_default()).unwrap();
    let root = &scene.nodes[scene.root];
    assert_eq!(root.name, "RootNode");
    // PreRotation needs the full pivot chain.
    let names: Vec<&str> = scene
        .preorder()
        .iter()
        .map(|&i| scene.nodes[i].name.as_str())
        .collect();
    assert_eq!(
        names,
        [
            "RootNode",
            "Quad_$AssimpFbx$_Translation",
            "Quad_$AssimpFbx$_PreRotation",
            "Quad_$AssimpFbx$_Scaling",
            "Quad"
        ]
    );
    assert_eq!(scene.meshes.len(), 1);
    assert_eq!(scene.meshes[0].name, "Quad");

    let p = load_mesh(
        data,
        Format::Fbx,
        &MeshParams {
            is_static: true,
            ..Default::default()
        },
    )
    .unwrap();
    let m = &p.static_meshes[0];
    assert_eq!(m.num_uvs, 1);
    // The quad is triangulated (fanned from the first corner) and its four
    // corners kept after joining.
    assert_eq!(m.vertices.len(), 4 * 8);
    assert_eq!(m.elements.len(), 6);
    let t = m.transform.expect("mesh transform");
    // ProcessedAiScene.initMeshTransform multiplies with LWJGL's
    // Matrix4f.mul(parent, node), which composes child-first: the chain's
    // translation (0, 2, 0) comes out rotated by PreRotation (-90 deg about
    // x, mirrored by MAKE_LEFT_HANDED) and scaled by 0.5.
    assert!(
        (t[3][2] - 1.0).abs() < 1e-6 && t[3][1].abs() < 1e-6,
        "{t:?}"
    );
    // Scale 0.5 on x.
    assert!((t[0][0] - 0.5).abs() < 1e-6, "{t:?}");
    assert_eq!(m.min_xyz[0], 0.0);
    assert_eq!(m.max_xyz[0], 1.0);
}

#[test]
fn binary_fbx_quad() {
    check(&write_binary(&tree()));
}

#[test]
fn ascii_fbx_quad() {
    check(write_ascii(&tree()).as_bytes());
}
