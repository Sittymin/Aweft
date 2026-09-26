use crate::bridge::types::{CollabCallbacks, MainWindow};
use crate::store::AppStore;
use slint::ComponentHandle;
use std::{cell::RefCell, rc::Rc};

pub(super) fn setup_collab_callbacks(instance: &MainWindow, store: Rc<RefCell<AppStore>>) {
    let callbacks = instance.global::<CollabCallbacks>();

    let member_store = Rc::clone(&store);
    callbacks.on_copy_member_id(move || {
        let member_id = member_store.borrow().collab_bridge.member_id();
        if let Err(error) = member_store.borrow_mut().clipboard.write_text(&member_id) {
            log::error!("复制本机成员 ID 失败：{error:?}");
        }
    });

    let create_store = Rc::clone(&store);
    callbacks.on_create_session(move |name| {
        let mut app = create_store.borrow_mut();
        let result = {
            let AppStore {
                collab_bridge,
                state,
                ..
            } = &mut *app;
            collab_bridge.create_session(&mut state.project, name.as_str())
        };
        match result {
            Ok(()) => {
                app.sync_collab_ui();
                drop(app);
                crate::collab_bridge::start_event_pump(&create_store);
            }
            Err(error) => log::error!("创建协作会话失败：{error:?}"),
        }
    });

    let invite_store = Rc::clone(&store);
    callbacks.on_copy_invite(move |member_id| {
        let result = invite_store
            .borrow_mut()
            .collab_bridge
            .make_invite(member_id.as_str());
        match result {
            Ok(invite) => {
                if let Err(error) = invite_store.borrow_mut().clipboard.write_text(&invite) {
                    log::error!("邀请码生成成功，但复制到剪贴板失败：{error:?}");
                }
            }
            Err(error) => log::error!("生成协作邀请失败：{error:?}"),
        }
    });

    let accept_store = Rc::clone(&store);
    callbacks.on_accept_invite(move |invite| {
        let mut app = accept_store.borrow_mut();
        let result = {
            let AppStore {
                collab_bridge,
                state,
                ..
            } = &mut *app;
            collab_bridge.accept_invite(&mut state.project, invite.as_str())
        };
        match result {
            Ok(()) => {
                app.sync_collab_ui();
                drop(app);
                crate::collab_bridge::start_event_pump(&accept_store);
            }
            Err(error) => log::error!("加入协作会话失败：{error:?}"),
        }
    });

    callbacks.on_leave_session(move || {
        let mut app = store.borrow_mut();
        let result = {
            let AppStore {
                collab_bridge,
                state,
                ..
            } = &mut *app;
            collab_bridge.leave(&mut state.project)
        };
        if let Err(error) = result {
            log::error!("离开协作会话失败：{error:?}");
        }
        app.sync_collab_ui();
    });
}
