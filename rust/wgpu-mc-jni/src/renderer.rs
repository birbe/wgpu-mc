use std::collections::HashMap;
use std::io::Cursor;
use std::mem::size_of;
use std::time::Duration;
use std::{slice, thread};
use std::{sync::Arc, time::Instant};

use arc_swap::access::Access;
use arc_swap::{ArcSwap, ArcSwapAny};
use byteorder::LittleEndian;
use cgmath::{perspective, Deg, Matrix4, SquareMatrix};
use futures::executor::block_on;
use glam::ivec2;
use jni::objects::{AutoElements, JClass, JFloatArray, ReleaseMode};
use jni::sys::{jfloat, jint, jlong};
use jni::{
    objects::{JString, JValue},
    JNIEnv,
};
use jni_fn::jni_fn;
use once_cell::sync::{Lazy, OnceCell};
use parking_lot::{Mutex, RwLock};
use wgpu_mc::mc::resource::ResourcePath;
use wgpu_mc::mc::{RenderEffectsData, SkyState};
use winit::dpi::PhysicalSize;
use winit::event::{DeviceEvent, ElementState, Event, KeyEvent, WindowEvent};
use winit::event_loop::EventLoopBuilder;
use winit::keyboard::{KeyCode, ModifiersState, PhysicalKey};
use winit::platform::scancode::PhysicalKeyExtScancode;

use wgpu_mc::mc::block::{BlockMeshVertex, BlockstateKey};
use wgpu_mc::mc::chunk::{LightLevel, RenderLayer};
use wgpu_mc::mc::entity::{BundledEntityInstances, InstanceVertex, UploadedEntityInstances};
use wgpu_mc::render::graph::{Geometry, RenderGraph, ResourceBacking};
use wgpu_mc::render::pipeline::Vertex;
use wgpu_mc::render::shaderpack::{Mat4, Mat4ValueOrMult, ShaderPackConfig};
use wgpu_mc::texture::{BindableTexture, TextureAndView};
use wgpu_mc::util::BindableBuffer;
use wgpu_mc::wgpu::util::{BufferInitDescriptor, DeviceExt};
use wgpu_mc::wgpu::{BufferAddress, BufferBindingType, BufferUsages, PresentMode, TextureFormat};
use wgpu_mc::{Frustum, wgpu, WindowSize};
use wgpu_mc::{WgpuState, WmRenderer};

use crate::gl::{ElectrumGeometry, ElectrumVertex, GlTexture, GL_ALLOC};
use crate::lighting::LIGHTMAP_GLID;
use crate::{
    MinecraftRenderState, MinecraftResourceManagerAdapter, RenderMessage, WinitWindowWrapper,
    CHANNELS, CLEAR_COLOR, MC_STATE, RENDERER, SCENE, THREAD_POOL, WINDOW,
};

pub static MATRICES: Lazy<Mutex<Matrices>> = Lazy::new(|| {
    Mutex::new(Matrices {
        projection: [[0.0; 4]; 4],
        view: [[0.0; 4]; 4],
        terrain_transformation: [[0.0; 4]; 4],
    })
});

static SHOULD_STOP: OnceCell<()> = OnceCell::new();

pub struct Matrices {
    pub projection: [[f32; 4]; 4],
    pub view: [[f32; 4]; 4],
    pub terrain_transformation: [[f32; 4]; 4],
}

#[jni_fn("dev.birb.wgpu.rust.WgpuNative")]
pub fn setMatrix(mut env: JNIEnv, _class: JClass, id: jint, float_array: JFloatArray) {
    let elements: AutoElements<jfloat> =
        unsafe { env.get_array_elements(&float_array, ReleaseMode::NoCopyBack) }.unwrap();

    let slice = unsafe { slice::from_raw_parts(elements.as_ptr(), elements.len()) };

    let mut cursor = Cursor::new(bytemuck::cast_slice::<f32, u8>(slice));
    let mut converted = Vec::with_capacity(slice.len());

    for _ in 0..slice.len() {
        use byteorder::ReadBytesExt;
        converted.push(cursor.read_f32::<LittleEndian>().unwrap());
    }

    let slice_4x4: [[f32; 4]; 4] = *bytemuck::from_bytes(bytemuck::cast_slice(&converted));

    match id {
        0 => {
            MATRICES.lock().projection = slice_4x4;
        }
        1 => {
            // MATRICES.lock(). = slice_4x4;
        }
        2 => {
            MATRICES.lock().view = slice_4x4;
        }
        3 => {
            MATRICES.lock().terrain_transformation = slice_4x4;
        }
        _ => {}
    }
}

fn create_matrix_buffer(wm: &WmRenderer) -> Arc<wgpu::Buffer> {
    Arc::new(
        wm.wgpu_state
            .device
            .create_buffer_init(&BufferInitDescriptor {
                label: None,
                contents: &[0; 64],
                usage: wgpu::BufferUsages::COPY_DST | wgpu::BufferUsages::UNIFORM,
            }),
    )
}

#[jni_fn("dev.birb.wgpu.rust.WgpuNative")]
pub fn scheduleStop(_env: JNIEnv, _class: JClass) {
    let _ = SHOULD_STOP.set(());
}

pub fn start_rendering(mut env: JNIEnv, title: JString) {
    let title: String = env.get_string(&title).unwrap().into();

    // Hacky fix for starting the game on linux, needs more investigation (thanks, accusitive)
    // https://docs.rs/winit/latest/winit/event_loop/struct.EventLoopBuilder.html#method.build
    let mut event_loop = EventLoopBuilder::new();

    #[cfg(target_os = "linux")]
    {
        // double hacky fix B)
        if std::env::var("WAYLAND_DISPLAY").is_ok() {
            use winit::platform::wayland::EventLoopBuilderExtWayland;
            event_loop.with_any_thread(true);
        } else {
            use winit::platform::x11::EventLoopBuilderExtX11;
            event_loop.with_any_thread(true);
        }
    }

    let event_loop = event_loop.build().unwrap();

    let window = Arc::new(
        winit::window::WindowBuilder::new()
            .with_title(title)
            .with_inner_size(winit::dpi::Size::Physical(PhysicalSize {
                width: 1280,
                height: 720,
            }))
            .build(&event_loop)
            .unwrap(),
    );

    WINDOW.set(window.clone()).unwrap();

    let resource_provider = Arc::new(MinecraftResourceManagerAdapter {
        jvm: env.get_java_vm().unwrap(),
    });

    let instance = wgpu::Instance::new(wgpu::InstanceDescriptor {
        backends: wgpu::Backends::VULKAN,
        ..Default::default()
    });

    let surface = instance.create_surface(window.clone()).unwrap();
    let adapter = block_on(instance.request_adapter(&wgpu::RequestAdapterOptions {
        power_preference: wgpu::PowerPreference::HighPerformance,
        force_fallback_adapter: false,
        compatible_surface: Some(&surface),
    }))
    .unwrap();

    let required_limits = wgpu::Limits {
        max_push_constant_size: 128,
        max_bind_groups: 8,
        max_storage_buffers_per_shader_stage: 100000,
        ..Default::default()
    };

    let (device, queue) = block_on(adapter.request_device(
        &wgpu::DeviceDescriptor {
            label: None,
            required_features: wgpu::Features::default()
                | wgpu::Features::DEPTH_CLIP_CONTROL
                | wgpu::Features::PUSH_CONSTANTS
                | wgpu::Features::BUFFER_BINDING_ARRAY
                | wgpu::Features::STORAGE_RESOURCE_BINDING_ARRAY
                | wgpu::Features::SAMPLED_TEXTURE_AND_STORAGE_BUFFER_ARRAY_NON_UNIFORM_INDEXING
                | wgpu::Features::PARTIALLY_BOUND_BINDING_ARRAY,
            required_limits,
        },
        None, // Trace path
    ))
    .unwrap();

    const VSYNC: bool = false;

    let surface_caps = surface.get_capabilities(&adapter);
    let surface_config = wgpu::SurfaceConfiguration {
        usage: wgpu::TextureUsages::RENDER_ATTACHMENT,
        format: wgpu::TextureFormat::Bgra8Unorm,
        width: window.inner_size().width,
        height: window.inner_size().height,
        present_mode: if VSYNC {
            PresentMode::AutoVsync
        } else if surface_caps.present_modes.contains(&PresentMode::Immediate) {
            PresentMode::Immediate
        } else {
            surface_caps.present_modes[0]
        },

        desired_maximum_frame_latency: 2,
        alpha_mode: surface_caps.alpha_modes[0],
        view_formats: vec![],
    };

    surface.configure(&device, &surface_config);

    let wgpu_state = WgpuState {
        surface: RwLock::new((Some(surface), surface_config)),
        adapter,
        device,
        queue,
        size: Some(ArcSwap::new(Arc::new(WindowSize {
            width: window.inner_size().width,
            height: window.inner_size().height,
        }))),
    };

    let wm = WmRenderer::new(wgpu_state, resource_provider);

    let _ = RENDERER.set(wm.clone());

    wm.init();

    env.set_static_field(
        "dev/birb/wgpu/render/Wgpu",
        ("dev/birb/wgpu/render/Wgpu", "initialized", "Z"),
        JValue::Bool(true.into()),
    )
    .unwrap();

    let mut current_modifiers = ModifiersState::empty();

    log::trace!("Starting event loop");

    // {
    //     let tex_id = LIGHTMAP_GLID.lock().unwrap();
    //     let textures_read = GL_ALLOC.read();
    //     let lightmap = textures_read.get(&*tex_id).unwrap();
    //     let bindable = lightmap.bindable_texture.as_ref().unwrap();
    //     let asaa = ArcSwap::new(bindable.clone());
    // }

    let shader_pack: ShaderPackConfig =
        serde_yaml::from_str(include_str!("../graph.yaml")).unwrap();

    let mut render_resources = HashMap::new();

    let mat4_projection = create_matrix_buffer(&wm);
    let mat4_view = create_matrix_buffer(&wm);
    let mat4_model = create_matrix_buffer(&wm);

    render_resources.insert(
        "@mat4_view".into(),
        ResourceBacking::Buffer(mat4_view.clone(), BufferBindingType::Uniform),
    );

    render_resources.insert(
        "@mat4_perspective".into(),
        ResourceBacking::Buffer(mat4_projection.clone(), BufferBindingType::Uniform),
    );

    render_resources.insert(
        "@mat4_model".into(),
        ResourceBacking::Buffer(mat4_model.clone(), BufferBindingType::Uniform),
    );

    let mut custom_bind_groups = HashMap::new();
    custom_bind_groups.insert(
        "@texture_electrum_gui".into(),
        wm.bind_group_layouts.get("texture").unwrap(),
    );
    custom_bind_groups.insert(
        "@mat4_electrum_gui".into(),
        wm.bind_group_layouts.get("matrix").unwrap(),
    );

    let mut custom_geometry = HashMap::new();
    custom_geometry.insert(
        "@geo_electrum_gui".into(),
        vec![wgpu::VertexBufferLayout {
            array_stride: size_of::<ElectrumVertex>() as BufferAddress,
            step_mode: Default::default(),
            attributes: &ElectrumVertex::VAO,
        }],
    );

    let render_graph = RenderGraph::new(
        &wm,
        shader_pack,
        render_resources,
        Some(custom_bind_groups),
        Some(custom_geometry),
    );

    let mut geometry = HashMap::new();
    geometry.insert(
        "@geo_electrum_gui".into(),
        Box::new(ElectrumGeometry {
            pool: Arc::new(
                wm.wgpu_state
                    .device
                    .create_buffer_init(&BufferInitDescriptor {
                        label: None,
                        contents: &vec![0; 1_000_000],
                        usage: wgpu::BufferUsages::COPY_DST
                            | wgpu::BufferUsages::VERTEX
                            | wgpu::BufferUsages::INDEX,
                    }),
            ),
            last_bytes: None,
        }) as Box<dyn Geometry>,
    );

    let wm_clone = wm.clone();
    thread::spawn(move || {
        loop {
            wm_clone.submit_chunk_updates();
            thread::sleep(Duration::from_millis(10));
        }
    });

    event_loop
        .run(move |event, target| {
            if SHOULD_STOP.get().is_some() {
                target.exit();
            }

            match event {
                Event::AboutToWait => {
                    window.request_redraw();
                }
                Event::WindowEvent {
                    ref event,
                    window_id,
                } if window_id == window.id() => {
                    match event {
                        WindowEvent::CloseRequested => target.exit(),
                        WindowEvent::Resized(physical_size) => {
                            // Update the wgpu_state size for the render loop.
                            let state_size = wm.wgpu_state.size.as_ref().unwrap();
                            state_size.swap(Arc::new(WindowSize {
                                width: physical_size.width,
                                height: physical_size.height,
                            }));

                            CHANNELS
                                .0
                                .send(RenderMessage::Resized(
                                    physical_size.width,
                                    physical_size.height,
                                ))
                                .unwrap();
                        }
                        WindowEvent::MouseInput {
                            device_id: _,
                            state,
                            button,
                            ..
                        } => {
                            CHANNELS
                                .0
                                .send(RenderMessage::MouseState(*state, *button))
                                .unwrap();
                        }
                        WindowEvent::CursorMoved { position, .. } => {
                            CHANNELS
                                .0
                                .send(RenderMessage::CursorMove(position.x, position.y))
                                .unwrap();
                        }
                        WindowEvent::RedrawRequested => {
                            println!("draw");

                            {
                                let matrices = MATRICES.lock();

                                wm.wgpu_state.queue.write_buffer(
                                    &mat4_projection,
                                    0,
                                    bytemuck::cast_slice(&matrices.projection),
                                );
                                wm.wgpu_state.queue.write_buffer(
                                    &mat4_view,
                                    0,
                                    bytemuck::cast_slice(&matrices.view),
                                );
                                wm.wgpu_state.queue.write_buffer(
                                    &mat4_model,
                                    0,
                                    bytemuck::cast_slice(&matrices.terrain_transformation),
                                );
                            }

                            let mut surface_guard = wm.wgpu_state.surface.write();
                            let (surface, surface_config) = &mut *surface_guard;
                            let surface = surface.as_ref().unwrap();

                            let texture = surface.get_current_texture().unwrap_or_else(|_| {
                                //The surface is outdated, so we force an update. This can't be done on the window resize event for synchronization reasons.
                                let size = wm.wgpu_state.size.as_ref().unwrap().load();

                                surface_config.width = size.width;
                                surface_config.height = size.height;

                                surface.configure(&wm.wgpu_state.device, &surface_config);
                                surface.get_current_texture().unwrap()
                            });

                            let view = texture.texture.create_view(&wgpu::TextureViewDescriptor {
                                label: None,
                                format: Some(TextureFormat::Bgra8Unorm),
                                dimension: Some(wgpu::TextureViewDimension::D2),
                                aspect: Default::default(),
                                base_mip_level: 0,
                                mip_level_count: None,
                                base_array_layer: 0,
                                array_layer_count: None,
                            });

                            {
                                let mut encoder = wm.wgpu_state.device.create_command_encoder(
                                    &wgpu::CommandEncoderDescriptor { label: None },
                                );

                                render_graph.render(
                                    &wm,
                                    &mut encoder,
                                    &SCENE,
                                    &view,
                                    [0; 3],
                                    &mut geometry,
                                    &Frustum::from_modelview_projection([[0.0; 4]; 4])
                                );

                                wm.wgpu_state.queue.submit([encoder.finish()]);
                            }

                            texture.present();
                        }
                        WindowEvent::KeyboardInput {
                            event:
                                KeyEvent {
                                    physical_key: PhysicalKey::Code(key),
                                    text,
                                    state,
                                    ..
                                },
                            ..
                        } => {
                            if let Some(scancode) = key.to_scancode() {
                                CHANNELS
                                    .0
                                    .send(RenderMessage::KeyState(
                                        keycode_to_glfw(*key),
                                        scancode,
                                        match state {
                                            ElementState::Pressed => 1,  // GLFW_PRESS
                                            ElementState::Released => 0, // GLFW_RELEASE
                                        },
                                        modifiers_to_glfw(current_modifiers),
                                    ))
                                    .unwrap();

                                if let Some(text) = text {
                                    for c in text.chars() {
                                        CHANNELS
                                            .0
                                            .send(RenderMessage::CharTyped(
                                                c,
                                                modifiers_to_glfw(current_modifiers),
                                            ))
                                            .unwrap();
                                    }
                                }
                            }
                        }
                        WindowEvent::ModifiersChanged(new_modifiers) => {
                            current_modifiers = new_modifiers.state();
                        }
                        WindowEvent::Focused(focused) => {
                            CHANNELS.0.send(RenderMessage::Focused(*focused)).unwrap();
                        }
                        _ => {}
                    }
                }
                Event::DeviceEvent {
                    device_id: _,
                    event: DeviceEvent::MouseMotion { delta },
                } => {
                    CHANNELS
                        .0
                        .send(RenderMessage::MouseMove(delta.0, delta.1))
                        .unwrap();
                }
                _ => {}
            }
        })
        .unwrap();
}

fn keycode_to_glfw(code: KeyCode) -> u32 {
    match code {
        KeyCode::Space => 32,
        KeyCode::Quote => 39,
        KeyCode::Comma => 44,
        KeyCode::Minus => 45,
        KeyCode::Period => 46,
        KeyCode::Slash => 47,
        KeyCode::Digit0 => 48,
        KeyCode::Digit1 => 49,
        KeyCode::Digit2 => 50,
        KeyCode::Digit3 => 51,
        KeyCode::Digit4 => 52,
        KeyCode::Digit5 => 53,
        KeyCode::Digit6 => 54,
        KeyCode::Digit7 => 55,
        KeyCode::Digit8 => 56,
        KeyCode::Digit9 => 57,
        KeyCode::Semicolon => 59,
        KeyCode::Equal => 61,
        KeyCode::KeyA => 65,
        KeyCode::KeyB => 66,
        KeyCode::KeyC => 67,
        KeyCode::KeyD => 68,
        KeyCode::KeyE => 69,
        KeyCode::KeyF => 70,
        KeyCode::KeyG => 71,
        KeyCode::KeyH => 72,
        KeyCode::KeyI => 73,
        KeyCode::KeyJ => 74,
        KeyCode::KeyK => 75,
        KeyCode::KeyL => 76,
        KeyCode::KeyM => 77,
        KeyCode::KeyN => 78,
        KeyCode::KeyO => 79,
        KeyCode::KeyP => 80,
        KeyCode::KeyQ => 81,
        KeyCode::KeyR => 82,
        KeyCode::KeyS => 83,
        KeyCode::KeyT => 84,
        KeyCode::KeyU => 85,
        KeyCode::KeyV => 86,
        KeyCode::KeyW => 87,
        KeyCode::KeyX => 88,
        KeyCode::KeyY => 89,
        KeyCode::KeyZ => 90,
        KeyCode::BracketLeft => 91,
        KeyCode::Backslash => 92,
        KeyCode::BracketRight => 93,
        KeyCode::Backquote => 96,
        // what the fuck are WORLD_1 (161) and WORLD_2 (162)
        KeyCode::Escape => 256,
        KeyCode::Enter => 257,
        KeyCode::Tab => 258,
        KeyCode::Backspace => 259,
        KeyCode::Insert => 260,
        KeyCode::Delete => 261,
        KeyCode::ArrowRight => 262,
        KeyCode::ArrowLeft => 263,
        KeyCode::ArrowDown => 264,
        KeyCode::ArrowUp => 265,
        KeyCode::PageUp => 266,
        KeyCode::PageDown => 267,
        KeyCode::Home => 268,
        KeyCode::End => 269,
        KeyCode::CapsLock => 280,
        KeyCode::ScrollLock => 281,
        KeyCode::NumLock => 282,
        KeyCode::PrintScreen => 283,
        KeyCode::Pause => 284,
        KeyCode::F1 => 290,
        KeyCode::F2 => 291,
        KeyCode::F3 => 292,
        KeyCode::F4 => 293,
        KeyCode::F5 => 294,
        KeyCode::F6 => 295,
        KeyCode::F7 => 296,
        KeyCode::F8 => 297,
        KeyCode::F9 => 298,
        KeyCode::F10 => 299,
        KeyCode::F11 => 300,
        KeyCode::F12 => 301,
        KeyCode::F13 => 302,
        KeyCode::F14 => 303,
        KeyCode::F15 => 304,
        KeyCode::F16 => 305,
        KeyCode::F17 => 306,
        KeyCode::F18 => 307,
        KeyCode::F19 => 308,
        KeyCode::F20 => 309,
        KeyCode::F21 => 310,
        KeyCode::F22 => 311,
        KeyCode::F23 => 312,
        KeyCode::F24 => 313,
        KeyCode::F25 => 314,
        KeyCode::Numpad0 => 320,
        KeyCode::Numpad1 => 321,
        KeyCode::Numpad2 => 322,
        KeyCode::Numpad3 => 323,
        KeyCode::Numpad4 => 324,
        KeyCode::Numpad5 => 325,
        KeyCode::Numpad6 => 326,
        KeyCode::Numpad7 => 327,
        KeyCode::Numpad8 => 328,
        KeyCode::Numpad9 => 329,
        KeyCode::NumpadDecimal => 330,
        KeyCode::NumpadDivide => 331,
        KeyCode::NumpadMultiply => 332,
        KeyCode::NumpadSubtract => 333,
        KeyCode::NumpadAdd => 334,
        KeyCode::NumpadEnter => 335,
        KeyCode::NumpadEqual => 336,
        KeyCode::ShiftLeft => 340,
        KeyCode::ControlLeft => 341,
        KeyCode::AltLeft => 342,
        KeyCode::SuperLeft => 343,
        KeyCode::ShiftRight => 344,
        KeyCode::ControlRight => 345,
        KeyCode::AltRight => 346,
        KeyCode::SuperRight => 347,
        KeyCode::ContextMenu => 348,
        _ => 0,
    }
}

fn modifiers_to_glfw(state: ModifiersState) -> u32 {
    if state.is_empty() {
        return 0;
    }

    let mut mods = 0;

    if state.shift_key() {
        mods |= 1;
    }
    if state.control_key() {
        mods |= 2;
    }
    if state.alt_key() {
        mods |= 4;
    }
    if state.super_key() {
        mods |= 8;
    }

    mods
}

#[derive(Copy, Clone, Hash, Eq, PartialEq)]
pub enum MCTextureId {
    BlockAtlas,
    Lightmap,
}

pub static ENTITY_INSTANCES: Lazy<Mutex<HashMap<String, BundledEntityInstances>>> =
    Lazy::new(|| Mutex::new(HashMap::new()));

pub static MC_TEXTURES: Lazy<Mutex<HashMap<MCTextureId, Arc<BindableTexture>>>> =
    Lazy::new(|| Mutex::new(HashMap::new()));

#[jni_fn("dev.birb.wgpu.rust.WgpuNative")]
pub fn clearEntities(_env: JNIEnv, _class: JClass) {
    ENTITY_INSTANCES.lock().clear();
}

#[jni_fn("dev.birb.wgpu.rust.WgpuNative")]
pub fn identifyGlTexture(_env: JNIEnv, _class: JClass, texture: jint, gl_id: jint) {
    let alloc_read = GL_ALLOC.read();
    let gl_texture = alloc_read.get(&(gl_id as u32)).unwrap();

    let mut mc_textures = MC_TEXTURES.lock();
    mc_textures.insert(
        match texture {
            0 => MCTextureId::BlockAtlas,
            1 => MCTextureId::Lightmap,
            _ => unreachable!(),
        },
        gl_texture.bindable_texture.as_ref().unwrap().clone(),
    );
}

#[jni_fn("dev.birb.wgpu.rust.WgpuNative")]
pub fn setEntityInstanceBuffer(
    mut env: JNIEnv,
    _class: JClass,
    entity_name: JString,
    mat4_ptr: jlong,
    mat4_len: jint,
    overlay_ptr: jlong,
    overlay_len: jint,
    instance_count: jint,
    texture_id: jint,
) -> jlong {
    assert!(instance_count >= 0);
    let now = Instant::now();
    let instance_count = instance_count as u32;

    let wm = RENDERER.get().unwrap();

    //TODO this is slow, let's use an integer id somewhere
    let entity_name: String = env.get_string(&entity_name).unwrap().into();

    if instance_count == 0 {
        ENTITY_INSTANCES.lock().remove(&entity_name);
        return Instant::now().duration_since(now).as_nanos() as jlong;
    }

    let mat4s = unsafe { slice::from_raw_parts(mat4_ptr as usize as *mut f32, mat4_len as usize) };

    let overlays =
        unsafe { slice::from_raw_parts(overlay_ptr as usize as *mut i32, overlay_len as usize) };

    let transforms: Vec<f32> = Vec::from(mat4s);
    let overlays: Vec<i32> = Vec::from(overlays);
    let verts: Vec<InstanceVertex> = (0..instance_count)
        .map(|index| InstanceVertex {
            entity_index: index,
            uv_offset: [0, 0],
        })
        .collect();

    let mut instances = ENTITY_INSTANCES.lock();
    let bundled_entity_instances =
        if let Some(bundled_entity_instances) = instances.get_mut(&entity_name) {
            bundled_entity_instances.count = instance_count;
            bundled_entity_instances
        } else {
            let texture = {
                let gl_alloc = GL_ALLOC.read();

                match gl_alloc.get(&(texture_id as u32)) {
                    None => return 0,
                    Some(GlTexture {
                        bindable_texture: None,
                        ..
                    }) => return 0,
                    _ => {}
                }

                gl_alloc
                    .get(&(texture_id as u32))
                    .unwrap()
                    .bindable_texture
                    .as_ref()
                    .unwrap()
                    .clone()
            };
            let models = wm.mc.entity_models.read();
            let entity = models.get(&entity_name).unwrap();
            instances.insert(
                entity_name.clone(),
                BundledEntityInstances::new(wm, entity.clone(), instance_count, texture),
            );
            instances.get(&entity_name).unwrap()
        };

    wm.wgpu_state.queue.write_buffer(
        bundled_entity_instances.uploaded.instance_vbo.as_ref(),
        0,
        bytemuck::cast_slice(&verts),
    );
    wm.wgpu_state.queue.write_buffer(
        &bundled_entity_instances.uploaded.transform_ssbo.buffer,
        0,
        bytemuck::cast_slice(&transforms),
    );
    wm.wgpu_state.queue.write_buffer(
        &bundled_entity_instances.uploaded.overlay_ssbo.buffer,
        0,
        bytemuck::cast_slice(&overlays),
    );
    Instant::now().duration_since(now).as_nanos() as jlong
}

#[jni_fn("dev.birb.wgpu.rust.WgpuNative")]
pub fn bindSkyData(
    _env: JNIEnv,
    _class: JClass,
    r: jfloat,
    g: jfloat,
    b: jfloat,
    angle: jfloat,
    brightness: jfloat,
    star_shimmer: jfloat,
    moon_phase: jint,
) {
    // let mut sky_data = (**RENDERER.get().unwrap().mc.sky_data.load()).clone();
    // sky_data.color_r = r;
    // sky_data.color_g = g;
    // sky_data.color_b = b;
    // sky_data.angle = angle;
    // sky_data.brightness = brightness;
    // sky_data.star_shimmer = star_shimmer;
    // sky_data.moon_phase = moon_phase;
    //
    // RENDERER.get().unwrap().mc.sky_data.swap(Arc::new(sky_data));
}

#[jni_fn("dev.birb.wgpu.rust.WgpuNative")]
pub fn bindRenderEffectsData(
    env: JNIEnv,
    _class: JClass,
    fog_start: jfloat,
    fog_end: jfloat,
    fog_shape: jint,
    fog_color: JFloatArray,
    color_modulator: JFloatArray,
    dimension_fog_color: JFloatArray,
) {
    let mut render_effects_data = RenderEffectsData {
        fog_start,
        fog_end,
        fog_shape: fog_shape as f32,
        ..Default::default()
    };

    let mut fog_color_vec = vec![0f32; env.get_array_length(&fog_color).unwrap() as usize];
    env.get_float_array_region(&fog_color, 0, &mut fog_color_vec[..])
        .unwrap();

    let mut color_modulator_vec =
        vec![0f32; env.get_array_length(&color_modulator).unwrap() as usize];
    env.get_float_array_region(&color_modulator, 0, &mut color_modulator_vec[..])
        .unwrap();

    let mut dimension_fog_color_vec =
        vec![0f32; env.get_array_length(&dimension_fog_color).unwrap() as usize];
    env.get_float_array_region(&dimension_fog_color, 0, &mut dimension_fog_color_vec[..])
        .unwrap();

    // render_effects_data.fog_color = fog_color_vec;
    // render_effects_data.color_modulator = color_modulator_vec;
    // render_effects_data.dimension_fog_color = dimension_fog_color_vec;
    //
    // RENDERER
    //     .get()
    //     .unwrap()
    //     .mc
    //     .render_effects
    //     .swap(Arc::new(render_effects_data));
}
