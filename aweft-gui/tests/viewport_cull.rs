// 视口裁剪回归测试：`if` 条件元素在 for 内惰性创建，
// 只有落在可见范围内的昂贵元素才会被实例化（巨量数据下避免 Slint 崩溃）。
// 完整机制说明见 graph_editor.slint 中 canvas.view-time-lo 处的详细注释。
use i_slint_backend_testing::{init_integration_test_with_system_time, ElementHandle};
use slint::ComponentHandle;

slint::slint! {
    export component Marker inherits Rectangle {
        width: 9px;
        height: 9px;
    }

    export component Culled inherits Window {
        in property <[float]> times: [0.0, 1.0, 2.0, 3.0, 4.0, 5.0];
        in property <float> view-lo: 0.0;
        in property <float> view-hi: 100.0;
        for t in root.times: Rectangle {
            width: 20px;
            height: 20px;
            if t >= root.view-lo && t <= root.view-hi: Marker {
                x: 0px;
                y: 0px;
            }
        }
    }

    export component SpanMarker inherits Rectangle {
        width: 5px;
        height: 5px;
    }

    // 线段/片断的裁剪条件：区间 [start,end] 与视口 [lo,hi] 相交才实例化
    export component SpanCulled inherits Window {
        in property <[float]> starts: [0.0, 4.0, 8.0];
        in property <[float]> ends: [2.0, 6.0, 10.0];
        in property <float> view-lo: 0.0;
        in property <float> view-hi: 100.0;
        for i in 3: Rectangle {
            width: 20px;
            height: 20px;
            if root.ends[i] >= root.view-lo && root.starts[i] <= root.view-hi: SpanMarker {
                x: 0px;
                y: 0px;
            }
        }
    }
}

#[test]
fn only_visible_markers_are_instantiated() {
    // 测试后端整个进程只能初始化一次，所有场景放同一个测试里。
    init_integration_test_with_system_time();

    let inst = Culled::new().unwrap();

    let all = ElementHandle::find_by_element_type_name(&inst, "Marker").count();
    assert_eq!(all, 6, "全可见时应创建全部 6 个标记");

    // 收窄视口：只显示 [1,2] 区间 → 只有 t=1,2 两个标记被实例化
    inst.set_view_lo(1.0);
    inst.set_view_hi(2.0);
    inst.window().request_redraw();
    let partial = ElementHandle::find_by_element_type_name(&inst, "Marker").count();
    assert_eq!(partial, 2, "视口裁剪后应只实例化可见的 2 个标记，得 {partial}");

    // 视口落在空隙（无标记）→ 0 个
    inst.set_view_lo(9.0);
    inst.set_view_hi(10.0);
    inst.window().request_redraw();
    let none = ElementHandle::find_by_element_type_name(&inst, "Marker").count();
    assert_eq!(none, 0, "空视口应实例化 0 个标记，得 {none}");

    // 线段/片断的区间相交裁剪（连接线 / NLA 片断用的条件）
    let span = SpanCulled::new().unwrap();
    let all_span = ElementHandle::find_by_element_type_name(&span, "SpanMarker").count();
    assert_eq!(all_span, 3, "全可见时应创建全部 3 个片断");

    // 视口 [3,5]：与 [0,2](否) [4,6](是) [8,10](否) → 1 个
    span.set_view_lo(3.0);
    span.set_view_hi(5.0);
    span.window().request_redraw();
    let one = ElementHandle::find_by_element_type_name(&span, "SpanMarker").count();
    assert_eq!(one, 1, "区间相交裁剪后应只实例化 1 个片断，得 {one}");

    // 视口 [5.5,7.5]：仅 [4,6] 相交 → 1 个
    span.set_view_lo(5.5);
    span.set_view_hi(7.5);
    span.window().request_redraw();
    let one2 = ElementHandle::find_by_element_type_name(&span, "SpanMarker").count();
    assert_eq!(one2, 1, "区间相交裁剪后应只实例化 1 个片断，得 {one2}");
}
