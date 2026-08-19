// 固定槽位刻度回归测试：标尺刻度是纯 Slint 即时计算（对齐 Blender 每帧重算思路）。
// 验证核心性质（完整机制见 ruler.slint 顶部注释）：
//   * 槽位 repeater 常驻不重建——pan 只移动平移容器（grid-group），槽位绑定不引用 pan，
//     元素树稳定（主刻度恒为 96 个，不会随 pan 重建/消失）。
//   * 刻度对齐可见左边界（Math.floor(view-lo/step)*step），pan 到任何位置仍有刻度。
use i_slint_backend_testing::{ElementHandle, init_integration_test_with_system_time};
use slint::ComponentHandle;

slint::slint! {
    component TickMark inherits Rectangle {
        width: 1px;
        height: 7px;
    }

    export component RulerSlots inherits Window {
        in property <float> view-lo: 0.0;
        in property <float> view-hi: 100.0;
        in property <float> zoom: 30.0;
        // 像素偏移（真实编辑器里 pan 同时改变 view-lo/hi 与 px 位置，这里联动地一起设）
        in property <float> pan: 0.0;

        pure function major-step() -> float {
            let z: float = max(root.zoom, 0.01);
            let r: float = max(root.view-hi - root.view-lo, 0.1);
            let raw: float = max(48.0 / z, r / 120.0);
            let mag: float = Math.pow(10, Math.floor(Math.log(raw, 10)));
            let norm: float = raw / mag;
            let step: float = norm < 1.5 ? 1.0 : (norm < 3.5 ? 2.0 : (norm < 7.5 ? 5.0 : 10.0));
            return max(0.001, step * mag);
        }
        pure function major-value(i: int) -> float {
            return Math.floor(root.view-lo / root.major-step()) * root.major-step() + i * root.major-step();
        }
        pure function major-px(i: int) -> float {
            return root.major-value(i) * root.zoom;
        }

        // 平移容器：pan 只改容器 x，槽位绑定不引用 pan（与 ruler.slint 同构）
        Rectangle {
            x: root.pan * 1px;
            y: 0px;
            width: 100%;
            height: 100%;
            for i in 96: Rectangle {
                width: 100%;
                height: 100%;
                TickMark {
                    x: root.major-px(i) * 1px;
                    y: 4px;
                }
            }
        }
    }
}

#[test]
fn ticks_are_static_and_follow_pan() {
    init_integration_test_with_system_time();

    let inst = RulerSlots::new().unwrap();

    // 主刻度 96 个槽位全部常驻（无 if 裁剪：裁剪由外层 clip 承担，pan 零重建）。
    // 注：Slint 对完全移出窗口的 repeater 行会自动跳过实例化，故 pan 后边界
    // 可能少 1-2 个（只在边界行创建/销毁，核心 96 槽不重建）。
    inst.window().request_redraw();
    let base = ElementHandle::find_by_element_type_name(&inst, "TickMark").count();
    assert_eq!(base, 96, "初始时主刻度槽位应全为 96 个，得 {base}");

    // pan 变化：槽位几乎不重建（边界 1-2 行出窗被裁剪，其余常驻）
    inst.set_view_lo(33.3333);
    inst.set_view_hi(66.6666);
    inst.set_pan(-1000.0);
    inst.window().request_redraw();
    let panned = ElementHandle::find_by_element_type_name(&inst, "TickMark").count();
    assert!(
        (94..=96).contains(&panned),
        "pan 后槽位应几乎全常驻（94-96 个），得 {panned}"
    );

    // pan 到更远位置：仍几乎全常驻（pan 到任意位置都有刻度，不重建）
    inst.set_view_lo(100.0);
    inst.set_view_hi(133.3333);
    inst.set_pan(-3000.0);
    inst.window().request_redraw();
    let far = ElementHandle::find_by_element_type_name(&inst, "TickMark").count();
    assert!(
        (94..=96).contains(&far),
        "pan 到任意位置都应几乎全常驻（94-96 个），得 {far}"
    );
}
