use dioxus::prelude::*;


use chrono::Duration;
use core::marker::PhantomData;
use std::path::PathBuf;
//use dioxus_i18n::{prelude::*, t};
//use dioxus_i18n::unic_langid::langid;

//use unic_langid::langid;
#[cfg(feature = "server")]
use tokio::signal;

#[cfg(feature = "server")]
fn block_on<T>(app_future: impl Future<Output = T>) {
    if let Ok(handle) = tokio::runtime::Handle::try_current() {
        handle.block_on(app_future);
    } else {
        tokio::runtime::Builder::new_multi_thread()
            .enable_all()
            .build()
            .unwrap()
            .block_on(app_future);
    }
}

#[cfg(all(feature = "server",not(debug_assertions)))]
use tokio::net::TcpListener;

use dioxus_logger::tracing::{Level, info};
fn main() {

    //dioxus::LaunchBuilder::new()
    //    .with_cfg(server_only!(ServeConfig::builder().incremental(
    //        dioxus::server::IncrementalRendererConfig::default()
    //            .invalidate_after(std::time::Duration::from_secs(120)),
    //    )))
    //    .launch(app);

    dioxus_logger::init(Level::WARN).expect("logger failed to init");

    #[cfg(not(feature = "server"))]
    dioxus::launch(app);

    #[cfg(feature="server")]
    let service_callback = || async move {

        #[cfg(all(feature="server",debug_assertions))]
        return Ok(dioxus::server::router(app)
            );
        #[cfg(all(feature="server",not(debug_assertions)))]
        return dioxus::server::router(app);
    };

    #[cfg(all(feature="server",not(debug_assertions)))]
    block_on(
        async move {
            let m =  service_callback().await.into_make_service_with_connect_info::<std::net::SocketAddr>();
            let addr = dioxus_cli_config::fullstack_address_or_localhost();
            let listener = TcpListener::bind(addr)
            .await
            .with_context(|| format!("Failed to bind to address {addr}"))
            .unwrap();
            axum::serve(listener,m).with_graceful_shutdown(async {
                signal::ctrl_c().await.expect("failed tol install CTRL-C handler")
            }).await
            .unwrap();
            return ;
        }
    );

    #[cfg(all(feature="server",debug_assertions))]
    dioxus::serve(service_callback);

}

fn app() -> Element {
    rsx! {
        div {
            style: "width: 100vw; height: 100vh; margin: 0; padding: 0;",

            // Dioxus UI overlay
            div {
                style: "position: absolute; top: 12px; left: 12px; z-index: 100; color: white; font-family: monospace; background: rgba(0,0,0,0.7); padding: 10px; border-radius: 5px;",
                "Dioxus + wgpu 3D Demo"
                br {}
                "Controls: Space: Change UVs, X/Y/Z: Rotate, R: Reset"
                br {}
                button {
                    /*onclick: move |_| send_reset_rotation(), */
                    style: "margin-top: 10px; padding: 5px 10px; background: #4CAF50; color: white; border: none; border-radius: 3px; cursor: pointer; margin-right: 5px;",
                    "Reset Rotation (Dioxus)"
                }
                button {
                    /* onclick: move |_| send_toggle_texture(), */
                    style: "margin-top: 10px; padding: 5px 10px; background: #2196F3; color: white; border: none; border-radius: 3px; cursor: pointer;",
                    "Toggle Texture (Dioxus)"
                }
            }
            div {
                // Canvas for wgpu rendering
                WgpuCanvas {}
            }
        }
    }
}

use wgpu_gameui::layout::Rect;
use wgpu_gameui::{DrawList, InputState, Theme, UiRenderer, KeyboardNav, UiState, Frame};
use wgpu_gameui::{
    Button, ClickTracker, CursorIcon, CursorState, DragCapture, DragHandle, DragTracker,
    DrawContext, Dropdown, DropdownState, FocusState, FontHandle, LayerStack,
    ScrollState, ScrollView, StyleResolver, TextAlign, TextBlock, TextInput,  UiContext,
};
use web_sys::wasm_bindgen::{prelude::Closure, JsCast};
use std::cell::{RefCell,RefMut};
use std::rc::Rc;
use web_time::Instant;

#[derive(Default)]
struct UiPersist{
    scroll: ScrollState,
    modal_open: bool,
    modal_button_was_clicked: bool,
    text_input: Rc<RefCell<TextInput>>,
    text_input2: Rc<RefCell<TextInput>>,
    /// Single keyboard-focus owner shared by both text inputs. Tab / Shift-Tab
    /// cycle between them; clicking elsewhere or pressing Esc blurs.
    focus: Rc<RefCell<FocusState>>,
    /// Single open-dropdown owner. Click to open, click an option / outside /
    /// Esc to close.
    dropdowns: DropdownState,
    /// Currently selected dropdown option index.
    dropdown_sel: usize,
    /// Activation count for the two keyboard-operable demo buttons, so the live
    /// example shows Space/Enter activation working.
    button_clicks: u32,
}

#[component]
fn WgpuCanvas() -> Element {
    use_future(move || async move {       
        //#[cfg(target_arch="wasm32")]
        run_wgpu_app().await;
    });

    rsx! {
        canvas {
            id: "wgpu-canvas",
            style: "width: 100%; height: 100%; display: block;",
        }
    }
}

struct RenderState {
    device: Rc<RefCell<wgpu::Device>>,
    queue: wgpu::Queue,
    surface: Rc<RefCell<wgpu::Surface<'static>>>,
    config: Rc<RefCell<wgpu::SurfaceConfiguration>>,
//    render_pipeline: wgpu::RenderPipeline,
//    vertex_buffer: wgpu::Buffer,
//    index_buffer: wgpu::Buffer,
//    num_indices: u32,
//    uniform_buffer: wgpu::Buffer,
//    bind_group: wgpu::BindGroup,
    depth_view: wgpu::TextureView,
//    rotation: Rc<RefCell<glam::Quat>>,
//    uv_offset: Rc<RefCell<f32>>,
    input_state: Rc<RefCell<InputState>>,
    last_time: Rc<RefCell<f64>>,
    /// Cross-frame drag detection feeding `input.is_dragging`/`drag_delta`.
    drag: Rc<RefCell<DragTracker>>,
    /// Top-left of the draggable demo box.
    drag_box: Rect,
    /// Drag-ownership arbiter for the movable box (so it can't fight other
    /// draggables for a single pointer gesture).
    drag_capture:  Rc<RefCell<DragCapture>>,
    /// Double-click and hold detection feeding `input.mouse_double_clicked`/`mouse_held`.
    clicks: Rc<RefCell<ClickTracker>>,
    /// App start time, for wall-clock timestamps fed to `ClickTracker::update`.
    start: Instant,
    ui_persist : Rc<RefCell<UiPersist>>,
    nine_slice_id: u32,
    icon_sprite: u32,
//    canvas: web_sys::HtmlCanvasElement,
}


const CHECKER_SIZE: u32 = 32;

fn checkerboard_pixels(size: u32, a: [u8; 4], b: [u8; 4]) -> Vec<u8> {
    let mut out = Vec::with_capacity((size * size * 4) as usize);
    for y in 0..size {
        for x in 0..size {
            let cell = ((x / 4) + (y / 4)) % 2;
            let c = if cell == 0 { a } else { b };
            out.extend_from_slice(&c);
        }
    }
    out
}

/// Encode a small RGBA gradient as a PNG in memory, so the example can exercise
/// the runtime image-decode path (`UiRenderer::load_image_bytes`) the same way
/// Teardown's `UiImage(path)` loads an encoded file.
fn synth_png(w: u32, h: u32) -> Vec<u8> {
    use image::{ImageFormat, Rgba, RgbaImage};
    let mut img = RgbaImage::new(w, h);
    for y in 0..h {
        for x in 0..w {
            let r = (x * 255 / w.max(1)) as u8;
            let g = (y * 255 / h.max(1)) as u8;
            img.put_pixel(x, y, Rgba([r, g, 160, 255]));
        }
    }
    let mut bytes = Vec::new();
    img.write_to(&mut std::io::Cursor::new(&mut bytes), ImageFormat::Png)
        .expect("encode png");
    bytes
}

fn solid_with_border(size: u32, fill: [u8; 4], border: [u8; 4], thickness: u32) -> Vec<u8> {
    let mut out = vec![0u8; (size * size * 4) as usize];
    for y in 0..size {
        for x in 0..size {
            let on_border =
                x < thickness || y < thickness || x >= size - thickness || y >= size - thickness;
            let c = if on_border { border } else { fill };
            let idx = ((y * size + x) * 4) as usize;
            out[idx..idx + 4].copy_from_slice(&c);
        }
    }
    out
}

/// Focus ids for the two text inputs — any stable, unique `u64` works.
const TEXT_ID_A: u64 = 0;
const TEXT_ID_B: u64 = 1;
/// Focus-independent id for the dropdown.
const DROPDOWN_ID: u64 = 2;
/// Drag-capture id for the movable demo box.
const DRAG_BOX_ID: u64 = 3;
/// Focus ids for the two keyboard-operable demo buttons (Tab to reach them,
/// Space/Enter to activate).
const BTN_A_ID: u64 = 4;
const BTN_B_ID: u64 = 5;

const DROPDOWN_ITEMS: [&str; 5] = ["Fire", "Water", "Earth", "Air", "Aether"];

//#[cfg(target_arch="wasm32")]
async fn run_wgpu_app() {

    let web_window = web_sys::window().expect("no window");

    let web_document = web_window.document().expect("no document");

    let web_canvas = web_document
        .get_element_by_id("wgpu-canvas")
        .expect("no canvas")
        .dyn_into::<web_sys::HtmlCanvasElement>()
        .expect("not a canvas");
    /* 
    
    use winit::platform::web::WindowAttributesExtWebSys;
    
    let attrs = Window::default_attributes()
        .with_canvas(web_canvas.clone().into())
        .with_title("hello_ui — wgpu-gameui")
        //.with_inner_size(winit::dpi::LogicalSize::new(800.0, 480.0))
        ;
      
    let attrs = Window::default_attributes()
        .with_title("hello_ui — wgpu-gameui")
        .with_inner_size(winit::dpi::LogicalSize::new(800.0, 480.0))
        ;
    */
    let instance = wgpu::Instance::new(wgpu::InstanceDescriptor::new_without_display_handle());
    #[cfg(target_arch="wasm32")]
    let surface = instance
        .create_surface(wgpu::SurfaceTarget::Canvas(web_canvas.clone()))
        .expect("create surface");
    #[cfg(not(target_arch="wasm32"))]
    let window = std::sync::Arc::new(winit::event_loop::EventLoop::builder().build().unwrap().create_window(winit::window::Window::default_attributes()
        .with_title("hello_ui — wgpu-gameui")
        .with_inner_size(winit::dpi::LogicalSize::new(800.0, 800.0))).expect("create window"));
    #[cfg(not(target_arch="wasm32"))]
    let surface = instance
        .create_surface(window.clone())
        .expect("create surface");
    
    let surface = Rc::new(RefCell::new(surface));
    let surface_clone = surface.clone();
    let surface_clone1 = surface_clone.borrow_mut();
    let adapter = instance.request_adapter(&wgpu::RequestAdapterOptions {
        power_preference: wgpu::PowerPreference::default(),
        compatible_surface: Some(&surface_clone1),
        force_fallback_adapter: false,
        apply_limit_buckets: false,
    }).await
    .expect("request adapter");
    
    let (device, queue) = adapter.request_device(
        &wgpu::DeviceDescriptor {
            label: Some("hello_ui device"),
            ..Default::default()
        }.into()
    ).await
    .expect("request device");

    let device = Rc::new(RefCell::new(device));
    let device_clone = device.clone();
    let device_clone1 = device_clone.borrow_mut();

    // Get device limits for clamping surface size


    let surface_caps = surface_clone1.get_capabilities(&adapter);
     
    let surface_format = surface_caps
        .formats
        .iter()
        .copied()
        .find(|f| f.is_srgb())
        .unwrap_or(surface_caps.formats[0]);

    // Set canvas size to match display, clamped to device limits
    let limits = device_clone1.limits();
    let max_texture_size = limits.max_texture_dimension_2d;

    let width = (web_canvas.client_width() as u32).min(max_texture_size).max(1);
    let height = (web_canvas.client_height() as u32).min(max_texture_size).max(1);
    web_canvas.set_width(width);
    web_canvas.set_height(height);
    
    let config = Rc::new(RefCell::new(wgpu::SurfaceConfiguration {
        usage: wgpu::TextureUsages::RENDER_ATTACHMENT,
        format: surface_format,
        width,
        height,
        present_mode: wgpu::PresentMode::AutoVsync,
        alpha_mode: surface_caps.alpha_modes[0],
        view_formats: vec![],
        desired_maximum_frame_latency: 2,
        color_space: wgpu::SurfaceColorSpace::Auto,
    }));
    let config_clone = config.clone();
    let mut config_clone1 = config_clone.borrow_mut();
    surface_clone1.configure(&device_clone1, &config_clone1);

    let font_system = wgpu_gameui::shared_font_system();
    let font_for_loading = font_system.clone();
    
    let mut ui_renderer = UiRenderer::new(&device_clone1, &queue, surface_format, font_system);
    
    let theme = Theme::default();
    
    let custom_font = wgpu_gameui::load_font_bytes(&font_for_loading, notosans::REGULAR_TTF)
        .expect("load custom font");
    let png = synth_png(64, 64);
    let image_sprite = ui_renderer
        .load_image_bytes("demo_gradient", &png)
        .expect("decode demo image");

    let depth_texture = create_depth_texture(&device_clone1, &config_clone1);
    let depth_view = depth_texture.create_view(&wgpu::TextureViewDescriptor::default());

    // Start render loop
    let f = Rc::new(RefCell::new(None::<Closure<dyn FnMut()>>));
    let g = f.clone();

    let text_input = Rc::new(RefCell::new(TextInput::default()));
    let text_input_clone=  text_input.clone();
    let text_input2 = Rc::new(RefCell::new(TextInput::default()));

    let text_input2_clone=  text_input2.clone(); 
    let focus = Rc::new(RefCell::new(FocusState::default()));
    let focus_clone = focus.clone();
    let ui_persist = Rc::new(RefCell::new(UiPersist{
        text_input,
        text_input2,
        focus,
        ..Default::default()
    }));
    let ui_persist_clone = ui_persist.clone();

    //let mut input = InputState::default();

    let input_state = Rc::new(RefCell::new(InputState::new()));
    let input_state_clone = input_state.clone();
    let input_state_clone2 = input_state.clone();
    let input_state_clone3 = input_state.clone();
    let input_state_clone4 = input_state.clone();
    let last_time = Rc::new(RefCell::new(web_window.performance().unwrap().now() / 1000.0));

    let drag= Rc::new(RefCell::new(DragTracker::new()));
    let drag_clone = drag.clone();
    let drag_box = Rect::new(340.0, 360.0, 150.0, 56.0);
    let drag_capture = Rc::new(RefCell::new(DragCapture::new()));
    let drag_capture_clone = drag_capture.clone();
    let clicks = Rc::new(RefCell::new(ClickTracker::new()));
    let clicks_clone = clicks.clone(); 
    let start = Instant::now();  

    // Upload some sprites.
    let icon_pixels = checkerboard_pixels(CHECKER_SIZE, [220, 80, 80, 255], [40, 40, 40, 255]);
    let icon_sprite = ui_renderer.load_sprite_rgba8("icon", CHECKER_SIZE, CHECKER_SIZE, &icon_pixels);

    let frame_pixels = solid_with_border(32, [180, 180, 200, 255], [60, 60, 90, 255], 4);
    let frame_sprite = ui_renderer.load_sprite_rgba8("frame", 32, 32, &frame_pixels);
    let nine_slice_id = ui_renderer.register_nine_slice("frame", frame_sprite, [4, 4, 4, 4]);



    // Render state to be moved into render loop
    let render_state = Rc::new(RefCell::new(RenderState {
        device:device.clone(),
        queue,
        surface:surface.clone(),
        config: config.clone(),
//        render_pipeline,
//        vertex_buffer,
//        index_buffer,
//        num_indices,
//        uniform_buffer,
//        bind_group,
        depth_view,
//        rotation,
//        uv_offset,
        input_state,
        last_time,
        drag,

        drag_box,

        drag_capture,

        clicks,
        start,
        ui_persist,
        nine_slice_id,
        icon_sprite,
//        canvas: web_canvas,
    }));


    let render_state_clone = render_state.clone();
    let surface_clone = surface.clone();
    let device_clone = device.clone();
    let config_clone = config.clone();
    let canvas_resize_closure = Closure::wrap(Box::new (move |_entries: web_sys::wasm_bindgen::JsValue| {

        // Set canvas size to match display, clamped to device limits
        //let mut state :RefMut<RenderState> = render_state_clone.borrow_mut();

        let mut surface_clone1 = surface_clone.borrow_mut();
        let mut device_clone1=device_clone.borrow_mut();
        let mut config_clone1 = config_clone.borrow_mut();
        let web_window = web_sys::window().expect("no window");

        let web_document = web_window.document().expect("no document");

        let web_canvas = web_document
            .get_element_by_id("wgpu-canvas")
            .expect("no canvas")
            .dyn_into::<web_sys::HtmlCanvasElement>()
            .expect("not a canvas");

        let limits = device_clone1.limits();
        let max_texture_size = limits.max_texture_dimension_2d;

        let width = (web_canvas.client_width() as u32).min(max_texture_size).max(1);
        let height = (web_canvas.client_height() as u32).min(max_texture_size).max(1);
        web_canvas.set_width(width);
        web_canvas.set_height(height);
        //let mut config_clone1= config_clone.borrow_mut();
        config_clone1.width=width;
        config_clone1.height=height;
        /*
        let config = wgpu::SurfaceConfiguration {
            usage: wgpu::TextureUsages::RENDER_ATTACHMENT,
            format: surface_format,
            width,
            height,
            present_mode: wgpu::PresentMode::AutoVsync,
            alpha_mode: surface_caps.alpha_modes[0],
            view_formats: vec![],
            desired_maximum_frame_latency: 2,
            color_space: wgpu::SurfaceColorSpace::Auto,
        };
        */
        surface_clone1.configure(&device_clone1, &config_clone1);

    }) as Box<dyn FnMut(_)>);
    let observe_canvas_resize:web_sys::ResizeObserver =web_sys::ResizeObserver::new(canvas_resize_closure.as_ref().unchecked_ref()).unwrap();
    observe_canvas_resize.observe(&web_canvas);
    canvas_resize_closure.forget();

    // Create a closure to handle mouse clicks
    let mouse_up_closure = Closure::wrap(Box::new(move |event: web_sys::MouseEvent| {
        let web_window = web_sys::window().expect("no window");
         
        let web_document = web_window.document().expect("no document");
    
        let web_canvas = web_document
            .get_element_by_id("wgpu-canvas")
            .expect("no canvas");

        let mut input_state_clone= input_state_clone4.borrow_mut();
        // Access MouseEvent properties
        let x  =  event.client_x() as f32;
        let y =  event.client_y() as f32;
        let rect = web_canvas.get_bounding_client_rect();
        input_state_clone.mouse_x =x - rect.left() as f32;
        input_state_clone.mouse_y = y - rect.top() as f32;;
        let left_button =  (event.buttons() & 1u16) != 0u16;
        let right_button = (event.buttons() & 2u16) != 0u16;
        let wheel_button  = (event.buttons() & 4u16) != 0u16;
        //let backward_button = (event.buttons() & 8u16) != 0u16;
        //let forward_button = (event.buttons() & 16u16) != 0u16;

        if !left_button {
            input_state_clone.mouse_down = false;
            input_state_clone.mouse_released = true;
        }

        if !right_button {
            input_state_clone.mouse_right_down = false;
            input_state_clone.mouse_right_released = true;
        }
        if !wheel_button {
            input_state_clone.mouse_middle_down = false;
            input_state_clone.mouse_middle_released = true;
        }
        
        //web_sys::console::log_1(&format!("Mouse up at: ({}, {})", x, y).into());
    }) as Box<dyn FnMut(_)>);
    web_canvas.add_event_listener_with_callback("mouseup", mouse_up_closure.as_ref().unchecked_ref()).unwrap();
    mouse_up_closure.forget();

        // Create a closure to handle mouse clicks
    let mouse_down_closure = Closure::wrap(Box::new(move |event: web_sys::MouseEvent| {

        let web_window = web_sys::window().expect("no window");
     
        let web_document = web_window.document().expect("no document");
    
        let web_canvas = web_document
            .get_element_by_id("wgpu-canvas")
            .expect("no canvas");

        let mut input_state_clone= input_state_clone3.borrow_mut();
        // Access MouseEvent properties
        let x  =  event.client_x() as f32;
        let y =  event.client_y() as f32;
        let rect = web_canvas.get_bounding_client_rect();
        input_state_clone.mouse_x =x - rect.left() as f32;
        input_state_clone.mouse_y = y - rect.top() as f32;;

        let left_button =  (event.buttons() & 1u16) != 0u16;
        let right_button = (event.buttons() & 2u16) != 0u16;
        let wheel_button  = (event.buttons() & 4u16) != 0u16;
        //let backward_button = (event.buttons() & 8u16) != 0u16;
        //let forward_button = (event.buttons() & 16u16) != 0u16;

        if left_button {
            input_state_clone.mouse_down = true;
            input_state_clone.mouse_clicked = true;
        }

        if right_button {
            input_state_clone.mouse_right_down = true;
            input_state_clone.mouse_right_clicked = true;
        }
        if wheel_button {
            input_state_clone.mouse_middle_down = true;
            input_state_clone.mouse_middle_clicked = true;
        }
        
        //web_sys::console::log_1(&format!("Mouse down at: ({}, {})", x, y).into());
    }) as Box<dyn FnMut(_)>);
    web_canvas.add_event_listener_with_callback("mousedown", mouse_down_closure.as_ref().unchecked_ref()).unwrap();
    mouse_down_closure.forget();


    let mouse_move_closure = Closure::wrap(Box::new(move |event: web_sys::MouseEvent| {

        let web_window = web_sys::window().expect("no window");
     
        let web_document = web_window.document().expect("no document");
    
        let web_canvas = web_document
            .get_element_by_id("wgpu-canvas")
            .expect("no canvas");

        let mut input_state_clone= input_state_clone2.borrow_mut();
        // Access MouseEvent properties
        let x  =  event.client_x() as f32;
        let y =  event.client_y() as f32;
        let rect = web_canvas.get_bounding_client_rect();
        input_state_clone.mouse_x =x - rect.left() as f32;
        input_state_clone.mouse_y = y - rect.top() as f32;;

        let left_button =  (event.buttons() & 1u16) != 0u16;
        let right_button = (event.buttons() & 2u16) != 0u16;
        let wheel_button  = (event.buttons() & 4u16) != 0u16;
        //let backward_button = (event.buttons() & 8u16) != 0u16;
        //let forward_button = (event.buttons() & 16u16) != 0u16;

        input_state_clone.mouse_down = left_button;

        input_state_clone.mouse_right_down = right_button;

        input_state_clone.mouse_middle_down = wheel_button;
    
        //web_sys::console::log_1(&format!("Mouse moved to: ({}, {})", x, y).into());
    }) as Box<dyn FnMut(_)>);
    web_canvas.add_event_listener_with_callback("mousemove", mouse_move_closure.as_ref().unchecked_ref()).unwrap();
    mouse_move_closure.forget();


    let render_state_clone = render_state.clone();
    let surface_clone = surface.clone();
    let device_clone = device.clone();
    let config_clone = config.clone();

    *g.borrow_mut() = Some(Closure::wrap(Box::new(move || {

        let mut state :RefMut<RenderState> = render_state_clone.borrow_mut();

        let surface_clone1 = surface_clone.borrow_mut();
        let device_clone1=device_clone.borrow_mut();
        let config_clone1 = config_clone.borrow_mut();
        //let state_2= render_state_clone2.borrow();
        //let mut state_3 = render_state_clone3.borrow_mut();
        //let mut state_4 = render_state_clone4.borrow_mut();
        let frame = match surface_clone1.get_current_texture() {
            wgpu::CurrentSurfaceTexture::Success(f) => f,
            wgpu::CurrentSurfaceTexture::Suboptimal(_) => {
                surface_clone1.configure(&device_clone1, &config_clone1);
                request_animation_frame(f.borrow().as_ref().unwrap());
                return;                
            },
            wgpu::CurrentSurfaceTexture::Lost | wgpu::CurrentSurfaceTexture::Outdated=> {
                surface_clone1.configure(&device_clone1, &config_clone1);
                request_animation_frame(f.borrow().as_ref().unwrap());
                return;
            },
            _ => {
                //tracing::warn!("Surface error: {:?}", e);
                request_animation_frame(f.borrow().as_ref().unwrap());
                return;
            }
        };
        let view = frame
            .texture
            .create_view(&wgpu::TextureViewDescriptor::default());

        let t = state.start.elapsed().as_secs_f64();
        let mut input_state_clone = input_state_clone.borrow_mut();
        let mut drag_clone = drag_clone.borrow_mut();
        let mut clicks_clone = clicks_clone.borrow_mut();
        drag_clone.update(&mut input_state_clone);
        clicks_clone.update(&mut input_state_clone, t);

        wgpu_gameui::map_keyboard(&mut input_state_clone);


        let mut ui_persist_clone = ui_persist_clone.borrow_mut();

        // Per-frame cursor accumulator: hovered widgets request an icon
        // through their DrawContext; we apply the winner to the window
        // after drawing. Fresh each frame.
        let mut frame_cursor = CursorState::new();

        // Build a LayerStack so we can demo modal layers.
        let mut layers = LayerStack::new();

        // Push the modal first (if open) so layer dispatch sees the
        // full z-order when computing `input_for_base`.
        let modal_rect = Rect::new(220.0, 100.0, 360.0, 200.0);
        let modal_idx = if ui_persist_clone.modal_open {
            Some(layers.push_modal(modal_rect))
        } else {
            None
        };


        // Establish the open dropdown's popup layer at frame-top (from
        // last frame's geometry) so `input_for_base` blocks clicks to
        // widgets under the open list — same as the modal above.
        ui_persist_clone.dropdowns.begin_frame(&input_state_clone);
        let dropdown_popup = ui_persist_clone.dropdowns.push_open_layer(&mut layers);

        // Resolve input for the base layer. When a modal is open this
        // sets `mouse_consumed = true` so base widgets can't fire.
        let mut base_input = layers.input_for_base(&input_state_clone);

        let list = layers.base_mut();

        // Background nine-slice panel.
        list.nine_slice_id(
            state.nine_slice_id,
            40.0,
            40.0,
            480.0,
            260.0,
            [1.0, 1.0, 1.0, 1.0],
        );
        list.rounded_rect(
            Rect::new(60.0, 60.0, 440.0, 220.0),
            8.0,
            [0.12, 0.14, 0.20, 0.95],
        );

        // "Open Modal" button rect.
        let open_btn = Rect::new(80.0, 220.0, 160.0, 40.0);
        let btn_hovered = !base_input.mouse_consumed
            && open_btn.contains(base_input.mouse_x, base_input.mouse_y);
        let btn_color = if btn_hovered {
            [0.40, 0.65, 0.95, 1.0]
        } else {
            [0.30, 0.55, 0.85, 1.0]
        };
        list.rounded_rect(open_btn, 6.0, btn_color);
        if btn_hovered && base_input.mouse_clicked {
            ui_persist_clone.modal_open = true;
            ui_persist_clone.modal_button_was_clicked = true;
        }

        list.icon_sprite(
            state.icon_sprite,
            260.0,
            220.0,
            40.0,
            40.0,
            [1.0, 1.0, 1.0, 1.0],
        );

        // Title: MSDF outline demo (white fill, dark 2px outline).
        list.text(
            TextBlock::new("hello_ui — wgpu-gameui", 80.0, 80.0)
                .with_size(22.0)
                .with_color(255, 255, 255)
                .with_outline(10, 12, 18, 255, 2.0),
        );
        // Subtitle: MSDF drop-shadow demo.
        list.text(
            TextBlock::new("Click 'Open Modal' to demo the layer system", 80.0, 120.0)
                .with_size(14.0)
                .with_color(200, 210, 230)
                .with_shadow(0, 0, 0, 180, 1.0, 1.0, 1.0),
        );
        list.text(
            TextBlock::new("Open Modal", 110.0, 232.0)
                .with_size(16.0)
                .with_color(255, 255, 255),
        );

        // ---------- ScrollView demo ----------
        // 30 rows of overflowing content in a 200x240 viewport.
        let scroll_viewport = Rect::new(560.0, 60.0, 200.0, 240.0);
        list.rounded_rect(scroll_viewport, 6.0, [0.06, 0.07, 0.10, 1.0]);
        ui_persist_clone.scroll.content_size = [180.0, 30.0 * 24.0];

        ScrollView::new(scroll_viewport).vertical_only().draw(
            &mut ui_persist_clone.scroll,
            list,
            &StyleResolver::new(&theme),
            &mut base_input,
            |list, vp| {
                for i in 0..30usize {
                    let y = vp.y + i as f32 * 24.0;
                    let bg = if i % 2 == 0 {
                        [0.16, 0.18, 0.24, 1.0]
                    } else {
                        [0.10, 0.12, 0.18, 1.0]
                    };
                    list.quad(vp.x + 4.0, y + 2.0, vp.width - 12.0, 20.0, bg);
                    list.text(
                        TextBlock::new(&format!("Row #{:02}", i), vp.x + 12.0, y + 4.0)
                            .with_size(14.0)
                            .with_color(200, 210, 230),
                    );
                }
            },
        );

        let mut text_input_clone = text_input_clone.borrow_mut();
        let mut text_input2_clone = text_input2_clone.borrow_mut();
        let mut focus_clone = focus_clone.borrow_mut();
        // ---------- Text input demo ----------
        // Two inputs sharing one FocusState: click to focus, Tab /
        // Shift-Tab to cycle, click empty space or press Esc to blur.
        // Focus arbitration is resolved in `focus.end_frame()` below.
        {
            focus_clone.begin_frame(&base_input);

            // Position the two text inputs.
            text_input_clone.x = 80.0;
            text_input_clone.y = 300.0;
            text_input_clone.width = 240.0;
            text_input_clone.height = 28.0;
            text_input2_clone.x = 80.0;
            text_input2_clone.y = 336.0;
            text_input2_clone.width = 240.0;
            text_input2_clone.height = 28.0;

            // Background label for the inputs.
            list.text(
                TextBlock::new("Text Input (Tab to cycle):", 80.0, 280.0)
                    .with_size(12.0)
                    .with_color(180, 190, 210),
            );
            {
                let mut ctx = DrawContext::new(
                    list,
                    &mut focus_clone,
                    &theme,
                    &base_input,
                    config_clone1.width as f32,
                    config_clone1.height as f32,
                )
                .with_cursor(&mut frame_cursor);
                text_input_clone.draw(TEXT_ID_A, &mut ctx);
                text_input2_clone.draw(TEXT_ID_B, &mut ctx);
            }

            // Two keyboard-operable buttons sharing the same focus ring
            // as the text inputs: Tab/Shift-Tab cycle through inputs and
            // buttons; Space/Enter activates a focused button.
            {
                list.text(
                    TextBlock::new(
                        &format!(
                            "Buttons (Tab to focus, Space/Enter to click): {} clicks",
                            ui_persist_clone.button_clicks
                        ),
                        80.0,
                        372.0,
                    )
                    .with_size(12.0)
                    .with_color(180, 190, 210),
                );

                let mut ctx = DrawContext::new(
                    list,
                    &mut focus_clone,
                    &theme,
                    &base_input,
                    config_clone1.width as f32,
                    config_clone1.height as f32,
                )
                .with_cursor(&mut frame_cursor);

                if Button::new("Click A")
                    .focusable(BTN_A_ID)
                    .draw(Rect::new(80.0, 390.0, 100.0, 28.0), &mut ctx)
                {
                    ui_persist_clone.button_clicks += 1;
                }
                if Button::new("Click B")
                    .focusable(BTN_B_ID)
                    .draw(Rect::new(190.0, 390.0, 100.0, 28.0), &mut ctx)
                {
                    ui_persist_clone.button_clicks += 1;
                }
            }

            focus_clone.end_frame(None);
        }

        // ---------- Dropdown demo ----------
        // The button draws inline here; the open option list is drawn
        // after the base scope (into the popup pushed at frame-top).
        {
            list.text(
                TextBlock::new("Dropdown:", 340.0, 280.0)
                    .with_size(12.0)
                    .with_color(180, 190, 210),
            );
            let dd_rect = Rect::new(340.0, 300.0, 150.0, 28.0);
            let mut ctx = DrawContext::new(
                list,
                &mut focus_clone,
                &theme,
                &base_input,
                config_clone1.width as f32,
                config_clone1.height as f32,
            )
            .with_cursor(&mut frame_cursor);
            Dropdown::new(&DROPDOWN_ITEMS, ui_persist_clone.dropdown_sel).draw(
                DROPDOWN_ID,
                dd_rect,
                &mut ui_persist_clone.dropdowns,
                &mut ctx,
            );
        }

        // ---------- Image + custom font + alignment demo ----------
        // Decoded image drawn at full size, then the same image cropped
        // to its top-left quarter (UV [0,0,0.5,0.5]) stretched to match
        list.image(
            image_sprite,
            Rect::new(80.0, 520.0, 64.0, 64.0),
            [1.0, 1.0, 1.0, 1.0],
        );
        list.image_cropped(
            image_sprite,
            Rect::new(152.0, 520.0, 64.0, 64.0),
            [0.0, 0.0, 0.5, 0.5],
            [1.0, 1.0, 1.0, 1.0],
        );
        // A line shaped in the runtime-loaded custom font.
        list.text(
            TextBlock::new("Custom font: Noto Sans", 232.0, 522.0)
                .with_size(16.0)
                .with_color(255, 228, 160)
                .with_font(custom_font.clone()),
        );
        // Center- and right-aligned lines within a 300px-wide box.
        list.text(
            TextBlock::new("centered in 300px", 232.0, 548.0)
                .with_size(14.0)
                .with_color(200, 210, 230)
                .with_max_width(300.0)
                .with_align(TextAlign::Center),
        );
        list.text(
            TextBlock::new("right-aligned in 300px", 232.0, 566.0)
                .with_size(14.0)
                .with_color(200, 210, 230)
                .with_max_width(300.0)
                .with_align(TextAlign::Right),
        );
        // Bold + italic, using the bundled default sans-serif (real faces).
        list.text(
            TextBlock::new("Bold", 232.0, 588.0)
                .with_size(16.0)
                .with_color(255, 255, 255)
                .bold(),
        );
        list.text(
            TextBlock::new("Italic", 290.0, 588.0)
                .with_size(16.0)
                .with_color(255, 255, 255)
                .italic(),
        );

        // A small UiContext font-stack snippet: push a bold 18px font,
        // draw a line via `text_line`, then pop back to the default.
        let stack_font = custom_font.clone();
        {
            let mut ui = UiContext::new(list);
            ui.push();
            ui.translate(232.0, 610.0);
            ui.rotate(270.0f32.to_radians());
            ui.font(stack_font, 18.0);
            ui.bold(true);
            ui.text_line("font-stack: pushed bold", [0.7, 0.9, 1.0, 1.0]);
            ui.pop();
        }

        // ---------- Modal demo ----------
        if let Some(idx) = modal_idx {
            // Resolve input for THIS layer.
            let modal_input = layers.input_for_layer(idx, &input_state_clone);

            let m = &mut layers.layers_mut()[idx].list;
            // Dim the background with a full-screen scrim.
            m.quad(
                0.0,
                0.0,
                config_clone1.width as f32,
                config_clone1.height as f32,
                [0.0, 0.0, 0.0, 0.55],
            );
            m.rounded_rect(modal_rect, 8.0, [0.18, 0.20, 0.26, 1.0]);
            m.text(
                TextBlock::new("Modal Dialog", modal_rect.x + 16.0, modal_rect.y + 16.0)
                    .with_size(20.0)
                    .with_color(255, 255, 255),
            );
            m.text(
                TextBlock::new(
                    "Lower layers can't be hovered while this is open.",
                    modal_rect.x + 16.0,
                    modal_rect.y + 56.0,
                )
                .with_size(14.0)
                .with_color(200, 210, 230)
                .with_max_width(modal_rect.width - 32.0),
            );
            let close_btn = Rect::new(
                modal_rect.x + modal_rect.width - 110.0,
                modal_rect.y + modal_rect.height - 50.0,
                90.0,
                34.0,
            );
            let close_hover = close_btn.contains(modal_input.mouse_x, modal_input.mouse_y);
            m.rounded_rect(
                close_btn,
                6.0,
                if close_hover {
                    [0.40, 0.65, 0.95, 1.0]
                } else {
                    [0.30, 0.55, 0.85, 1.0]
                },
            );
            m.text(
                TextBlock::new("Close", close_btn.x + 22.0, close_btn.y + 8.0)
                    .with_size(16.0)
                    .with_color(255, 255, 255),
            );
            if close_hover && modal_input.mouse_clicked {
                ui_persist_clone.modal_open = false;
            }
            layers.pop_layer();
        }

        // Deferred dropdown list (Popup layer above the base content).
        // Picking an option updates the selection and closes the menu.
        if let Some((id, idx)) = ui_persist_clone.dropdowns.draw_open_layer(
            &mut layers,
            dropdown_popup,
            &StyleResolver::new(&theme),
            &input_state_clone,
        ) {
            if id == DROPDOWN_ID {
                ui_persist_clone.dropdown_sel = idx;
            }
        }
        ui_persist_clone.dropdowns.end_frame();

        // Bonus: rotated badge from the original demo, on the base layer
        // (built via UiContext::with_layers so it stays clipped to the
        // base list).
        {
            let mut ui = UiContext::with_layers(&mut layers);
            ui.push();
            ui.translate(660.0, 320.0);
            ui.rotate(330.0_f32.to_radians());
            ui.center();
            ui.rounded_rect(160.0, 40.0, 6.0, [0.95, 0.45, 0.30, 1.0]);
            ui.pop();
        }

        let mut drag_capture_clone = drag_capture_clone.borrow_mut();
        // ---------- Draggable box demo (DragHandle + DragTracker) ----------
        // `DragHandle::bare()` arbitrates the grab through a shared
        // `DragCapture` (so it can't fight other draggables) and returns
        // the per-frame delta sourced from the `DragTracker`. We use the
        // bare handle as a pure hit-zone and keep the box's own visuals.
        {
            let handle_rect = state.drag_box;
            let out = {
                let mut ctx = DrawContext::new(
                    layers.base_mut(),
                    &mut focus_clone,
                    &theme,
                    &base_input,
                    config_clone1.width as f32,
                    config_clone1.height as f32,
                )
                .with_cursor(&mut frame_cursor);
                DragHandle::bare().draw(
                    DRAG_BOX_ID,
                    &mut drag_capture_clone,
                    handle_rect,
                    &mut ctx,
                )
            };
            state.drag_box.x += out.delta[0];
            state.drag_box.y += out.delta[1];

            let color = if out.dragging {
                [0.95, 0.75, 0.30, 1.0]
            } else {
                [0.30, 0.70, 0.55, 1.0]
            };
            let list = layers.base_mut();
            list.rounded_rect(state.drag_box, 8.0, color);
            list.text(
                TextBlock::new("drag me", state.drag_box.x + 18.0, state.drag_box.y + 18.0)
                    .with_size(16.0)
                    .with_color(20, 24, 28),
            );
        }

        // ---------- Double-click / hold demo (ClickTracker) ----------
        {
            let demo_rect = Rect::new(340.0, 430.0, 150.0, 56.0);
            let on_btn = !base_input.mouse_consumed
                && demo_rect.contains(base_input.mouse_x, base_input.mouse_y);
            let color = if on_btn && base_input.mouse_held {
                [0.85, 0.30, 0.30, 1.0] // red = held
            } else if on_btn && base_input.mouse_double_clicked {
                [0.40, 0.80, 0.40, 1.0] // green = double-click
            } else if on_btn {
                [0.40, 0.55, 0.80, 1.0] // hover
            } else {
                [0.26, 0.38, 0.60, 1.0] // idle
            };
            let label = if on_btn && base_input.mouse_held {
                "HELD"
            } else if on_btn && base_input.mouse_double_clicked {
                "DOUBLE!"
            } else {
                "dbl/hold"
            };
            let list = layers.base_mut();
            list.rounded_rect(demo_rect, 8.0, color);
            list.text(
                TextBlock::new(label, demo_rect.x + 18.0, demo_rect.y + 18.0)
                    .with_size(16.0)
                    .with_color(230, 235, 245),
            );
        }

        input_state_clone.end_frame();
        // Apply the cursor requested by whichever widget the pointer is
        // over this frame (Default if none asked).
        //window.set_cursor(to_winit_cursor(frame_cursor.resolve()));


        let mut encoder =
            device_clone1
                .create_command_encoder(&wgpu::CommandEncoderDescriptor {
                    label: Some("hello_ui encoder"),
                });

        // Clear the frame manually (UiRenderer always loads).
        {
            let _clear = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
                label: Some("clear"),
                color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                    view: &view,
                    resolve_target: None,
                    ops: wgpu::Operations {
                        load: wgpu::LoadOp::Clear(wgpu::Color {
                            r: 0.05,
                            g: 0.06,
                            b: 0.08,
                            a: 1.0,
                        }),
                        store: wgpu::StoreOp::Store,
                    },
                    depth_slice: None,
                })],
                depth_stencil_attachment: None,
                timestamp_writes: None,
                occlusion_query_set: None,
                multiview_mask: None,
            });
        }

        ui_renderer.render_layers(
            &device_clone1,
            &state.queue,
            &mut encoder,
            &view,
            (config_clone1.width, config_clone1.height),
            1.0,
            &layers,
        );



        state.queue.submit(Some(encoder.finish()));
        state.queue.present(frame);

        request_animation_frame(f.borrow().as_ref().unwrap());
    }) as Box<dyn FnMut()>));

    request_animation_frame(g.borrow().as_ref().unwrap());
}

fn create_depth_texture(
    device: &wgpu::Device,
    config: &wgpu::SurfaceConfiguration,
) -> wgpu::Texture {
    device.create_texture(&wgpu::TextureDescriptor {
        label: Some("Depth Texture"),
        size: wgpu::Extent3d {
            width: config.width,
            height: config.height,
            depth_or_array_layers: 1,
        },
        mip_level_count: 1,
        sample_count: 1,
        dimension: wgpu::TextureDimension::D2,
        format: wgpu::TextureFormat::Depth24Plus,
        usage: wgpu::TextureUsages::RENDER_ATTACHMENT | wgpu::TextureUsages::TEXTURE_BINDING,
        view_formats: &[],
    })
}

fn request_animation_frame(f: &Closure<dyn FnMut()>) {

    web_sys::window()
        .unwrap()
        .request_animation_frame(f.as_ref().unchecked_ref())
        .expect("should register `requestAnimationFrame` OK");
}

async fn fetch_bytes(url: &str) -> Vec<u8> {
    use js_sys::futures::JsFuture;

    let window = web_sys::window().unwrap();
    let resp_value = JsFuture::from(window.fetch_with_str(url)).await.unwrap();
    let resp: web_sys::Response = resp_value.dyn_into().unwrap();
    let array_buffer = JsFuture::from(resp.array_buffer().unwrap()).await.unwrap();
    let uint8_array = js_sys::Uint8Array::new(&array_buffer);
    uint8_array.to_vec()
}
