//! 实证：按下关键帧时更新**外层通道模型**（`selected`）会破坏 Slint 拖拽抓取。
//!
//! 背景：`graph_editor` 因此把通道选中延迟到 released 写回，另设 `selected_display`
//! 掩码（独立模型）让曲线在按下时即时高亮（"显示先动"）。本测试**锁定该约束**：
//! 一旦 Slint 行为变化（外层模型更新不再破坏 grab），此测试会失败——那时即可移除
//! 延迟写回与 `selected_display` 的解耦，直接在按下时更新 channels 模型。

use i_slint_backend_testing::{ElementHandle, init_integration_test_with_system_time};
use slint::platform::PointerEventButton;
use slint::{ComponentHandle, LogicalPosition, Model, ModelRc, VecModel};
use std::cell::RefCell;
use std::rc::Rc;

slint::slint! {
    export struct Keyframe { time: float, selected: bool }
    export struct Channel { keyframes: [Keyframe], selected: bool }

    component Marker inherits Rectangle {
        in property <float> time;
        in property <bool> selected: false;
        callback dragged(dt: float);
        callback released;
        callback make-selected;
        private property <length> hit-size: 17px;
        private property <bool> dragging: false;
        private property <float> press-mouse-x: 0.0;
        x: root.time * 30px - hit-size / 2;
        y: 0px;
        width: hit-size;
        height: hit-size;
        background: transparent;
        opacity: root.selected ? 1.0 : 0.35;
        Rectangle {
            width: 9px;
            height: 9px;
            x: (parent.width - self.width) / 2;
            y: (parent.height - self.height) / 2;
            background: #ff6d78;
        }
        ta := TouchArea {
            pointer-event(event) => {
                if (event.kind == PointerEventKind.down && event.button == PointerEventButton.left) {
                    root.dragging = true;
                    root.press-mouse-x = self.mouse-x / 1px;
                    root.make-selected();
                } else if (event.kind == PointerEventKind.move && root.dragging) {
                    root.dragged((self.mouse-x / 1px - root.press-mouse-x) / 30.0);
                } else if (event.kind == PointerEventKind.up) {
                    if root.dragging {
                        root.released();
                    }
                    root.dragging = false;
                }
            }
        }
    }

    export component DragTest inherits Window {
        width: 400px;
        height: 100px;
        in-out property <[Channel]> channels;
        callback dragged(index: int, dt: float);
        callback released();
        callback make-selected(index: int);
        // 复刻 GraphEditor：外层 for（通道）内层 for（关键帧）
        for ch[i] in root.channels: Rectangle {
            width: 100%;
            height: 20px;
            for key[j] in ch.keyframes: Marker {
                time: key.time;
                selected: key.selected;
                dragged(dt) => { root.dragged(i, dt); }
                released => { root.released(); }
                make-selected => { root.make-selected(i); }
            }
        }
    }
}

#[test]
fn outer_channel_selected_update_on_press_interrupts_drag() {
    init_integration_test_with_system_time();

    slint::spawn_local(async move {
        let app = DragTest::new().unwrap();
        let channels = Rc::new(VecModel::from(vec![
            Channel {
                keyframes: ModelRc::from(Rc::new(VecModel::from(vec![Keyframe {
                    time: 0.0,
                    selected: false,
                }]))),
                selected: false,
            },
            Channel {
                keyframes: ModelRc::from(Rc::new(VecModel::from(vec![Keyframe {
                    time: 2.0,
                    selected: false,
                }]))),
                selected: false,
            },
        ]));
        app.set_channels(ModelRc::from(channels.clone()));

        let dts = Rc::new(RefCell::new(vec![]));
        let released = Rc::new(RefCell::new(0usize));
        {
            let dts = dts.clone();
            let channels = channels.clone();
            app.on_dragged(move |i, dt| {
                dts.borrow_mut().push(dt);
                // 复刻 keyframe_dragged：只更新内层关键帧模型（marker 位置随之移动）
                let mut ch = channels.row_data(i as usize).unwrap();
                let mut kf = ch.keyframes.row_data(0).unwrap();
                kf.time += dt;
                ch.keyframes.set_row_data(0, kf);
            });
        }
        {
            let released = released.clone();
            app.on_released(move || *released.borrow_mut() += 1);
        }
        {
            // 按下时：清掉所有关键帧选中 + 重选被按下的关键帧（内层），
            // **并更新外层 channel.selected = true**（复刻"按下即选中整条曲线"）。
            let channels = channels.clone();
            app.on_make_selected(move |i| {
                for c in 0..channels.row_count() {
                    let mut ch = channels.row_data(c).unwrap();
                    for k in 0..ch.keyframes.row_count() {
                        let mut kf = ch.keyframes.row_data(k).unwrap();
                        if kf.selected {
                            kf.selected = false;
                            ch.keyframes.set_row_data(k, kf);
                        }
                    }
                }
                let mut ch = channels.row_data(i as usize).unwrap();
                let mut kf = ch.keyframes.row_data(0).unwrap();
                kf.selected = true;
                ch.keyframes.set_row_data(0, kf);
                // 外层通道 selected 更新（曲线高亮的来源）
                ch.selected = true;
                channels.set_row_data(i as usize, ch);
            });
        }

        let elem = ElementHandle::find_by_element_type_name(&app, "Marker")
            .nth(1)
            .expect("找不到第 2 个 marker");
        elem.drag(LogicalPosition::new(210.0, 28.5), PointerEventButton::Left)
            .await;

        let got = dts.borrow().clone();
        let ch1 = channels.row_data(1).unwrap();
        let final_time = ch1.keyframes.row_data(0).unwrap().time;
        println!(
            "收到 dragged 次数: {}，dt 累计: {:.3}，最终 time: {final_time:.3}，released: {}",
            got.len(),
            got.iter().sum::<f32>(),
            *released.borrow()
        );
        // 锁定约束：更新外层 channels 模型会重建嵌套关键帧 repeater → grab 中断，
        // marker 不再收到 move（time 停在 2.0）。若此断言失败（拖拽存活），
        // 说明 Slint 已不破坏 grab，可移除"延迟写回 + selected_display 解耦"。
        assert!(
            final_time <= 2.01,
            "外层 channels 更新本应中断拖拽（time 停在 2.0），实际动了 {final_time:.3}——可移除解耦"
        );
        slint::quit_event_loop().unwrap();
    })
    .unwrap();
    slint::run_event_loop().unwrap();
}
