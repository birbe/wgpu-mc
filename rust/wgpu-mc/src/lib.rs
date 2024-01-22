/*!
# wgpu-mc
wgpu-mc is a pure-Rust crate which is designed to be usable by anyone who needs to render
Minecraft-style scenes using Rust. The main user of this crate at this time is the Minecraft mod
Electrum which replaces Minecraft's official renderer with wgpu-mc.
However, anyone is able to use this crate, and the API is designed to be completely independent
of any single project, allowing anyone to use it. It is mostly batteries-included, except for a
few things.

# Considerations

This crate is unstable and subject to change. The basic structure for features such
as terrain rendering and entity rendering are already in-place but could very well change significantly
in the future.

# Setup

wgpu-mc, as you could have probably guessed, uses the [wgpu](https://github.com/gfx-rs/wgpu) crate
for communicating with the GPU. Assuming you aren't running wgpu-mc headless (if you are, I assume
you already know what you're doing), wgpu-mc can handle surface and device setup for you, as long
as you pass in a valid window handle. See [init_wgpu]

# Rendering

wgpu-mc makes use of a trait called `WmPipeline` to describe any struct which is used for
rendering. There are multiple built in pipelines, but they aren't required to use while rendering.

## Terrain Rendering

The first step to begin terrain rendering is to implement [BlockStateProvider](cr).
This is a trait that provides a block state key for a given coordinate.

## Entity Rendering

To render entities, you need an entity model. wgpu-mc makes no assumptions about how entity models are defined,
so it's up to you to provide them to wgpu-mc.

See the [render::entity] module for an example of rendering an example entity.
 */

use std::borrow::Borrow;
use std::collections::HashMap;
use std::num::NonZeroU64;
use std::sync::Arc;

use arc_swap::ArcSwap;
pub use minecraft_assets;
use parking_lot::{Mutex, RwLock};
pub use wgpu;
use wgpu::{
    BindGroupDescriptor, BindGroupEntry, Buffer, BufferDescriptor, Extent3d, PresentMode
};
use wgpu::util::StagingBelt;

use crate::mc::entity::BundledEntityInstances;
use crate::mc::MinecraftState;
use crate::mc::resource::ResourceProvider;
use crate::render::atlas::Atlas;
use crate::render::graph::ShaderGraph;
use crate::render::pipeline::{BLOCK_ATLAS, ENTITY_ATLAS, WmPipelines};
use crate::texture::{BindableTexture, TextureHandle, TextureSamplerView};

pub mod mc;
pub mod render;
pub mod texture;
pub mod util;

pub const CHUNK_STAGING_BELT_SIZE: u64 = 64_000_000;

/// Provides access to most of the wgpu structs relating directly to communicating/getting
/// information about the gpu.
pub struct WgpuState {
    pub surface: RwLock<(Option<wgpu::Surface<'static>>, wgpu::SurfaceConfiguration)>,
    pub adapter: wgpu::Adapter,
    pub device: wgpu::Device,
    pub queue: wgpu::Queue,
    pub size: Option<ArcSwap<WindowSize>>,
}

/// The main wgpu-mc renderer struct. This mostly just contains wgpu state.
/// Resources pertaining to Minecraft go in `MinecraftState`
#[derive(Clone)]
pub struct WmRenderer {
    pub wgpu_state: Arc<WgpuState>,
    pub texture_handles: Arc<RwLock<HashMap<String, TextureHandle>>>,
    pub pipelines: Arc<ArcSwap<WmPipelines>>,
    pub mc: Arc<MinecraftState>,
    pub chunk_update_queue: Arc<Mutex<Vec<(Arc<Buffer>, Vec<u8>)>>>,
    pub chunk_staging_belt: Arc<Mutex<StagingBelt>>,
    #[cfg(feature = "tracing")]
    pub puffin_http: Arc<puffin_http::Server>,
}

#[derive(Copy, Clone)]
pub struct WindowSize {
    pub width: u32,
    pub height: u32,
}

pub trait HasWindowSize {
    fn get_window_size(&self) -> WindowSize;
}

impl WmRenderer {

    pub fn new(wgpu_state: WgpuState, resource_provider: Arc<dyn ResourceProvider>) -> WmRenderer {
        #[cfg(feature = "tracing")]
        let puffin_http = {
            let server_addr = format!("127.0.0.1:{}", puffin_http::DEFAULT_PORT);
            let puffin_server = puffin_http::Server::new(&server_addr).unwrap();
            eprintln!("Run this to view profiling data:  puffin_viewer {server_addr}");
            puffin::set_scopes_on(true);
            Arc::new(puffin_server)
        };

        let pipelines = WmPipelines::new(resource_provider.clone());

        let mc = MinecraftState::new(&wgpu_state, resource_provider);

        Self {
            wgpu_state: Arc::new(wgpu_state),

            texture_handles: Arc::new(RwLock::new(HashMap::new())),
            pipelines: Arc::new(ArcSwap::new(Arc::new(pipelines))),
            mc: Arc::new(mc),
            chunk_update_queue: Arc::new(Mutex::new(Vec::new())),
            chunk_staging_belt: Arc::new(Mutex::new(StagingBelt::new(CHUNK_STAGING_BELT_SIZE))),
            #[cfg(feature = "tracing")]
            puffin_http,
        }
    }

    pub fn init(&self) {
        let pipelines = self.pipelines.load();
        pipelines.init(self);

        let atlases = [BLOCK_ATLAS, ENTITY_ATLAS]
            .iter()
            .map(|&name| {
                (
                    name.into(),
                    Arc::new(ArcSwap::new(Arc::new(Atlas::new(
                        &self.wgpu_state,
                        &pipelines,
                        false,
                    )))),
                )
            })
            .collect();

        self.mc.texture_manager.atlases.store(Arc::new(atlases));

        self.create_texture_handle(
            "wm_framebuffer_depth".into(),
            wgpu::TextureFormat::Depth32Float,
            &self.wgpu_state.surface.read().1,
        );
    }

    pub fn create_texture_handle(
        &self,
        name: String,
        format: wgpu::TextureFormat,
        config: &wgpu::SurfaceConfiguration,
    ) -> TextureHandle {
        let tsv = TextureSamplerView::from_rgb_bytes(
            &self.wgpu_state,
            &[],
            Extent3d {
                width: config.width,
                height: config.height,
                depth_or_array_layers: 1,
            },
            None,
            format,
        )
        .unwrap();

        let handle = TextureHandle {
            bindable_texture: Arc::new(ArcSwap::new(Arc::new(BindableTexture::from_tsv(
                &self.wgpu_state,
                &self.pipelines.load(),
                tsv,
                matches!(format, wgpu::TextureFormat::Depth32Float),
            )))),
        };

        let mut handles = self.texture_handles.write();
        handles
            .entry(name)
            .or_insert(handle.clone())
            .bindable_texture
            .store(handle.bindable_texture.load_full());

        handle
    }

    pub fn update_surface_size(
        &self,
        mut surface_config: wgpu::SurfaceConfiguration,
        new_size: WindowSize,
    ) -> Option<wgpu::SurfaceConfiguration> {
        if new_size.width == 0 || new_size.height == 0 {
            return None;
        }

        surface_config.width = new_size.width;
        surface_config.height = new_size.height;

        let handles = { self.texture_handles.read().clone() };

        handles.iter().for_each(|(name, handle)| {
            let texture = handle.bindable_texture.load();

            self.create_texture_handle(name.clone(), texture.tsv.format, &surface_config);
        });

        Some(surface_config)
    }

    pub fn upload_animated_block_buffer(&self, data: Vec<f32>) {
        let d = data.as_slice();

        let buf = self.mc.animated_block_buffer.borrow().load_full();

        if buf.is_none() {
            let animated_block_buffer = self.wgpu_state.device.create_buffer(&BufferDescriptor {
                label: None,
                size: (d.len() * 8) as wgpu::BufferAddress,
                usage: wgpu::BufferUsages::STORAGE | wgpu::BufferUsages::COPY_DST,
                mapped_at_creation: false,
            });
            let animated_block_bind_group =
                self.wgpu_state
                    .device
                    .create_bind_group(&BindGroupDescriptor {
                        label: None,
                        layout: self
                            .pipelines
                            .load()
                            .bind_group_layouts
                            .read()
                            .get("ssbo")
                            .unwrap(),
                        entries: &[BindGroupEntry {
                            binding: 0,
                            resource: wgpu::BindingResource::Buffer(
                                animated_block_buffer.as_entire_buffer_binding(),
                            ),
                        }],
                    });

            self.mc
                .animated_block_buffer
                .store(Arc::new(Some(animated_block_buffer)));
            self.mc
                .animated_block_bind_group
                .store(Arc::new(Some(animated_block_bind_group)));
        }

        self.wgpu_state.queue.write_buffer(
            (**self.mc.animated_block_buffer.load()).as_ref().unwrap(),
            0,
            bytemuck::cast_slice(d),
        );
    }

    pub fn render<'graph: 'resources, 'resources>(
        &self,
        graph: &ShaderGraph,
        output_texture_view: &wgpu::TextureView,
        surface_config: &wgpu::SurfaceConfiguration,
        entity_instances: &'resources HashMap<String, BundledEntityInstances>,
        clear_color: Option<[f32; 3]>
    ) -> Result<(), wgpu::SurfaceError> {
        #[cfg(feature = "tracing")]
        puffin::GlobalProfiler::lock().new_frame();

        graph.render(self, output_texture_view, surface_config, entity_instances, clear_color.unwrap_or([0.0, 0.0, 0.0]));

        Ok(())
    }

    pub fn submit_chunk_updates(&self) {
        puffin::profile_function!();

        let updates = {
            let mut updates = self.chunk_update_queue.lock();
            std::mem::take(&mut *updates)
        };

        if updates.len() == 0 {
            return;
        }

        let mut encoder = self
            .wgpu_state
            .device
            .create_command_encoder(&wgpu::CommandEncoderDescriptor { label: None });

        let mut staging_belt = self.chunk_staging_belt.lock();

        updates.into_iter().for_each(|(queue, data)| {
            let mut view = staging_belt.write_buffer(
                &mut encoder,
                &queue,
                0,
                NonZeroU64::new(data.len() as u64).unwrap(),
                &self.wgpu_state.device,
            );
            view.copy_from_slice(&data);
        });

        staging_belt.finish();
        self.wgpu_state.queue.submit([encoder.finish()]);
        staging_belt.recall();
    }

    pub fn get_backend_description(&self) -> String {
        format!(
            "wgpu 0.18 ({:?})",
            self.wgpu_state.adapter.get_info().backend
        )
    }
}
