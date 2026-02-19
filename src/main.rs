#![cfg_attr(target_os = "windows", windows_subsystem = "windows")]

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

// Default window dimensions in logical pixels.
// "Logical" means these are DPI-aware units, so on a high-DPI screen
// the actual pixel count may be higher, but the window will appear
// the same physical size regardless of screen density.
const WINDOW_WIDTH: u32 = 1280;
const WINDOW_HEIGHT: u32 = 720;

// AppState holds everything our application needs to remember between frames.
// Because the event loop in winit is driven by OS events, we can't use
// local variables inside the loop to track things like which image we're on.
// Instead, we bundle all mutable state into this struct and pass it into
// the closure so it persists across events.
struct AppState {
    images: Vec<PathBuf>,   // Sorted list of image file paths found in the chosen folder
    current_index: usize,   // Index into `images` pointing to the currently displayed image
    zoom: f32,              // Current zoom multiplier. 1.0 = fit to window, 2.0 = 2x, etc.
    img_data: Vec<u8>,      // Raw RGBA pixel data for the currently loaded image
    img_width: u32,         // Width of the currently loaded image in pixels
    img_height: u32,        // Height of the currently loaded image in pixels
}

impl AppState {
    // AppState::new() is our constructor. It returns Option<Self> rather than Self
    // because several things can fail: the user might cancel the folder picker,
    // or the folder might contain no images. Returning None lets main() handle
    // these cases cleanly without panicking.
    fn new() -> Option<Self> {
        
        let file = FileDialog::new()
            .add_filter("Images", &["jpg", "jpeg", "png", "gif", "webp", "bmp"])
            .pick_file()?;

        // Then derive the folder from the file's parent directory
        let folder = file.parent()?.to_path_buf();

        // Read the contents of the selected folder.
        // `read_dir` returns an iterator of directory entries.
        // We chain several iterator adapters to filter and transform them:
        let mut images: Vec<PathBuf> = std::fs::read_dir(&folder)
            .ok()?                                            // Convert Result to Option, return None on error
            .filter_map(|entry| {           // filter_map keeps only Some values and unwraps them
                let path = entry.ok()?.path();              // Get the full path, skip entries we can't read
                
                // Check the file extension to see if it's a supported image format.
                // We call to_lowercase() so that .JPG and .jpg both match.
                match path.extension()?.to_str()?.to_lowercase().as_str() {
                    "jpg" | "jpeg" | "png" | "gif" | "webp" | "bmp" => Some(path),
                    _ => None, // Non-image files are filtered out by returning None
                }
            })
            .collect();     // Gather all the Some(path) values into a Vec

        // Sort the image paths alphabetically so navigation order is predictable.
        // PathBuf implements Ord, so this sorts by full path string.
        images.sort();

        // If the folder had no supported images, return None so main() can
        // print an error rather than trying to display nothing.
        if images.is_empty() {
            return None;
        }

        // Find the index of the file the user picked so we start on that image
        let current_index = images.iter()
            .position(|p| p == &file)
            .unwrap_or(0);  // Fall back to first image if somehow not found


        // Load the first image immediately so we have something to display
        // as soon as the window opens.
        let (img_data, img_width, img_height) = load_image(&images[current_index]);

        // Return the fully initialized AppState wrapped in Some.
        Some(Self {
            images,
            current_index,
            zoom: 1.0,
            img_data,
            img_width,
            img_height,
        })
    }

    // Advance to the next image in the folder, if there is one.
    // We guard against going past the end of the list with the bounds check.
    fn go_next(&mut self) {
        // If we're at the last image, wrap to the first. Otherwise, advance by 1.
        self.current_index = (self.current_index + 1) % self.images.len();
        self.load_current();
        self.zoom = 1.0;
    }

    fn go_prev(&mut self) {
        // If we're at the first image (index 0), wrap to the last.
        // Otherwise step back by 1.
        // We can't just subtract 1 from a usize at 0 because it would underflow,
        // so we use checked_sub and fall back to the last index if it returns None.
        self.current_index = self.current_index
            .checked_sub(1)
            .unwrap_or(self.images.len() -1);
        self.load_current();
        self.zoom = 1.0;
    }

    // Increase zoom by 10% per step, capped at 5x to prevent runaway scaling.
    fn zoom_in(&mut self) {
        self.zoom = (self.zoom + 0.1).min(5.0);
    }

    // Decrease zoom by 10% per step, floored at 0.1x so the image never
    // disappears entirely.
    fn zoom_out(&mut self) {
        self.zoom = (self.zoom - 0.1).max(0.1);
    }

    // Load the image at current_index from disk and store its data in self.
    // This is called every time the user navigates to a new image.
    // We only keep one image in memory at a time to avoid loading the
    // entire folder upfront, which could use a lot of RAM for large collections.
    fn load_current(&mut self) {
        let path = self.images[self.current_index].clone();
        let (data, w, h) = load_image(&path);
        self.img_data = data;
        self.img_width = w;
        self.img_height = h;
    }

    // Build a window title string that includes the current filename and
    // position in the folder, e.g. "Gallerust — cat.jpg (3/12)".
    // This is called whenever we navigate to update the title bar.
    fn title(&self) -> String {
        // Extract just the filename from the full path.
        // unwrap_or_default() gives us an empty string if there's no filename
        // (which shouldn't happen in practice, but keeps the code safe).
        let filename = self.images[self.current_index]
            .file_name()
            .unwrap_or_default()
            .to_string_lossy();         // Convert OsStr to a regular string, replacing invalid UTF-8
        format!(
            "Gallerust — {} ({}/{})",
            filename,
            self.current_index + 1,     // Add 1 because users expect 1-based counting
            self.images.len()
        )
    }
}

// Load an image from disk and return its raw RGBA pixel data plus dimensions.
// This is a standalone function (not a method) because it's used both during
// AppState construction and when navigating between images.
//
// The `image` crate handles decoding many formats (JPEG, PNG, etc.) for us.
// We always convert to RGBA8 (4 bytes per pixel: red, green, blue, alpha)
// because that's the format `pixels` expects for the framebuffer.
fn load_image(path: &PathBuf) -> (Vec<u8>, u32, u32) {
    let img = image::open(path).expect("Failed to open image");
    let img = img.to_rgba8();   // Convert to RGBA regardless of source format
    let (w, h) = img.dimensions();
    (img.into_raw(), w, h)                                     // into_raw() gives us the underlying Vec<u8>
}

fn main() {
    // The EventLoop is winit's connection to the OS event system.
    // It's responsible for receiving input events, redraw requests,
    // and other OS messages and dispatching them to our closure.
    let event_loop = EventLoop::new().unwrap();

    // Initialize app state. This opens the folder picker dialog and loads
    // the first image. If the user cancels or the folder is empty, we exit.
    let mut state = match AppState::new() {
        Some(s) => s,
        None => {
            eprintln!("No folder selected or no images found.");
            return;     // Exit main(), which cleanly shuts down the app
        }
    };


    // Create the OS window. We wrap it in Arc (Atomic Reference Counting)
    // so that both the SurfaceTexture (which needs a reference to the window
    // to render to it) and the event loop closure (which needs to call
    // request_redraw and set_title) can both hold a reference to the same
    // window without ownership conflicts.
    // Arc works here because winit::Window is Send + Sync.
    let window = Arc::new(
        WindowBuilder::new()
            .with_title(state.title())
            .with_inner_size(LogicalSize::new(WINDOW_WIDTH, WINDOW_HEIGHT))
            .build(&event_loop)
            .unwrap()
    );

    let size = window.inner_size();     // Get the actual pixel size of the window's drawable area

    // SurfaceTexture links the pixels framebuffer to our window.
    // It needs a reference to the window so it knows where to present
    // rendered frames. This is why we needed Arc — SurfaceTexture holds
    // this reference for its entire lifetime.
    let surface_texture = SurfaceTexture::new(size.width, size.height, window.as_ref());
    
    // Pixels manages our raw framebuffer — a grid of RGBA bytes that we write
    // into directly, which it then uploads to the GPU and displays in the window.
    // The buffer size should match the window's drawable area.
    let mut pixels = Pixels::new(size.width, size.height, surface_texture).unwrap();

    // Clone the Arc before moving into the closure. The closure will capture
    // this clone, while the original `window` Arc remains owned by
    // the SurfaceTexture above. Both point to the same underlying Window.
    let window_clone = window.clone();

    // Start the event loop. This call blocks and never returns normally —
    // the app lives entirely inside this closure from here on.
    // `move` transfers ownership of state, pixels, and window_clone into
    // the closure so they live as long as the event loop runs.
    // `elwt` is the EventLoopWindowTarget, used to control flow (exit, wait, etc.)
    let _ = event_loop.run(move |event, elwt| {
        // Tell the event loop to sleep until the next OS event arrives,
        // rather than spinning in a busy loop. This keeps CPU usage low
        // since a photo viewer doesn't need to animate constantly.
        elwt.set_control_flow(ControlFlow::Wait);

        match event {
            // WindowEvent covers all events that are scoped to our specific window:
            // input, resize, close, redraw, etc. We filter by window_id to make
            // sure we're handling events for our window (important if you ever
            // have multiple windows open).
            Event::WindowEvent { event, window_id } if window_id == window_clone.id() => {
                match event {
                    // The user clicked the X button or pressed Alt+F4.
                    WindowEvent::CloseRequested => elwt.exit(),

                    // The window was resized by the user dragging its edge.
                    // We need to tell both the surface and the pixel buffer
                    // about the new size so rendering stays correct.
                    WindowEvent::Resized(new_size) => {
                        pixels
                            .resize_surface(new_size.width, new_size.height)
                            .unwrap();
                        pixels
                            .resize_buffer(new_size.width, new_size.height)
                            .unwrap();
                        window_clone.request_redraw();
                    }

                    // A keyboard key was pressed. We destructure the event to get
                    // the physical key code and check it was a Press (not a Release).
                    // PhysicalKey::Code gives us layout-independent key codes,
                    // so arrow keys work regardless of the user's keyboard language.
                    WindowEvent::KeyboardInput {
                        event: KeyEvent {
                            physical_key: PhysicalKey::Code(key),
                            state: winit::event::ElementState::Pressed,
                            .. // `..` ignores the other fields we don't need
                        },
                        ..
                    } => {
                        match key {
                            KeyCode::ArrowRight => {
                                state.go_next();
                                window_clone.set_title(&state.title());     // Updates title bar
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

                    // Mouse scroll wheel or trackpad scroll.
                    // Two delta types exist because mice report line-based deltas
                    // while trackpads report pixel-based deltas.
                    WindowEvent::MouseWheel { delta, .. } => {
                        let scroll = match delta {
                            // LineDelta: x is horizontal scroll, y is vertical.
                            // Scrolling up gives a positive y value.
                            MouseScrollDelta::LineDelta(_, y) => y,
                            // PixelDelta: raw pixel distance, we scale it down
                            // to get a similar feel to line-based scrolling.
                            MouseScrollDelta::PixelDelta(pos) => pos.y as f32 * 0.01,
                        };
                        if scroll > 0.0 {
                            state.zoom_in();
                        } else {
                            state.zoom_out();
                        }
                        window_clone.request_redraw();
                    }

                    // The OS is asking us to redraw the window.
                    // This fires after we call request_redraw(), but also
                    // whenever the OS needs it (e.g. after the window is
                    // uncovered by another window being moved).
                    WindowEvent::RedrawRequested => {
                        let size = window_clone.inner_size();
                        draw_image(
                            pixels.frame_mut(), // Mutable reference to the raw pixel buffer
                            &state.img_data,
                            state.img_width,
                            state.img_height,
                            size.width,
                            size.height,
                            state.zoom,
                        );
                        pixels.render().unwrap(); // Upload the pixel buffer to the GPU and present it in the window
                    }

                    _ => {}
                }
            }

            // AboutToWait fires when the event loop has processed all pending events
            // and is about to sleep. We use it to schedule a redraw on the next frame.
            // This keeps the display responsive without burning CPU in a busy loop.
            Event::AboutToWait => {
                window_clone.request_redraw();
            }

            _ => {}
        }
    });
}

// Draw the current image into the pixel framebuffer, scaled to fit the window
// and centered with black bars if the aspect ratios don't match.
//
// Parameters:
//   frame       - The raw RGBA pixel buffer managed by `pixels`. We write directly into this.
//   img         - The source image's raw RGBA pixel data.
//   img_width   - Source image width in pixels.
//   img_height  - Source image height in pixels.
//   frame_width - Current window/framebuffer width in pixels.
//   frame_height- Current window/framebuffer height in pixels.
//   zoom        - Current zoom multiplier (1.0 = fit to window).
fn draw_image(
    frame: &mut [u8],
    img: &[u8],
    img_width: u32,
    img_height: u32,
    frame_width: u32,
    frame_height: u32,
    zoom: f32,
) {
    // Fill the entire frame with opaque black before drawing the image.
    // This ensures the letterbox/pillarbox bars are black rather than
    // showing garbage data from a previous frame.
    for pixel in frame.chunks_exact_mut(4) {
        pixel.copy_from_slice(&[0, 0, 0, 255]);
    }

    // Calculate how much to scale the image to fit the window while
    // preserving its aspect ratio. We compute separate scale factors for
    // width and height, then take the smaller one so the image fits in
    // both dimensions without being cropped.
    let scale_x = frame_width as f32 / img_width as f32;
    let scale_y = frame_height as f32 / img_height as f32;
    let base_scale = scale_x.min(scale_y);

    // Apply the user's zoom on top of the fit scale.
    // At zoom = 1.0 the image fits the window exactly.
    // At zoom = 2.0 it's twice as large (and may extend beyond the window edges).
    let scale = base_scale * zoom;

    let scaled_width = (img_width as f32 * scale) as u32;
    let scaled_height = (img_height as f32 * scale) as u32;

    // Center the scaled image within the frame by computing offsets.
    // If the image is narrower than the frame, offset_x > 0 (pillarboxing).
    // If the image is shorter than the frame, offset_y > 0 (letterboxing).
    // max(0) prevents negative offsets if the image is larger than the frame.
    let offset_x = ((frame_width as i32 - scaled_width as i32) / 2).max(0) as u32;
    let offset_y = ((frame_height as i32 - scaled_height as i32) / 2).max(0) as u32;

    // Iterate over every pixel in the scaled image and write it to the frame.
    for y in 0..scaled_height {
        let frame_y = y + offset_y;

        // Stop if we've gone below the bottom of the frame (can happen when zoomed in)
        if frame_y >= frame_height {
            break;
        }

        for x in 0..scaled_width {
            let frame_x = x + offset_x;

            // Skip pixels that fall outside the right edge of the frame
            if frame_x >= frame_width {
                continue;
            }

            // Nearest-neighbor sampling: map each output pixel back to the
            // corresponding source pixel by dividing by the scale factor.
            // This is fast but can look blocky when zoomed in significantly.
            // A future improvement would be bilinear interpolation for smoother scaling.
            let src_x = (x as f32 / scale) as u32;
            let src_y = (y as f32 / scale) as u32;

            // Clamp to image bounds to avoid reading past the end of the buffer.
            // Floating point rounding could otherwise cause an out-of-bounds index
            // on the very last pixel row/column.
            let src_x = src_x.min(img_width - 1);
            let src_y = src_y.min(img_height - 1);

            // Convert 2D (x, y) coordinates to 1D byte indices.
            // Each pixel is 4 bytes (RGBA), so we multiply by 4.
            let src_index = ((src_y * img_width + src_x) * 4) as usize;
            let dst_index = ((frame_y * frame_width + frame_x) * 4) as usize;

            frame[dst_index..dst_index + 4].copy_from_slice(&img[src_index..src_index + 4]);
        }
    }
}