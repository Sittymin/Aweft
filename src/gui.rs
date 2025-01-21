use eframe::egui;
use std::sync::Arc;

pub fn gui() -> eframe::Result<()> {
    let native_options = eframe::NativeOptions::default();

    // 运行原生应用程序
    eframe::run_native(
        "My Application",
        native_options,
        Box::new(|cc| Ok(Box::new(MyApp::new(cc)))),
    )
}

struct MyApp {
    // 你的应用状态
}

impl MyApp {
    fn new(cc: &eframe::CreationContext<'_>) -> Self {
        let mut fonts = egui::FontDefinitions::default();

        // 添加中文字体
        fonts.font_data.insert(
            "霞鹜新晰黑".to_owned(),
            Arc::new(egui::FontData::from_static(include_bytes!(
                "../assets/LXGWNeoXiHei.ttf"
            ))),
        );

        // 将中文字体设置为首选字体
        fonts
            .families
            .entry(egui::FontFamily::Proportional)
            .or_default()
            .insert(0, "霞鹜新晰黑".to_owned());

        cc.egui_ctx.set_fonts(fonts);
        // cc.egui_ctx 是 egui::Context,可以在这里自定义字体和样式
        Self {
            // 初始化你的应用状态
        }
    }
}

impl eframe::App for MyApp {
    fn update(&mut self, ctx: &egui::Context, frame: &mut eframe::Frame) {
        egui::CentralPanel::default().show(ctx, |ui| {
            ui.heading("这里是 Heading");
            // 这里使用你之前的窗口代码
            egui::Window::new("My Window").show(ctx, |ui| {
                ui.label("一个内部窗口");
            });
        });
    }
}
