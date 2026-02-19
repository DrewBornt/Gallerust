#![cfg_attr(target_os = "windows", windows_subsystem = "windows")]

// eframe is the application framework that wraps egui. It handles window creation,
// the event loop, and the rendering backend so we don't have to manage any of that
// ourselves. This replaces winit + pixels entirely.
use eframe::egui;

// egui's image support works through "textures" — images uploaded to the GPU
// that egui can then draw efficiently. ColorImage is the CPU-side representation
// we build from our raw pixel data before uploading.
use egui::ColorImage;

// TextureHandle is a reference-counted handle to a GPU texture.
// We wrap it in Option because we don't have an image loaded at startup.
use egui::TextureHandle;

use std::path::PathBuf;
use rfd::FileDialog;

fn main() -> eframe::Result<()> {
    // NativeOptions configures the native window that eframe creates.
    let options = eframe::NativeOptions {
        viewport: egui::ViewportBuilder::default()
            .with_title("Gallerust")
            .with_maximized(true),
        ..Default::default()
    };

    // eframe::run_native is the entry point. It takes:
    // 1. The window title (also used as the app ID for state persistence)
    // 2. The window options
    // 3. A boxed closure that constructs our App struct
    // This call blocks and runs the event loop until the window is closed.
    eframe::run_native(
        "Gallerust",
        options,
        Box::new(|cc| {
            // Enable image support in egui. Without this, egui won't know
            // how to load image bytes into textures.
            egui_extras::install_image_loaders(&cc.egui_ctx);
            Box::new(Gallerust::new())
        }),
    )
}

// Our main application struct. Holds all state that persists between frames.
struct Gallerust {
    // Sorted list of image file paths found in the selected folder.
    images: Vec<PathBuf>,

    // Index into `images` for the currently displayed image.
    current_index: usize,

    // The current zoom level. 1.0 means "fit to available space".
    // We use a multiplicative zoom model now (see zoom section below)
    // so 2.0 means twice the fit size, 0.5 means half, etc.
    zoom: f32,

    // The GPU texture for the currently displayed image.
    // None means no image is loaded yet (before the user picks a file).
    texture: Option<TextureHandle>,
}

impl Gallerust {
    fn new() -> Self {
        Self {
            images: Vec::new(),
            current_index: 0,
            zoom: 1.0,
            texture: None,
        }
    }

    // Open a file picker dialog and load the selected image and its folder.
    fn open_file(&mut self, ctx: &egui::Context) {
        // Show a native OS file picker filtered to supported image types.
        // pick_file() blocks until the user makes a selection or cancels.
        let Some(file) = FileDialog::new()
            .add_filter("Images", &["jpg", "jpeg", "png", "gif", "webp", "bmp"])
            .pick_file()
        else {
            return; // User cancelled, do nothing
        };

        // Derive the folder from the selected file's parent directory so
        // we can browse all images in the same folder.
        let Some(folder) = file.parent() else {
            return;
        };

        // Use match instead of unwrap() so a folder read failure doesn't
        // crash the app. This can happen if a drive is ejected, permissions
        // change, or the path is on an unavailable network share.
        let read_result = match std::fs::read_dir(folder) {
            Ok(r) => r,
            Err(e) => {
                eprintln!("Failed to read folder: {e}");
                return;
            }
        };

        // Scan folder for supported image files, filtering by extension.
        // to_lowercase() ensures .JPG and .jpg both match.
        let mut images: Vec<PathBuf> = read_result
            .filter_map(|entry| {
                let path = entry.ok()?.path();
                match path.extension()?.to_str()?.to_lowercase().as_str() {
                    "jpg" | "jpeg" | "png" | "gif" | "webp" | "bmp" => Some(path),
                    _ => None,
                }
            })
            .collect();

        images.sort();

        if images.is_empty() {
            eprintln!("No supported images found in folder.");
            return;
        }

        // Start on the file the user actually picked rather than always
        // defaulting to the first file alphabetically.
        let current_index = images.iter()
            .position(|p| p == &file)
            .unwrap_or(0);

        self.images = images;
        self.current_index = current_index;
        self.zoom = 1.0;
        self.load_texture(ctx);
    }

    // Load the image at current_index from disk and upload it to the GPU
    // as an egui texture. egui handles scaling and rendering from here.
    fn load_texture(&mut self, ctx: &egui::Context) {
        let path = &self.images[self.current_index];

        let img = match image::open(path) {
            Ok(i) => i.to_rgba8(),
            Err(e) => {
                eprintln!("Failed to load image: {e}");
                return;
            }
        };

        let (width, height) = img.dimensions();

        // ColorImage is egui's CPU-side image type. We convert the raw RGBA
        // bytes into Color32 values by chunking into groups of 4 bytes.
        let pixels: Vec<egui::Color32> = img
            .chunks_exact(4)
            .map(|p| egui::Color32::from_rgba_unmultiplied(p[0], p[1], p[2], p[3]))
            .collect();

        let color_image = ColorImage {
            size: [width as usize, height as usize],
            pixels,
        };

        // Upload to GPU. TextureOptions::LINEAR gives smooth scaling
        // (bilinear filtering) instead of blocky nearest-neighbor.
        self.texture = Some(ctx.load_texture(
            "current_image",
            color_image,
            egui::TextureOptions::LINEAR,
        ));
    }

    // Navigate to the next image, wrapping from last back to first.
    fn go_next(&mut self, ctx: &egui::Context) {
        if self.images.is_empty() { return; }
        self.current_index = (self.current_index + 1) % self.images.len();
        self.zoom = 1.0;
        self.load_texture(ctx);
    }

    // Navigate to the previous image, wrapping from first back to last.
    fn go_prev(&mut self, ctx: &egui::Context) {
        if self.images.is_empty() { return; }
        // checked_sub prevents usize underflow at index 0
        self.current_index = self.current_index
            .checked_sub(1)
            .unwrap_or(self.images.len() - 1);
        self.zoom = 1.0;
        self.load_texture(ctx);
    }

    // Apply a multiplicative zoom delta, clamped to a safe range.
    // This is used by both scroll wheel and pinch-to-zoom gestures.
    // Multiplicative zoom feels more natural than additive because each
    // step is proportional to the current zoom level — going from 1.0 to
    // 2.0 feels the same as going from 2.0 to 4.0.
    fn apply_zoom_delta(&mut self, delta: f32) {
        self.zoom = (self.zoom * delta).clamp(0.1, 5.0);
    }

    // Build the title string e.g. "cat.jpg (3/12)".
    fn title(&self) -> String {
        if self.images.is_empty() {
            return "Gallerust".to_string();
        }
        let filename = self.images[self.current_index]
            .file_name()
            .unwrap_or_default()
            .to_string_lossy();
        format!("{} ({}/{})", filename, self.current_index + 1, self.images.len())
    }
}

impl eframe::App for Gallerust {
    fn update(&mut self, ctx: &egui::Context, _frame: &mut eframe::Frame) {

        // ── Zoom input ───────────────────────────────────────────────────────
        // We handle zoom at the top of update(), before the panels are drawn,
        // so input is never missed regardless of what widget has focus.

        // zoom_delta() is egui's cross-platform normalized zoom input.
        // It returns a multiplier: 1.1 = 10% larger, 0.9 = 10% smaller, 1.0 = no change.
        // It handles both scroll wheels AND trackpad pinch-to-zoom gestures automatically,
        // and egui normalizes the raw platform delta values for us so we don't have to
        // worry about different mice or OSes reporting wildly different scroll magnitudes.
        let zoom_delta = ctx.input(|i| i.zoom_delta());
        if zoom_delta != 1.0 {
            self.apply_zoom_delta(zoom_delta);
        }

        // Keyboard zoom uses the same multiplicative model for consistency.
        // 1.1 and 0.9 match what a single scroll notch typically produces,
        // so keyboard and scroll wheel feel equivalent.
        if ctx.input(|i| i.key_pressed(egui::Key::Plus) || i.key_pressed(egui::Key::Equals)) {
            self.apply_zoom_delta(1.1);
        }
        if ctx.input(|i| i.key_pressed(egui::Key::Minus)) {
            self.apply_zoom_delta(0.9);
        }

        // ── Bottom toolbar panel ─────────────────────────────────────────────
        // Panels claim space from the edges inward. Bottom panel is declared
        // first so the central panel fills the remaining space above it.
        egui::TopBottomPanel::bottom("toolbar")
            .exact_height(48.0)
            .show(ctx, |ui| {
                ui.horizontal_centered(|ui| {

                    if ui.button("📂 Open").clicked() {
                        self.open_file(ctx);
                    }

                    ui.separator();

                    if ui.button("◀ Prev").clicked() {
                        self.go_prev(ctx);
                    }
                    if ui.button("Next ▶").clicked() {
                        self.go_next(ctx);
                    }

                    ui.separator();

                    ui.label(self.title());

                    ui.separator();

                    // Zoom slider. Because we now use a multiplicative zoom model,
                    // the slider still works fine — it just directly sets self.zoom
                    // to whatever value the user drags to.
                    ui.label("Zoom:");
                    ui.add(
                        egui::Slider::new(&mut self.zoom, 0.1..=5.0)
                            .step_by(0.1)
                            .fixed_decimals(1)
                    );

                    if ui.button("↺").on_hover_text("Reset zoom").clicked() {
                        self.zoom = 1.0;
                    }
                });
            });

        // ── Central panel (image display area) ──────────────────────────────
        egui::CentralPanel::default()
    .frame(egui::Frame::none().fill(egui::Color32::BLACK))
    .show(ctx, |ui| {
        if let Some(texture) = &self.texture {
            let available = ui.available_size();

            let img_size = texture.size_vec2();
            let scale_x = available.x / img_size.x;
            let scale_y = available.y / img_size.y;
            let base_scale = scale_x.min(scale_y);
            let final_scale = base_scale * self.zoom;

            let display_size = egui::vec2(
                img_size.x * final_scale,
                img_size.y * final_scale,
            );

            // ScrollArea prevents the image from overflowing outside the
            // central panel when zoomed in. Without this, a large display_size
            // would push content past the panel boundary and hide the toolbar.
            // auto_shrink(false) ensures the scroll area always fills the
            // full available space even when the image is smaller than the panel.
            egui::ScrollArea::both()
                .auto_shrink([false, false])
                .show(ui, |ui| {
                    // When the image is smaller than the panel (zoom <= 1.0),
                    // we still want it centered. We do this by adding padding
                    // inside the scroll area.
                    let padding_x = ((available.x - display_size.x) / 2.0).max(0.0);
                    let padding_y = ((available.y - display_size.y) / 2.0).max(0.0);

                    ui.add_space(padding_y);
                    ui.horizontal(|ui| {
                        ui.add_space(padding_x);
                        ui.add(
                            egui::Image::new(texture)
                                .fit_to_exact_size(display_size)
                        );
                    });
                });

        } else {
            ui.centered_and_justified(|ui| {
                ui.label("Click '📂 Open' to select an image");
            });
        }

        // Keyboard navigation
        if ctx.input(|i| i.key_pressed(egui::Key::ArrowRight)) {
            self.go_next(ctx);
        }
        if ctx.input(|i| i.key_pressed(egui::Key::ArrowLeft)) {
            self.go_prev(ctx);
        }
    });
    }
}