// 验证 Slint 焦点机制：点击非输入区域（编辑器背景 FocusScope，focus-on-click 默认 true）
// 会使 TextInput 失去焦点——这是"点击画板退出文本编辑"的标准做法。
use i_slint_backend_testing::{init_integration_test_with_system_time, ElementHandle};
use slint::platform::PointerEventButton;
use slint::ComponentHandle;

slint::slint! {
    export component FocusHarness inherits Window {
        out property <bool> input-focused: input.has-focus;

        // 背景 FocusScope：点击画布任意非输入区会 claim 焦点，使 TextInput 失焦
        FocusScope {
            x: 0px;
            y: 0px;
            width: 100%;
            height: 100%;
        }

        input := TextInput {
            x: 200px;
            y: 200px;
            width: 120px;
            height: 30px;
            text: "hello";
        }
    }
}

#[test]
fn click_on_background_focus_scope_defocuses_text_input() {
    init_integration_test_with_system_time();
    let inst = FocusHarness::new().unwrap();
    inst.window().set_size(slint::PhysicalSize::new(600, 500));

    slint::spawn_local(async move {
        // 点击 TextInput 获得焦点
        {
            let elem = ElementHandle::find_by_element_type_name(&inst, "TextInput")
                .nth(0)
                .expect("找不到 TextInput");
            elem.single_click(PointerEventButton::Left).await;
        }
        assert!(inst.get_input_focused(), "点击输入框后应获得焦点");

        // 点击画布空白（背景 FocusScope 区域）
        {
            let elem = ElementHandle::find_by_element_type_name(&inst, "FocusScope")
                .nth(0)
                .expect("找不到 FocusScope");
            elem.single_click(PointerEventButton::Left).await;
        }
        assert!(!inst.get_input_focused(), "点击画布空白后 TextInput 应失去焦点");

        slint::quit_event_loop().unwrap();
    });
    slint::run_event_loop().unwrap();
}
