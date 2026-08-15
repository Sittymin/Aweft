use super::*;
use proptest::prelude::*;

// ── proptest ───────────────────────────────────────────────────────────────────

proptest! {
    #[test]
    fn random_dag_matches_reference_model(
        plan in prop::collection::vec(
            (0u8..4, any::<u16>(), any::<u16>(), any::<bool>(), any::<i16>(), any::<i16>()),
            1..40,
        )
    ) {
        let mut graph = NodeGraph::new("test-dag");

        let src: NodeHandle<StyleSource> =
            graph.add_node(NodeKind::StyleSource, &[], &[]).unwrap();
        let source = src.outputs.style.id;
        let mut outputs = vec![source];
        let mut expected = vec![StyleResult::default()];

        for (operation, parent_a, parent_b, boolean, raw_x, raw_y) in plan {
            let a = parent_a as usize % outputs.len();
            let b = parent_b as usize % outputs.len();
            let x = f64::from(raw_x) / 16.0;
            let y = f64::from(raw_y) / 16.0;

            let (output, result) = match operation {
                0 => {
                    let h: NodeHandle<SetFontSize> = graph
                        .add_node(NodeKind::StyleOp(StyleOpKind::SetFontSize), &[], &[])
                        .unwrap();
                    graph.set_value(h.inputs.size, x).unwrap();
                    graph.connect_raw(outputs[a], h.inputs.style.id).unwrap();
                    let mut r = expected[a].clone();
                    r.font_size = Some(x);
                    (h.outputs.style.id, r)
                }
                1 => {
                    let h: NodeHandle<SetBold> = graph
                        .add_node(NodeKind::StyleOp(StyleOpKind::SetBold), &[], &[])
                        .unwrap();
                    graph.set_value(h.inputs.bold, boolean).unwrap();
                    graph.connect_raw(outputs[a], h.inputs.style.id).unwrap();
                    let mut r = expected[a].clone();
                    r.bold = Some(boolean);
                    (h.outputs.style.id, r)
                }
                2 => {
                    let combine: NodeHandle<CombineVec2> = graph
                        .add_node(NodeKind::VecOp(VecOpKind::CombineVec2), &[], &[])
                        .unwrap();
                    graph.set_value(combine.inputs.x, x).unwrap();
                    graph.set_value(combine.inputs.y, y).unwrap();

                    let h: NodeHandle<SetPosition> = graph
                        .add_node(NodeKind::StyleOp(StyleOpKind::SetPosition), &[], &[])
                        .unwrap();
                    graph.connect_raw(outputs[a], h.inputs.style.id).unwrap();
                    graph.connect_raw(combine.outputs.vector.id, h.inputs.position.id).unwrap();

                    let mut r = expected[a].clone();
                    r.position = Some(Vec2 { x, y });
                    (h.outputs.style.id, r)
                }
                _ => {
                    let nid = graph
                        .add_node::<NodeId>(
                            NodeKind::MergeStyle,
                            &[SocketDecl::input("Style 1", SocketType::Style)],
                            &[],
                        )
                        .unwrap();
                    let a_in = graph.nodes[&nid].inputs[0];
                    let b_in = graph.nodes[&nid].inputs[1];
                    let out = graph.nodes[&nid].outputs[0];
                    graph.connect_raw(outputs[a], a_in).unwrap();
                    graph.connect_raw(outputs[b], b_in).unwrap();

                    let r = expected[a].clone().merge_over(expected[b].clone());
                    (out, r)
                }
            };
            outputs.push(output);
            expected.push(result);
        }

        let h: NodeHandle<GroupOutput> =
            graph.add_node(NodeKind::GroupOutput, &[], &[]).unwrap();
        graph.connect_raw(*outputs.last().unwrap(), h.inputs.style.id).unwrap();

        let expected = expected.last().unwrap().clone();
        prop_assert_eq!(graph.evaluate(&EvalContext::style()).unwrap(), expected);
    }
}

// ── typed API tests ────────────────────────────────────────────────────────────

#[test]
fn typed_api_basic() {
    let mut graph = NodeGraph::new("typed-api-test");

    let source: NodeHandle<StyleSource> = graph.add_node(NodeKind::StyleSource, &[], &[]).unwrap();
    let size: NodeHandle<SetFontSize> = graph
        .add_node(NodeKind::StyleOp(StyleOpKind::SetFontSize), &[], &[])
        .unwrap();
    let output: NodeHandle<GroupOutput> = graph.add_node(NodeKind::GroupOutput, &[], &[]).unwrap();

    graph
        .connect(source.outputs.style, size.inputs.style)
        .unwrap();
    graph
        .connect(size.outputs.style, output.inputs.style)
        .unwrap();
    graph.set_value(size.inputs.size, 42.0).unwrap();

    let result = graph.evaluate(&EvalContext::style()).unwrap();
    assert_eq!(result.font_size, Some(42.0));
}

#[test]
fn typed_api_vec2_chain() {
    let mut graph = NodeGraph::new("typed-vec2-test");

    let source: NodeHandle<StyleSource> = graph.add_node(NodeKind::StyleSource, &[], &[]).unwrap();
    let combine: NodeHandle<CombineVec2> = graph
        .add_node(NodeKind::VecOp(VecOpKind::CombineVec2), &[], &[])
        .unwrap();
    let position: NodeHandle<SetPosition> = graph
        .add_node(NodeKind::StyleOp(StyleOpKind::SetPosition), &[], &[])
        .unwrap();
    let output: NodeHandle<GroupOutput> = graph.add_node(NodeKind::GroupOutput, &[], &[]).unwrap();

    graph
        .connect(source.outputs.style, position.inputs.style)
        .unwrap();
    graph
        .connect(combine.outputs.vector, position.inputs.position)
        .unwrap();
    graph
        .connect(position.outputs.style, output.inputs.style)
        .unwrap();
    graph.set_value(combine.inputs.x, 1.0).unwrap();
    graph.set_value(combine.inputs.y, 2.0).unwrap();

    let result = graph.evaluate(&EvalContext::style()).unwrap();
    assert_eq!(result.position, Some(Vec2 { x: 1.0, y: 2.0 }));
}

#[test]
fn clip_rect_is_normalized() {
    let mut graph = NodeGraph::new("clip-rect-test");

    let source: NodeHandle<StyleSource> = graph.add_node(NodeKind::StyleSource, &[], &[]).unwrap();
    let clip: NodeHandle<SetClipRect> = graph
        .add_node(NodeKind::StyleOp(StyleOpKind::SetClipRect), &[], &[])
        .unwrap();
    let output: NodeHandle<GroupOutput> = graph.add_node(NodeKind::GroupOutput, &[], &[]).unwrap();

    graph
        .connect(source.outputs.style, clip.inputs.style)
        .unwrap();
    graph
        .connect(clip.outputs.style, output.inputs.style)
        .unwrap();
    graph
        .set_value(
            clip.inputs.clip_rect,
            ClipRect {
                x1: 10.0,
                y1: 20.0,
                x2: -5.0,
                y2: -8.0,
            },
        )
        .unwrap();

    let result = graph.evaluate(&EvalContext::style()).unwrap();
    assert_eq!(
        result.clip_rect,
        Some(ClipRect {
            x1: -5.0,
            y1: -8.0,
            x2: 10.0,
            y2: 20.0,
        })
    );
}

proptest! {
    #[test]
    fn separate_then_combine_vec3_round_trips(x: i16, y: i16, z: i16) {
        let (x, y, z) = (f64::from(x), f64::from(y), f64::from(z));
        let mut graph = NodeGraph::new("vec3-roundtrip");

        let source: NodeHandle<StyleSource> =
            graph.add_node(NodeKind::StyleSource, &[], &[]).unwrap();
        let sep: NodeHandle<SeparateVec3> = graph
            .add_node(NodeKind::VecOp(VecOpKind::SeparateVec3), &[], &[])
            .unwrap();
        let comb: NodeHandle<CombineVec3> = graph
            .add_node(NodeKind::VecOp(VecOpKind::CombineVec3), &[], &[])
            .unwrap();
        let rot: NodeHandle<SetFontRotation> = graph
            .add_node(NodeKind::StyleOp(StyleOpKind::SetFontRotation), &[], &[])
            .unwrap();
        let output: NodeHandle<GroupOutput> =
            graph.add_node(NodeKind::GroupOutput, &[], &[]).unwrap();

        graph.set_socket_value(sep.inputs.vector.id, Some(SocketValue::Vec3 { x, y, z })).unwrap();
        graph.connect(sep.outputs.x, comb.inputs.x).unwrap();
        graph.connect(sep.outputs.y, comb.inputs.y).unwrap();
        graph.connect(sep.outputs.z, comb.inputs.z).unwrap();
        graph.connect(source.outputs.style, rot.inputs.style).unwrap();
        graph.connect(comb.outputs.vector, rot.inputs.rotation).unwrap();
        graph.connect(rot.outputs.style, output.inputs.style).unwrap();

        let result = graph.evaluate(&EvalContext::style()).unwrap();
        prop_assert_eq!(result.font_rotation, Some(Vec3 { x, y, z }));
    }
}

// ── dirty tracking tests ───────────────────────────────────────────────────────

#[test]
fn incremental_evaluation_skips_clean_nodes() {
    let mut graph = NodeGraph::new("incremental-test");

    let source: NodeHandle<StyleSource> = graph.add_node(NodeKind::StyleSource, &[], &[]).unwrap();
    let size: NodeHandle<SetFontSize> = graph
        .add_node(NodeKind::StyleOp(StyleOpKind::SetFontSize), &[], &[])
        .unwrap();
    let bold: NodeHandle<SetBold> = graph
        .add_node(NodeKind::StyleOp(StyleOpKind::SetBold), &[], &[])
        .unwrap();
    let output: NodeHandle<GroupOutput> = graph.add_node(NodeKind::GroupOutput, &[], &[]).unwrap();

    graph
        .connect(source.outputs.style, size.inputs.style)
        .unwrap();
    graph
        .connect(size.outputs.style, bold.inputs.style)
        .unwrap();
    graph
        .connect(bold.outputs.style, output.inputs.style)
        .unwrap();
    graph.set_value(size.inputs.size, 10.0).unwrap();
    graph.set_value(bold.inputs.bold, true).unwrap();

    let result1 = graph.evaluate(&EvalContext::style()).unwrap();
    assert_eq!(result1.font_size, Some(10.0));
    assert_eq!(result1.bold, Some(true));
    assert!(graph.dirty.is_empty());

    let bold_input = graph.nodes[&bold.id].inputs[1];
    graph
        .set_socket_value(bold_input, Some(SocketValue::Bool(false)))
        .unwrap();

    assert!(graph.dirty.contains(&bold.id));

    let result2 = graph.evaluate(&EvalContext::style()).unwrap();
    assert_eq!(result2.font_size, Some(10.0));
    assert_eq!(result2.bold, Some(false));
    assert!(graph.dirty.is_empty());
}

#[test]
fn add_node_only_marks_new_node_dirty() {
    let mut graph = NodeGraph::new("dirty-add-node");

    let source: NodeHandle<StyleSource> = graph.add_node(NodeKind::StyleSource, &[], &[]).unwrap();
    let size: NodeHandle<SetFontSize> = graph
        .add_node(NodeKind::StyleOp(StyleOpKind::SetFontSize), &[], &[])
        .unwrap();
    let output: NodeHandle<GroupOutput> = graph.add_node(NodeKind::GroupOutput, &[], &[]).unwrap();

    graph
        .connect(source.outputs.style, size.inputs.style)
        .unwrap();
    graph
        .connect(size.outputs.style, output.inputs.style)
        .unwrap();
    graph.set_value(size.inputs.size, 20.0).unwrap();

    let _ = graph.evaluate(&EvalContext::style()).unwrap();
    assert!(graph.cache.is_some());
    assert!(graph.dirty.is_empty());

    let new_node = graph
        .add_node::<NodeId>(NodeKind::StyleOp(StyleOpKind::SetBold), &[], &[])
        .unwrap();

    assert!(graph.cache.is_some());
    assert_eq!(graph.dirty.len(), 1);
    assert!(graph.dirty.contains(&new_node));
}

#[test]
fn add_link_marks_destination_and_downstream_dirty() {
    let mut graph = NodeGraph::new("dirty-link");

    let source: NodeHandle<StyleSource> = graph.add_node(NodeKind::StyleSource, &[], &[]).unwrap();
    let size: NodeHandle<SetFontSize> = graph
        .add_node(NodeKind::StyleOp(StyleOpKind::SetFontSize), &[], &[])
        .unwrap();
    let bold: NodeHandle<SetBold> = graph
        .add_node(NodeKind::StyleOp(StyleOpKind::SetBold), &[], &[])
        .unwrap();
    let output: NodeHandle<GroupOutput> = graph.add_node(NodeKind::GroupOutput, &[], &[]).unwrap();

    graph
        .connect(source.outputs.style, size.inputs.style)
        .unwrap();
    graph
        .connect(size.outputs.style, bold.inputs.style)
        .unwrap();
    graph
        .connect(bold.outputs.style, output.inputs.style)
        .unwrap();
    graph.set_value(size.inputs.size, 20.0).unwrap();
    graph.set_value(bold.inputs.bold, true).unwrap();

    let _ = graph.evaluate(&EvalContext::style()).unwrap();
    assert!(graph.dirty.is_empty());

    let bold_node = graph
        .add_node::<NodeId>(NodeKind::StyleOp(StyleOpKind::SetBold), &[], &[])
        .unwrap();
    let bold_in = graph.nodes[&bold_node].inputs[0];
    let bold_value_in = graph.nodes[&bold_node].inputs[1];
    graph.connect_raw(size.outputs.style.id, bold_in).unwrap();
    graph
        .set_socket_value(bold_value_in, Some(SocketValue::Bool(false)))
        .unwrap();

    assert!(graph.dirty.contains(&bold_node));
    assert!(!graph.dirty.contains(&size.id));
}

#[test]
fn disconnect_input_marks_destination_dirty() {
    let mut graph = NodeGraph::new("dirty-unlink-input");

    let source: NodeHandle<StyleSource> = graph.add_node(NodeKind::StyleSource, &[], &[]).unwrap();
    let size: NodeHandle<SetFontSize> = graph
        .add_node(NodeKind::StyleOp(StyleOpKind::SetFontSize), &[], &[])
        .unwrap();
    let bold: NodeHandle<SetBold> = graph
        .add_node(NodeKind::StyleOp(StyleOpKind::SetBold), &[], &[])
        .unwrap();
    let output: NodeHandle<GroupOutput> = graph.add_node(NodeKind::GroupOutput, &[], &[]).unwrap();

    graph
        .connect(source.outputs.style, size.inputs.style)
        .unwrap();
    graph
        .connect(size.outputs.style, bold.inputs.style)
        .unwrap();
    graph
        .connect(bold.outputs.style, output.inputs.style)
        .unwrap();
    graph.set_value(size.inputs.size, 20.0).unwrap();
    graph.set_value(bold.inputs.bold, true).unwrap();

    let _ = graph.evaluate(&EvalContext::style()).unwrap();
    assert!(graph.dirty.is_empty());

    let bold_style_in = graph.nodes[&bold.id].inputs[0];
    graph.disconnect_input(bold_style_in);

    assert!(!graph.links.iter().any(|l| l.to == bold_style_in));
    assert!(graph.dirty.contains(&bold.id));
    assert!(!graph.dirty.contains(&size.id));
}

#[test]
fn disconnect_output_marks_destinations_dirty() {
    let mut graph = NodeGraph::new("dirty-unlink-output");

    let source: NodeHandle<StyleSource> = graph.add_node(NodeKind::StyleSource, &[], &[]).unwrap();
    let size: NodeHandle<SetFontSize> = graph
        .add_node(NodeKind::StyleOp(StyleOpKind::SetFontSize), &[], &[])
        .unwrap();
    let bold1: NodeHandle<SetBold> = graph
        .add_node(NodeKind::StyleOp(StyleOpKind::SetBold), &[], &[])
        .unwrap();
    let bold2: NodeHandle<SetBold> = graph
        .add_node(NodeKind::StyleOp(StyleOpKind::SetBold), &[], &[])
        .unwrap();
    let output: NodeHandle<GroupOutput> = graph.add_node(NodeKind::GroupOutput, &[], &[]).unwrap();

    graph
        .connect(source.outputs.style, size.inputs.style)
        .unwrap();
    graph
        .connect(size.outputs.style, bold1.inputs.style)
        .unwrap();
    graph
        .connect(size.outputs.style, bold2.inputs.style)
        .unwrap();
    graph
        .connect(bold1.outputs.style, output.inputs.style)
        .unwrap();
    graph.set_value(size.inputs.size, 20.0).unwrap();
    graph.set_value(bold1.inputs.bold, true).unwrap();
    graph.set_value(bold2.inputs.bold, false).unwrap();

    let _ = graph.evaluate(&EvalContext::style()).unwrap();
    assert!(graph.dirty.is_empty());

    graph.disconnect_output(size.outputs.style.id);

    assert!(
        graph
            .output_targets(size.outputs.style.id)
            .unwrap_or(&[])
            .is_empty()
    );
    assert!(graph.dirty.contains(&bold1.id));
    assert!(graph.dirty.contains(&bold2.id));
    assert!(!graph.dirty.contains(&size.id));
}

#[test]
fn remove_node_removes_links_and_marks_downstream_dirty() {
    let mut graph = NodeGraph::new("dirty-remove-node");

    let source: NodeHandle<StyleSource> = graph.add_node(NodeKind::StyleSource, &[], &[]).unwrap();
    let size: NodeHandle<SetFontSize> = graph
        .add_node(NodeKind::StyleOp(StyleOpKind::SetFontSize), &[], &[])
        .unwrap();
    let bold: NodeHandle<SetBold> = graph
        .add_node(NodeKind::StyleOp(StyleOpKind::SetBold), &[], &[])
        .unwrap();
    let output: NodeHandle<GroupOutput> = graph.add_node(NodeKind::GroupOutput, &[], &[]).unwrap();

    graph
        .connect(source.outputs.style, size.inputs.style)
        .unwrap();
    graph
        .connect(size.outputs.style, bold.inputs.style)
        .unwrap();
    graph
        .connect(bold.outputs.style, output.inputs.style)
        .unwrap();
    graph.set_value(size.inputs.size, 20.0).unwrap();
    graph.set_value(bold.inputs.bold, true).unwrap();

    let result_before = graph.evaluate(&EvalContext::style()).unwrap();
    assert_eq!(result_before.font_size, Some(20.0));
    assert!(graph.dirty.is_empty());

    let size_out = size.outputs.style.id;
    graph.remove_node(size.id);

    assert!(!graph.nodes.contains_key(&size.id));
    for sid in graph.sockets.keys().copied().collect::<Vec<_>>() {
        assert_ne!(graph.sockets.get(&sid).unwrap().node, size.id);
    }
    assert!(
        !graph
            .links
            .iter()
            .any(|l| l.from == size_out || l.to == size_out)
    );

    let result_after = graph.evaluate(&EvalContext::style()).unwrap();
    assert_eq!(result_after.font_size, None);
}

#[test]
fn disconnect_removes_specific_link() {
    let mut graph = NodeGraph::new("remove-link");

    let source: NodeHandle<StyleSource> = graph.add_node(NodeKind::StyleSource, &[], &[]).unwrap();
    let size: NodeHandle<SetFontSize> = graph
        .add_node(NodeKind::StyleOp(StyleOpKind::SetFontSize), &[], &[])
        .unwrap();
    let bold: NodeHandle<SetBold> = graph
        .add_node(NodeKind::StyleOp(StyleOpKind::SetBold), &[], &[])
        .unwrap();
    let output: NodeHandle<GroupOutput> = graph.add_node(NodeKind::GroupOutput, &[], &[]).unwrap();

    graph
        .connect(source.outputs.style, size.inputs.style)
        .unwrap();
    graph
        .connect(size.outputs.style, bold.inputs.style)
        .unwrap();
    graph
        .connect(bold.outputs.style, output.inputs.style)
        .unwrap();
    graph.set_value(size.inputs.size, 20.0).unwrap();
    graph.set_value(bold.inputs.bold, true).unwrap();

    let _ = graph.evaluate(&EvalContext::style()).unwrap();

    let bold_style_in = graph.nodes[&bold.id].inputs[0];
    graph.disconnect(size.outputs.style.id, bold_style_in);

    assert!(graph.input_source(bold_style_in).is_none());
    assert!(graph.dirty.contains(&bold.id));
    assert!(!graph.dirty.contains(&size.id));
}

#[test]
fn viewer_takes_priority_over_group_output() {
    let mut graph = NodeGraph::new("viewer-priority");

    let source: NodeHandle<StyleSource> = graph.add_node(NodeKind::StyleSource, &[], &[]).unwrap();
    let size: NodeHandle<SetFontSize> = graph
        .add_node(NodeKind::StyleOp(StyleOpKind::SetFontSize), &[], &[])
        .unwrap();
    let bold: NodeHandle<SetBold> = graph
        .add_node(NodeKind::StyleOp(StyleOpKind::SetBold), &[], &[])
        .unwrap();
    let output: NodeHandle<GroupOutput> = graph.add_node(NodeKind::GroupOutput, &[], &[]).unwrap();

    graph
        .connect(source.outputs.style, size.inputs.style)
        .unwrap();
    graph
        .connect(size.outputs.style, bold.inputs.style)
        .unwrap();
    graph
        .connect(bold.outputs.style, output.inputs.style)
        .unwrap();
    graph.set_value(size.inputs.size, 20.0).unwrap();
    graph.set_value(bold.inputs.bold, true).unwrap();

    let viewer: NodeHandle<Viewer> = graph.add_node(NodeKind::Viewer, &[], &[]).unwrap();
    graph
        .connect_raw(size.outputs.style.id, viewer.inputs.style.id)
        .unwrap();

    let result = graph.evaluate(&EvalContext::style()).unwrap();
    assert_eq!(result.font_size, Some(20.0));
    assert_eq!(result.bold, None);
}

#[test]
fn evaluate_returns_default_when_no_output_or_viewer() {
    let mut graph = NodeGraph::new("no-output");

    let source: NodeHandle<StyleSource> = graph.add_node(NodeKind::StyleSource, &[], &[]).unwrap();
    let size: NodeHandle<SetFontSize> = graph
        .add_node(NodeKind::StyleOp(StyleOpKind::SetFontSize), &[], &[])
        .unwrap();

    graph
        .connect(source.outputs.style, size.inputs.style)
        .unwrap();
    graph.set_value(size.inputs.size, 20.0).unwrap();

    let result = graph.evaluate(&EvalContext::style()).unwrap();
    assert_eq!(result, StyleResult::default());
}

// ── error path tests ───────────────────────────────────────────────────────────

#[test]
fn duplicate_viewer_is_rejected() {
    let mut graph = NodeGraph::new("viewer-test");
    graph
        .add_node::<NodeId>(NodeKind::Viewer, &[], &[])
        .unwrap();
    let result = graph.add_node::<NodeId>(NodeKind::Viewer, &[], &[]);
    assert!(matches!(result, Err(EvalError::DuplicateViewer)));
}

#[test]
fn static_node_rejects_extra_sockets() {
    let mut graph = NodeGraph::new("static-extra-test");
    let result = graph.add_node::<NodeId>(
        NodeKind::StyleSource,
        &[SocketDecl::input("Extra", SocketType::Float)],
        &[],
    );
    assert!(matches!(result, Err(EvalError::StaticNodeExtras)));
}

#[test]
fn set_socket_value_rejects_type_mismatch() {
    let mut graph = NodeGraph::new("type-mismatch-test");
    let size = graph
        .add_node::<NodeId>(NodeKind::StyleOp(StyleOpKind::SetFontSize), &[], &[])
        .unwrap();
    let size_value_in = graph.nodes[&size].inputs[1];
    let result = graph.set_socket_value(size_value_in, Some(SocketValue::Bool(true)));
    assert!(matches!(
        result,
        Err(EvalError::TypeMismatch {
            expected: SocketType::Float,
            got: SocketType::Bool,
        })
    ));
}

#[test]
fn connect_raw_rejects_type_mismatch() {
    let mut graph = NodeGraph::new("link-type-mismatch-test");
    let size = graph
        .add_node::<NodeId>(NodeKind::StyleOp(StyleOpKind::SetFontSize), &[], &[])
        .unwrap();
    let vec = graph
        .add_node::<NodeId>(NodeKind::VecOp(VecOpKind::CombineVec2), &[], &[])
        .unwrap();
    let size_value_in = graph.nodes[&size].inputs[1];
    let vec_out = graph.nodes[&vec].outputs[0];
    let result = graph.connect_raw(vec_out, size_value_in);
    assert!(matches!(
        result,
        Err(EvalError::TypeMismatch {
            expected: SocketType::Vec2,
            got: SocketType::Float,
        })
    ));
}

// ── default socket tests ───────────────────────────────────────────────────────

#[test]
fn merge_style_has_default_input_and_output() {
    let mut graph = NodeGraph::new("merge-default-test");
    let merge = graph
        .add_node::<NodeId>(NodeKind::MergeStyle, &[], &[])
        .unwrap();
    let node = &graph.nodes[&merge];
    assert_eq!(node.inputs.len(), 1);
    assert_eq!(node.outputs.len(), 1);
    assert_eq!(graph.sockets[&node.inputs[0]].data_type, SocketType::Style);
    assert_eq!(graph.sockets[&node.outputs[0]].data_type, SocketType::Style);
}

#[test]
fn group_output_has_default_input() {
    let mut graph = NodeGraph::new("group-output-default-test");
    let output = graph
        .add_node::<NodeId>(NodeKind::GroupOutput, &[], &[])
        .unwrap();
    let node = &graph.nodes[&output];
    assert_eq!(node.inputs.len(), 1);
    assert_eq!(node.outputs.len(), 0);
    assert_eq!(graph.sockets[&node.inputs[0]].data_type, SocketType::Style);
}
