use pixels::{Pixels, SurfaceTexture};
use winit::{
    dpi::LogicalSize,
    event::{Event, KeyEvent, WindowEvent, MouseScrollDelta},
    event_loop::{ControlFlow, EventLoop},
    keyboard::{KeyCode, PhysicalKey},
    window::WindowBuilder,
};
use std::sync::Arc;
use std::path::PathBuf;
use rfd::FileDialog;

const WINDOW_WIDTH: u32 = 1280;
const WINDOW_HEIGHT: u32 = 720;

struct AppState {
    images: Vec<PathBuf>,
    current_index: usize,
    zoom: f32,
    img_data: Vec<u8>,
    img_width: u32,
    img_height: u32,
}

impl AppState {
    fn new() -> Option<Self> {
        // Open folder picker dialog
        let folder = FileDialog::new().pick_folder()?;

        // Scan folder for supported image files
        let mut images: Vec<PathBuf> = std::fs::read_dir(&folder)
            .ok()?
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
            return None;
        }

        let (img_data, img_width, img_height) = load_image(&images[0]);

        Some(Self {
            images,
            current_index: 0,
            zoom: 1.0,
            img_data,
            img_width,
            img_height,
        })
    }

    fn go_next(&mut self) {
        if self.current_index + 1 < self.images.len() {
            self.current_index += 1;
            self.load_current();
            self.zoom = 1.0;
        }
    }

    fn go_prev(&mut self) {
        if self.current_index > 0 {
            self.current_index -= 1;
            self.load_current();
            self.zoom = 1.0;
        }
    }

    fn zoom_in(&mut self) {
        self.zoom = (self.zoom + 0.1).min(5.0);
    }

    fn zoom_out(&mut self) {
        self.zoom = (self.zoom - 0.1).max(0.1);
    }

    fn load_current(&mut self) {
        let path = self.images[self.current_index].clone();
        let (data, w, h) = load_image(&path);
        self.img_data = data;
        self.img_width = w;
        self.img_height = h;
    }

    fn title(&self) -> String {
        let filename = self.images[self.current_index]
            .file_name()
            .unwrap_or_default()
            .to_string_lossy();
        format!(
            "Gallerust — {} ({}/{})",
            filename,
            self.current_index + 1,
            self.images.len()
        )
    }
}

fn load_image(path: &PathBuf) -> (Vec<u8>, u32, u32) {
    let img = image::open(path).expect("Failed to open image");
    let img = img.to_rgba8();
    let (w, h) = img.dimensions();
    (img.into_raw(), w, h)
}

fn main() {
    let event_loop = EventLoop::new().unwrap();

    let mut state = match AppState::new() {
        Some(s) => s,
        None => {
            eprintln!("No folder selected or no images found.");
            return;
        }
    };

    let window = Arc::new(
        WindowBuilder::new()
            .with_title(state.title())
            .with_inner_size(LogicalSize::new(WINDOW_WIDTH, WINDOW_HEIGHT))
            .build(&event_loop)
            .unwrap()
    );

    let size = window.inner_size();
    let surface_texture = SurfaceTexture::new(size.width, size.height, window.as_ref());
    let mut pixels = Pixels::new(size.width, size.height, surface_texture).unwrap();

    let window_clone = window.clone();

    let _ = event_loop.run(move |event, elwt| {
        elwt.set_control_flow(ControlFlow::Wait);

        match event {
            Event::WindowEvent { event, window_id } if window_id == window_clone.id() => {
                match event {
                    WindowEvent::CloseRequested => elwt.exit(),

                    WindowEvent::Resized(new_size) => {
                        pixels
                            .resize_surface(new_size.width, new_size.height)
                            .unwrap();
                        pixels
                            .resize_buffer(new_size.width, new_size.height)
                            .unwrap();
                        window_clone.request_redraw();
                    }

                    WindowEvent::KeyboardInput {
                        event: KeyEvent {
                            physical_key: PhysicalKey::Code(key),
                            state: winit::event::ElementState::Pressed,
                            ..
                        },
                        ..
                    } => {
                        match key {
                            KeyCode::ArrowRight => {
                                state.go_next();
                                window_clone.set_title(&state.title());
                                window_clone.request_redraw();
                            }
                            KeyCode::ArrowLeft => {
                                state.go_prev();
                                window_clone.set_title(&state.title());
                                window_clone.request_redraw();
                            }
                            KeyCode::Equal | KeyCode::NumpadAdd => {
                                state.zoom_in();
                                window_clone.request_redraw();
                            }
                            KeyCode::Minus | KeyCode::NumpadSubtract => {
                                state.zoom_out();
                                window_clone.request_redraw();
                            }
                            KeyCode::Escape => elwt.exit(),
                            _ => {}
                        }
                    }

                    WindowEvent::MouseWheel { delta, .. } => {
                        let scroll = match delta {
                            MouseScrollDelta::LineDelta(_, y) => y,
                            MouseScrollDelta::PixelDelta(pos) => pos.y as f32 * 0.01,
                        };
                        if scroll > 0.0 {
                            state.zoom_in();
                        } else {
                            state.zoom_out();
                        }
                        window_clone.request_redraw();
                    }

                    WindowEvent::RedrawRequested => {
                        let size = window_clone.inner_size();
                        draw_image(
                            pixels.frame_mut(),
                            &state.img_data,
                            state.img_width,
                            state.img_height,
                            size.width,
                            size.height,
                            state.zoom,
                        );
                        pixels.render().unwrap();
                    }

                    _ => {}
                }
            }

            Event::AboutToWait => {
                window_clone.request_redraw();
            }

            _ => {}
        }
    });
}

fn draw_image(
    frame: &mut [u8],
    img: &[u8],
    img_width: u32,
    img_height: u32,
    frame_width: u32,
    frame_height: u32,
    zoom: f32,
) {
    // Clear frame to black
    for pixel in frame.chunks_exact_mut(4) {
        pixel.copy_from_slice(&[0, 0, 0, 255]);
    }

    // Scale image to fit window, then apply zoom
    let scale_x = frame_width as f32 / img_width as f32;
    let scale_y = frame_height as f32 / img_height as f32;
    let base_scale = scale_x.min(scale_y);
    let scale = base_scale * zoom;

    let scaled_width = (img_width as f32 * scale) as u32;
    let scaled_height = (img_height as f32 * scale) as u32;

    // Center the image in the frame
    let offset_x = ((frame_width as i32 - scaled_width as i32) / 2).max(0) as u32;
    let offset_y = ((frame_height as i32 - scaled_height as i32) / 2).max(0) as u32;

    for y in 0..scaled_height {
        let frame_y = y + offset_y;
        if frame_y >= frame_height {
            break;
        }

        for x in 0..scaled_width {
            let frame_x = x + offset_x;
            if frame_x >= frame_width {
                continue;
            }

            // Map scaled pixel back to source image pixel (nearest neighbor)
            let src_x = (x as f32 / scale) as u32;
            let src_y = (y as f32 / scale) as u32;

            let src_x = src_x.min(img_width - 1);
            let src_y = src_y.min(img_height - 1);

            let src_index = ((src_y * img_width + src_x) * 4) as usize;
            let dst_index = ((frame_y * frame_width + frame_x) * 4) as usize;

            frame[dst_index..dst_index + 4].copy_from_slice(&img[src_index..src_index + 4]);
        }
    }
}