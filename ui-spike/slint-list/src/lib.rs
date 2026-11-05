//! The Slint side of the spike: the app component, its lazily computed model, shared by the window and the picture test.
use slint::platform::software_renderer::{
    MinimalSoftwareWindow, PremultipliedRgbaColor, RepaintBufferType,
};
use slint::platform::{Platform, WindowAdapter, WindowEvent};
use slint::{ComponentHandle, Model, ModelRc, ModelTracker, PhysicalSize};
use std::rc::Rc;
use ui_spike_common::{ROWS, Tone, row};

slint::include_modules!();

struct Rows;

impl Model for Rows {
    type Data = RowData;

    fn row_count(&self) -> usize {
        ROWS
    }

    fn row_data(&self, i: usize) -> Option<RowData> {
        (i < ROWS).then(|| {
            let r = row(i);
            RowData {
                title: r.title.into(),
                subtitle: r.subtitle.into(),
                initials: r.initials.into(),
                badge: r.badge.into(),
                tone: match r.tone {
                    Tone::Primary => 0,
                    Tone::Secondary => 1,
                    Tone::Outline => 2,
                    Tone::Destructive => 3,
                },
            }
        })
    }

    fn model_tracker(&self) -> &dyn ModelTracker {
        &()
    }
}

pub fn app(dark: bool) -> AppWindow {
    let app = AppWindow::new().expect("window");
    app.set_rows(ModelRc::new(Rows));
    app.set_count("100,000".into());
    let theme = app.global::<Theme>();
    theme.set_mode(if dark {
        ThemeMode::Dark
    } else {
        ThemeMode::Light
    });
    app
}

/// The list with no window, on Slint's software renderer: what the picture test and --headless share.
pub struct Headless {
    window: Rc<MinimalSoftwareWindow>,
}

struct HeadlessPlatform(Rc<MinimalSoftwareWindow>);

impl Platform for HeadlessPlatform {
    fn create_window_adapter(&self) -> Result<Rc<dyn WindowAdapter>, slint::PlatformError> {
        Ok(self.0.clone())
    }
}

impl Headless {
    pub fn new() -> Self {
        let window = MinimalSoftwareWindow::new(RepaintBufferType::NewBuffer);
        slint::platform::set_platform(Box::new(HeadlessPlatform(window.clone())))
            .expect("platform");
        Self { window }
    }

    /// Sizes the window in physical pixels at `scale`, shows the app.
    pub fn show(&self, app: &AppWindow, width: u32, height: u32, scale: f32) {
        self.window.dispatch_event(WindowEvent::ScaleFactorChanged {
            scale_factor: scale,
        });
        self.window.set_size(PhysicalSize::new(width, height));
        app.show().expect("show");
    }

    /// Lays out and draws the whole window. RGBA, opaque.
    pub fn draw(&self, pixels: &mut [PremultipliedRgbaColor], width: u32) {
        slint::platform::update_timers_and_animations();
        self.window.request_redraw();
        self.window.draw_if_needed(|renderer| {
            renderer.render(pixels, width as usize);
        });
    }

    /// One picture: the first draw lays the list out, the next sees the rows at the scroll offset.
    pub fn render_once(&self, dark: bool, width: u32, height: u32, scroll: f64) -> Vec<u8> {
        let app = app(dark);
        self.show(&app, width, height, 1.0);
        let mut pixels = vec![PremultipliedRgbaColor::default(); (width * height) as usize];
        for _ in 0..3 {
            app.set_scroll_y(-scroll as f32);
            self.draw(&mut pixels, width);
        }
        app.hide().expect("hide");
        pixels
            .iter()
            .flat_map(|p| [p.red, p.green, p.blue, 255])
            .collect()
    }
}

impl Default for Headless {
    fn default() -> Self {
        Self::new()
    }
}
