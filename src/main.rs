#![cfg_attr(target_os = "windows", windows_subsystem = "windows")]

// eframe is the application framework that wraps egui. It handles window creation,
// the event loop, and the rendering backend so we don't have to manage any of that
// ourselves. This replaces winit + pixels entirely.
use eframe::egui;

// egui's image support works through "textures" — images uploaded to the GPU
// that egui can then draw efficiently. ColorImage is the CPU-side representation
// we build from our raw pixel data before uploading.
use egui::ColorImage;

// RetainedImage was removed in newer egui versions in favor of TextureHandle,
// which is what we'll use. A TextureHandle is a reference-counted handle to a
// GPU texture. We wrap it in Option because we don't have an image loaded
// at startup.
use egui::TextureHandle;

use std::path::PathBuf;
use rfd::FileDialog;

fn main() -> eframe::Result<()> {
    // NativeOptions configures the native window that eframe creates.
    // We want it maximized by default, matching our previous behavior.
    let options = eframe::NativeOptions {
        viewport: egui::ViewportBuilder::default()
            .with_title("Gallerust")
            .with_maximized(true),
        ..Default::default()  // Use defaults for everything else
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
            // The CreationContext `cc` gives us access to the egui context
            // at startup. We need to call this to enable image support —
            // without it, egui won't know how to load image bytes into textures.
            egui_extras::install_image_loaders(&cc.egui_ctx);

            // Construct and return our app. Box::new wraps it in a heap
            // allocation as required by eframe's trait object interface.
            Box::new(Gallerust::new())
        }),
    )
}

// Our main application struct. This holds all the state that needs to persist
// between frames — equivalent to the AppState struct we had before, but now
// it also drives the UI directly through the eframe::App trait.
struct Gallerust {
    // The list of image paths found in the selected folder, sorted alphabetically.
    images: Vec<PathBuf>,

    // Index into `images` for the currently displayed image.
    current_index: usize,

    // The current zoom level. 1.0 means "fit to available space".
    // We store this as an f32 so the slider can mutate it directly.
    zoom: f32,

    // The GPU texture for the currently displayed image.
    // None means no image is loaded yet (before the user picks a folder).
    // We replace this every time the user navigates to a new image.
    texture: Option<TextureHandle>,
}

impl Gallerust {
    fn new() -> Self {
        // Start with an empty state. The user hasn't picked a folder yet,
        // so images is empty and texture is None. The UI will show a prompt.
        Self {
            images: Vec::new(),
            current_index: 0,
            zoom: 1.0,
            texture: None,
        }
    }

    // Open a file picker dialog and load the selected image and its folder.
    // This is called when the user clicks "Open Image" or on first launch.
    fn open_file(&mut self, ctx: &egui::Context) {
        // Show a native OS file picker filtered to supported image types.
        // pick_file() blocks until the user makes a selection or cancels.
        let Some(file) = FileDialog::new()
            .add_filter("Images", &["jpg", "jpeg", "png", "gif", "webp", "bmp"])
            .pick_file()
        else {
            return;  // User cancelled, do nothing
        };

        // Derive the folder from the selected file's parent directory.
        // We want to browse all images in the same folder, not just the one picked.
        let Some(folder) = file.parent() else {
            return;
        };

        // Use match instead of unwrap() so a folder read failure doesn't crash the app.
        // This can happen if the drive is ejected, permissions change, or the path
        // is on an unavailable network share.
        let read_result = match std::fs::read_dir(folder) {
            Ok(r) => r,
            Err(e) => {
                eprintln!("Failed to read folder: {e}");
                return;  // Bail out gracefully instead of panicking
            }
        };

        // Scan the folder for all supported image files, same logic as before.
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
            return;
        }

        // Start on the file the user actually picked rather than the first
        // file alphabetically.
        let current_index = images.iter()
            .position(|p| p == &file)
            .unwrap_or(0);

        self.images = images;
        self.current_index = current_index;
        self.zoom = 1.0;

        // Load the selected image into a GPU texture immediately.
        self.load_texture(ctx);
    }

    // Load the image at current_index from disk and upload it to the GPU
    // as an egui texture. This replaces our old load_image() + draw_image()
    // pipeline — egui handles the scaling and rendering for us once we
    // have a texture.
    fn load_texture(&mut self, ctx: &egui::Context) {
        let path = &self.images[self.current_index];

        // Use the `image` crate to decode the file into raw RGBA pixels,
        // same as before.
        let img = match image::open(path) {
            Ok(i) => i.to_rgba8(),
            Err(e) => {
                eprintln!("Failed to load image: {e}");
                return;
            }
        };

        let (width, height) = img.dimensions();

        // ColorImage is egui's CPU-side image type. It expects a flat Vec
        // of Color32 values (one per pixel). We convert from the raw RGBA
        // bytes by chunking into groups of 4 bytes and building Color32s.
        let pixels: Vec<egui::Color32> = img
            .chunks_exact(4)  // Split flat byte array into [R, G, B, A] groups
            .map(|p| egui::Color32::from_rgba_unmultiplied(p[0], p[1], p[2], p[3]))
            .collect();

        let color_image = ColorImage {
            size: [width as usize, height as usize],
            pixels,
        };

        // Upload the image to the GPU as a texture.
        // TextureOptions::LINEAR gives smooth scaling when the image is
        // scaled up or down, unlike the nearest-neighbor we had before.
        // The string "current_image" is just a debug label for the texture.
        self.texture = Some(ctx.load_texture(
            "current_image",
            color_image,
            egui::TextureOptions::LINEAR,
        ));
    }

    // Navigate to the next image, wrapping around from last to first.
    fn go_next(&mut self, ctx: &egui::Context) {
        if self.images.is_empty() { return; }
        self.current_index = (self.current_index + 1) % self.images.len();
        self.zoom = 1.0;
        self.load_texture(ctx);
    }

    // Navigate to the previous image, wrapping around from first to last.
    fn go_prev(&mut self, ctx: &egui::Context) {
        if self.images.is_empty() { return; }
        self.current_index = self.current_index
            .checked_sub(1)
            .unwrap_or(self.images.len() - 1);
        self.zoom = 1.0;
        self.load_texture(ctx);
    }

    // Build the title string showing filename and position, e.g. "cat.jpg (3/12)".
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

// The eframe::App trait is what makes our struct an eframe application.
// We only need to implement one method: `update`, which is called every frame.
// This is where all UI layout and image rendering happens.
impl eframe::App for Gallerust {
    fn update(&mut self, ctx: &egui::Context, _frame: &mut eframe::Frame) {

        // ── Bottom toolbar panel ─────────────────────────────────────────────
        // Panels in egui are declared first and claim space from the edges
        // inward. We declare the bottom panel before the central panel so
        // the central panel fills whatever space remains above it.
        egui::TopBottomPanel::bottom("toolbar")
            .exact_height(48.0)
            .show(ctx, |ui| {
                // horizontal_centered lays out widgets in a row, centered vertically
                ui.horizontal_centered(|ui| {

                    // Open button — lets the user pick a new image at any time
                    if ui.button("📂 Open").clicked() {
                        self.open_file(ctx);
                    }

                    ui.separator();

                    // Previous/next buttons. We pass ctx so load_texture can
                    // upload the new image to the GPU.
                    if ui.button("◀ Prev").clicked() {
                        self.go_prev(ctx);
                    }
                    if ui.button("Next ▶").clicked() {
                        self.go_next(ctx);
                    }

                    ui.separator();

                    // Filename and position indicator in the middle
                    ui.label(self.title());

                    ui.separator();

                    // Zoom slider on the right side.
                    // egui::Slider mutates state.zoom directly through a
                    // mutable reference — no callback needed.
                    ui.label("Zoom:");
                    ui.add(
                        egui::Slider::new(&mut self.zoom, 0.1..=5.0)
                            .step_by(0.1)       // Snap to 0.1 increments
                            .fixed_decimals(1)  // Show one decimal place
                    );

                    // Reset zoom button
                    if ui.button("↺").on_hover_text("Reset zoom").clicked() {
                        self.zoom = 1.0;
                    }
                });
            });

        // ── Central panel (image display area) ──────────────────────────────
        // CentralPanel fills all remaining space after panels have claimed
        // their portions. This is where the image is drawn.
        egui::CentralPanel::default()
            // Set the background to black, matching our old black letterbox bars
            .frame(egui::Frame::none().fill(egui::Color32::BLACK))
            .show(ctx, |ui| {
                if let Some(texture) = &self.texture {
                    // Get the available space in the central panel
                    let available = ui.available_size();

                    // Calculate the scaled image size, preserving aspect ratio.
                    // This replicates the fit-to-window logic from our old draw_image().
                    let img_size = texture.size_vec2();  // Native image dimensions as Vec2
                    let scale_x = available.x / img_size.x;
                    let scale_y = available.y / img_size.y;
                    let base_scale = scale_x.min(scale_y);  // Fit scale
                    let final_scale = base_scale * self.zoom;

                    let display_size = egui::vec2(
                        img_size.x * final_scale,
                        img_size.y * final_scale,
                    );

                    // Center the image in the available space by adding padding.
                    // We calculate how much empty space is left after the image
                    // and split it evenly on both sides.
                    let padding_x = ((available.x - display_size.x) / 2.0).max(0.0);
                    let padding_y = ((available.y - display_size.y) / 2.0).max(0.0);

                    ui.add_space(padding_y);  // Push image down from the top

                    ui.horizontal(|ui| {
                        ui.add_space(padding_x);  // Push image in from the left

                        // Finally, draw the image as an egui Image widget.
                        // egui handles uploading to GPU, scaling, and rendering.
                        ui.add(
                            egui::Image::new(texture)
                                .fit_to_exact_size(display_size)
                        );
                    });

                } else {
                    // No image loaded yet — show a centered prompt
                    ui.centered_and_justified(|ui| {
                        ui.label("Click '📂 Open' to select an image");
                    });
                }

                // Handle keyboard input for navigation and zoom.
                // We check for key presses here because egui processes
                // input through the context rather than separate events.
                if ctx.input(|i| i.key_pressed(egui::Key::ArrowRight)) {
                    self.go_next(ctx);
                }
                if ctx.input(|i| i.key_pressed(egui::Key::ArrowLeft)) {
                    self.go_prev(ctx);
                }
                if ctx.input(|i| i.key_pressed(egui::Key::Plus) || i.key_pressed(egui::Key::Equals)) {
                    self.zoom = (self.zoom + 0.1).min(5.0);
                }
                if ctx.input(|i| i.key_pressed(egui::Key::Minus)) {
                    self.zoom = (self.zoom - 0.1).max(0.1);
                }

                // Scroll wheel zoom — egui exposes scroll delta through ctx.input
                let scroll = ctx.input(|i| i.smooth_scroll_delta.y);
                if scroll != 0.0 {
                    self.zoom = (self.zoom + scroll * 0.001).clamp(0.1, 5.0);
                }
            });
    }
}