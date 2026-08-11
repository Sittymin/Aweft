// ContextMenu `no-close-index` 回归测试：
//   * 选择 no-close-index 对应的项（如角色下拉的"新建角色"）后菜单保持打开，
//     供外部原位替换输入框；
//   * 选择其他项或 no-close-index = -1 时菜单照常关闭。
// 交互通过 slint 无头测试后端（ElementHandle::single_click）模拟点击菜单项。
// 注意：测试后端进程内只能 init 一次，故所有场景合并为单个 #[test]。
use i_slint_backend_testing::{init_integration_test_with_system_time, ElementHandle};
use slint::platform::PointerEventButton;
use slint::ComponentHandle;

slint::slint! {
    import { ContextMenu, ContextMenuItem } from "../ui/widgets/context_menu.slint";

    export component MenuHarness inherits Window {
        in property <[ContextMenuItem]> items: [
            { text: "Alice", enabled: true, dot-color: Colors.transparent, separator-before: false },
            { text: "Bob", enabled: true, dot-color: Colors.transparent, separator-before: false },
            { text: "新建角色", enabled: true, dot-color: Colors.transparent, separator-before: true },
        ];
        in-out property <bool> menu-open: true;
        in property <int> no-close-index: -1;
        callback chosen(index: int);

        ContextMenu {
            x: 0px;
            y: 0px;
            items: root.items;
            open <=> root.menu-open;
            current-index: -1;
            no-close-index: root.no-close-index;
            item-selected(i) => { root.chosen(i); }
            container-width: 200px;
            container-height: 300px;
        }
    }
}

#[test]
fn no_close_index_behavior() {
    init_integration_test_with_system_time();
    let inst = MenuHarness::new().unwrap();
    inst.window().set_size(slint::PhysicalSize::new(400, 400));

    slint::spawn_local(async move {
        // 场景 A：no-close-index = 2（新建角色）→ 点击第 3 项，菜单保持打开
        inst.set_menu_open(true);
        inst.set_no_close_index(2);
        {
            let areas: Vec<_> = ElementHandle::find_by_element_type_name(&inst, "TouchArea").collect();
            println!("场景A TouchArea 数量: {}", areas.len());
            areas[2].single_click(PointerEventButton::Left).await;
        }
        assert!(inst.get_menu_open(), "A: 选择 no-close-index 项后菜单应保持打开");

        // 场景 B：no-close-index = 2 → 点击第 1 项，菜单关闭
        inst.set_menu_open(true);
        {
            let areas: Vec<_> = ElementHandle::find_by_element_type_name(&inst, "TouchArea").collect();
            areas[0].single_click(PointerEventButton::Left).await;
        }
        assert!(!inst.get_menu_open(), "B: 选择普通项后菜单应关闭");

        // 场景 C：no-close-index = -1（默认）→ 点击任何项都关闭
        inst.set_menu_open(true);
        inst.set_no_close_index(-1);
        {
            let areas: Vec<_> = ElementHandle::find_by_element_type_name(&inst, "TouchArea").collect();
            areas[2].single_click(PointerEventButton::Left).await;
        }
        assert!(!inst.get_menu_open(), "C: no-close-index=-1 时所有项都关闭菜单");

        slint::quit_event_loop().unwrap();
    });
    slint::run_event_loop().unwrap();
}
